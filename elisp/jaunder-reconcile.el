;;; jaunder-reconcile.el --- Post inventory and reconciliation -*- lexical-binding: t; -*-

;; Copyright (C) 2026 Jaunder contributors

;;; Commentary:
;; Enumerate an AtomPub Collection and the directly contained Org files of one
;; configured root.  Inventory is side-effect-free; reconciliation classifies
;; it and renders a persistent selection report without transferring Posts.

;;; Code:

(require 'cl-lib)
(require 'ediff)
(require 'org)
(require 'dom)
(require 'url-parse)
(require 'jaunder-atom)
(require 'jaunder-config)
(require 'jaunder-org)
(require 'jaunder-transport)
(require 'jaunder-datetime)
(require 'jaunder-inventory)
(require 'jaunder-reconcile-operation)
(require 'jaunder-publish)
(require 'jaunder-debug)

(declare-function jaunder--pull-destination "jaunder-pull")
(declare-function jaunder--pull-destination-exists-p "jaunder-pull")
(declare-function jaunder--pull-member "jaunder-pull")
(declare-function jaunder--pull-stage-member "jaunder-pull")
(declare-function jaunder--pull-write-checkpoint "jaunder-pull")
(declare-function jaunder--pull-response-identity "jaunder-pull")
(declare-function jaunder--render-pulled-member "jaunder-pull")
(declare-function jaunder-pull-result-status "jaunder-pull")
(declare-function jaunder-pull-result-id "jaunder-pull")
(declare-function jaunder-pull-result-slug "jaunder-pull")
(declare-function jaunder-pull-result-etag "jaunder-pull")
(declare-function jaunder-pull-result-synced-at "jaunder-pull")
(declare-function jaunder-pull-result-http-status "jaunder-pull")
(declare-function jaunder-pull-result-local-effect "jaunder-pull")

(defvar jaunder--pull-link-inventory)
(defvar jaunder--pull-original-proof)

(cl-defstruct (jaunder-reconcile-row
               (:constructor jaunder--make-reconcile-row))
  "One immutable classification in a reconciliation report."
  key state local member reason detail conflict local-sha256 remote-etag)

(cl-defstruct (jaunder-reconcile-report
               (:constructor jaunder--make-reconcile-report))
  "The complete reconciliation result for one configured root."
  root inventory rows)

(cl-defstruct (jaunder-reconcile-result
               (:constructor jaunder--make-reconcile-result))
  "One terminal outcome recorded by the reconciliation batch executor."
  action row-key outcome post-id slug etag synced-at http-status local-effect
  reason detail)

(defvar-local jaunder-reconcile-report nil
  "The inventory report currently displayed in this reconciliation buffer.")

(defvar-local jaunder-reconcile-marks nil
  "Hash table of stable row keys explicitly marked in this reconciliation buffer.")

(defvar-local jaunder-reconcile-last-batch-results nil
  "Ordered terminal results from the most recent batch in this buffer.")

(defvar jaunder--reconcile-progress-context nil
  "Dynamically bound (POSITION TOTAL POST-ID) for one selected pull row.")

(defvar jaunder--reconcile-progress-stage nil
  "Current selected pull stage, retained for a failed row's diagnosis.")

(defvar jaunder--reconcile-batch-refresh-progress nil
  "Non-nil while a selected-pull batch refreshes its final report.")

(defun jaunder--reconcile-pull-progress (stage)
  "Display STAGE for the selected Post before blocking on its next step."
  (when jaunder--reconcile-progress-context
    (setq jaunder--reconcile-progress-stage stage)
    (pcase-let ((`(,position ,total ,id) jaunder--reconcile-progress-context))
      (message "Jaunder pull: %d/%d Post %s — %s" position total id stage))
    (redisplay)))

(defun jaunder--reconcile-pull-error-detail (err)
  "Describe condition or error text ERR with the selected pull stage."
  (let ((detail (if (stringp err) err (error-message-string err))))
    (if jaunder--reconcile-progress-stage
        (format "%s: %s" jaunder--reconcile-progress-stage detail)
      detail)))

(cl-defstruct (jaunder-reconcile-merge-session
               (:constructor jaunder--make-reconcile-merge-session))
  "One conflict's review evidence and independently editable scratch result."
  row report-buffer path scratch local-view remote-view ediff-active ediff-ready)

(defvar-local jaunder-reconcile-merge-session nil
  "The owned conflict merge session of the current scratch buffer.")

(defvar-local jaunder-reconcile-merge-last-result nil
  "Last explicit completion result, also retained on the merge scratch.")

(defvar-local jaunder-reconcile-merge-allow-kill nil
  "Non-nil only during an explicit, confirmed scratch cleanup.")

(defun jaunder--reconcile-merge-confirm-kill ()
  "Require an explicit discard before any ordinary scratch-buffer kill."
  (or jaunder-reconcile-merge-allow-kill
      (not jaunder-reconcile-merge-session)
      (y-or-n-p "Discard the reconciliation merge scratch permanently? ")))

(defun jaunder--reconcile-merge-on-kill ()
  "Close orphaned snapshots when a confirmed kill discards inactive scratch."
  (when (and jaunder-reconcile-merge-session
             (not (jaunder-reconcile-merge-session-ediff-active
                   jaunder-reconcile-merge-session)))
    (jaunder--reconcile-merge-close-views jaunder-reconcile-merge-session)))

(define-derived-mode jaunder-reconcile-merge-mode org-mode "Jaunder-Merge"
  "Edit authored content for a reviewed two-way conflict merge.
Ediff quit does not publish; `C-c C-c' explicitly completes, `C-c C-k'
retains the scratch, and `C-c C-d' explicitly discards it.  Client-managed
JAUNDER metadata is restored from the reviewed local Post on completion."
  (define-key jaunder-reconcile-merge-mode-map (kbd "C-c C-c")
              #'jaunder-reconcile-merge-finish)
  (define-key jaunder-reconcile-merge-mode-map (kbd "C-c C-k")
              #'jaunder-reconcile-merge-cancel)
  (define-key jaunder-reconcile-merge-mode-map (kbd "C-c C-d")
              #'jaunder-reconcile-merge-discard)
  (add-hook 'kill-buffer-query-functions
            #'jaunder--reconcile-merge-confirm-kill nil t)
  (add-hook 'kill-buffer-hook #'jaunder--reconcile-merge-on-kill nil t))

(define-derived-mode jaunder-reconcile-report-mode special-mode "Jaunder-Reconcile"
  "Major mode for a Jaunder reconciliation report.
Mark rows or select a region for bulk actions; `e' merges the row at point."
  (setq-local truncate-lines t)
  (define-key jaunder-reconcile-report-mode-map "m" #'jaunder-reconcile-toggle-mark)
  (define-key jaunder-reconcile-report-mode-map "p" #'jaunder-reconcile-push-selected)
  (define-key jaunder-reconcile-report-mode-map "f" #'jaunder-reconcile-pull-selected)
  (define-key jaunder-reconcile-report-mode-map "l" #'jaunder-reconcile-keep-local-selected)
  (define-key jaunder-reconcile-report-mode-map "r" #'jaunder-reconcile-keep-remote-selected)
  (define-key jaunder-reconcile-report-mode-map "e" #'jaunder-reconcile-merge-selected)
  (define-key jaunder-reconcile-report-mode-map "g" #'jaunder-reconcile-refresh)
  (define-key jaunder-reconcile-report-mode-map "D" #'jaunder-reconcile-delete-selected))

(defun jaunder--reconcile-conflict-key (conflict)
  "Return a deterministic identity for inventory CONFLICT."
  (format "conflict:local=%s;post=%s"
          (mapconcat #'jaunder-inventory-local-path
                     (jaunder-inventory-conflict-locals conflict) ",")
          (mapconcat #'jaunder-inventory-member-id
                     (jaunder-inventory-conflict-members conflict) ",")))

(defun jaunder--reconcile-stable-row-key (row)
  "Return ROW's stable identity, deriving it from its inventory identity if needed."
  (or (jaunder-reconcile-row-key row)
      (let ((local (jaunder-reconcile-row-local row))
            (member (jaunder-reconcile-row-member row)))
        (setf (jaunder-reconcile-row-key row)
              (cond ((and member (jaunder-inventory-member-id member))
                     (format "post:%s" (jaunder-inventory-member-id member)))
                    (local (format "local:%s" (jaunder-inventory-local-path local)))
                    (t (jaunder--reconcile-conflict-key
                        (jaunder-reconcile-row-conflict row))))))))

(defun jaunder--reconcile-row-key-position (row-key)
  "Return this buffer position for ROW-KEY, comparing stable string identities."
  (let ((position (point-min)) found)
    (while (and (< position (point-max)) (not found))
      (if (equal (get-text-property position 'jaunder-reconcile-row-key) row-key)
          (setq found position)
        (setq position (next-single-property-change
                        position 'jaunder-reconcile-row-key nil (point-max)))))
    found))

(defun jaunder--reconcile-displayed-rows (report)
  "Return REPORT rows in the exact state-section order rendered to its buffer."
  (apply #'append
         (mapcar
          (lambda (state)
            (cl-remove-if-not (lambda (row) (eq (jaunder-reconcile-row-state row) state))
                              (jaunder-reconcile-report-rows report)))
          jaunder--reconcile-state-order)))

(defun jaunder--reconcile-resolve-selection (report marks region)
  "Return REPORT rows selected by REGION or MARKS in displayed order.
REGION is an inclusive zero-based cons of displayed-row indexes and takes
precedence over arbitrary MARKS when present.  Selection never changes row
eligibility."
  (let ((rows (jaunder--reconcile-displayed-rows report)))
    (if region
        (cl-loop for row in rows for index from 0
                 when (and (<= (car region) index) (<= index (cdr region)))
                 collect row)
      (cl-remove-if-not (lambda (row)
                          (gethash (jaunder--reconcile-stable-row-key row) marks))
                        rows))))

(defun jaunder-reconcile-selected-rows ()
  "Return marked rows, or rows touched by the active contiguous region.
The returned rows retain report display order rather than point traversal order.
An active region selects only its rows, including when it touches none."
  (if (use-region-p)
      (let ((region-keys (make-hash-table :test #'equal))
            (position (region-beginning))
            (end (region-end)))
        (while (< position end)
          (let ((row (get-text-property position 'jaunder-reconcile-row))
                (next (next-single-property-change
                       position 'jaunder-reconcile-row nil end)))
            (when row
              (puthash (jaunder--reconcile-stable-row-key row) t region-keys))
            (setq position next)))
        (jaunder--reconcile-resolve-selection
         jaunder-reconcile-report region-keys nil))
    (jaunder--reconcile-resolve-selection
     jaunder-reconcile-report jaunder-reconcile-marks nil)))

(defun jaunder-reconcile-toggle-mark ()
  "Toggle the mark for the reconciliation row at point."
  (interactive)
  (let ((row (get-text-property (point) 'jaunder-reconcile-row)))
    (unless row (user-error "Point is not on a reconciliation row"))
    (let ((key (jaunder--reconcile-stable-row-key row)))
      (if (gethash key jaunder-reconcile-marks)
          (remhash key jaunder-reconcile-marks)
        (puthash key t jaunder-reconcile-marks)))
    (jaunder--render-reconcile-report jaunder-reconcile-report (current-buffer))))

(defconst jaunder--reconcile-state-order
  '(unchanged server-ahead local-ahead conflict unclassifiable
              orphan local-draft server-only inventory-conflict)
  "Stable section order for `jaunder-reconcile' reports.")


(defun jaunder--reconcile-synced-time (value)
  "Parse VALUE as the canonical UTC sync instant, or return nil."
  (when (and (stringp value)
             (string-match-p
              "\\`[0-9]\\{4\\}-[0-9]\\{2\\}-[0-9]\\{2\\}T[0-9]\\{2\\}:[0-9]\\{2\\}:[0-9]\\{2\\}Z\\'"
              value))
    (condition-case nil
        (let ((time (date-to-time value)))
          (and (equal (format-time-string "%Y-%m-%dT%H:%M:%SZ" time t) value) time))
      (error nil))))

(defun jaunder--reconcile-time-p (value)
  "Return non-nil when VALUE is accepted by Emacs time arithmetic."
  (and value
       (condition-case nil
           (time-add value 0)
         (error nil))))

(defun jaunder--reconcile-file-sha256 (path)
  "Return SHA-256 of PATH's literal bytes, or nil when it cannot be read."
  (condition-case nil
      (with-temp-buffer
        (set-buffer-multibyte nil)
        (insert-file-contents-literally path)
        (secure-hash 'sha256 (current-buffer)))
    (error nil)))

(defun jaunder--reconcile-visiting-buffer-matches-bytes-p (buffer sha256)
  "Return non-nil if clean visiting BUFFER encodes to reviewed SHA256.
A clean buffer can still be stale after an external disk replacement.  Encode
with its file coding system before comparing with the literal on-disk digest;
an unknown encoding fails closed rather than publishing stale content."
  (with-current-buffer buffer
    (condition-case nil
        (equal (secure-hash
                'sha256
                (encode-coding-string
                 (buffer-substring-no-properties (point-min) (point-max))
                 (or buffer-file-coding-system 'utf-8-unix)))
               sha256)
      (error nil))))

(defun jaunder--classify-match (match outcome stored-etag synced-at mtime &optional persisted-local-ahead)
  "Classify MATCH using Member OUTCOME and its saved local synchronization state.
OUTCOME is either `(:error ERROR)' for a transport failure or `(:response
RESPONSE)'.  Prerequisites are checked in protocol order so each row has one
stable first failure reason."
  (let* ((response (plist-get outcome :response))
         (status (and response (plist-get response :status)))
         (reason
          (cond
           ((plist-get outcome :error) 'member-transport-error)
           ((not (integerp status)) 'member-http-error)
           ((= status 404) 'member-not-found)
           ((not (<= 200 status 299)) 'member-http-error)
           ((not (jaunder--strong-etag-p
                  (jaunder--response-header response "ETag"))) 'current-etag-invalid)
           ((not (jaunder--strong-etag-p stored-etag)) 'stored-etag-invalid)
           ((not (jaunder--reconcile-synced-time synced-at)) 'synced-at-invalid)
           ((not (jaunder--reconcile-time-p mtime))
            'file-mtime-unreadable))))
    (if reason
        (jaunder--make-reconcile-row :state 'unclassifiable :local
                                     (jaunder-inventory-match-local match)
                                     :member (jaunder-inventory-match-member match)
                                     :reason reason
                                     :detail (and (eq reason 'member-http-error) status))
      (let* ((current (jaunder--response-header response "ETag"))
             (synced (jaunder--reconcile-synced-time synced-at))
             (server-changed (not (equal current stored-etag)))
             ;; Recovery after a response-lost create may write ID and ETag
             ;; within the filesystem timestamp tolerance.  Its persisted
             ;; marker is authoritative until a conditional PUT succeeds.
             (local-changed (or (equal persisted-local-ahead "true")
                                (time-less-p (time-add synced 2) mtime))))
        (jaunder--make-reconcile-row
         :state (cond ((and server-changed local-changed) 'conflict)
                      ((or server-changed
                           (not (equal
                                 (jaunder-inventory-local-path
                                  (jaunder-inventory-match-local match))
                                 (expand-file-name
                                  (concat (jaunder-inventory-member-slug
                                           (jaunder-inventory-match-member match)) ".org")
                                  (file-name-directory
                                   (jaunder-inventory-local-path
                                    (jaunder-inventory-match-local match)))))))
                       'server-ahead)
                      (local-changed 'local-ahead)
                      (t 'unchanged))
         :local (jaunder-inventory-match-local match)
         :member (jaunder-inventory-match-member match)
         :local-sha256
         (jaunder--reconcile-file-sha256
          (jaunder-inventory-local-path (jaunder-inventory-match-local match)))
         :remote-etag current)))))

(defun jaunder--reconcile-local-markers (local)
  "Return LOCAL's saved ETag, sync instant, and mtime without signalling.
Marker and mtime reads fail independently so an unreadable timestamp cannot
hide otherwise valid synchronization markers."
  (let ((markers
         (condition-case nil
             (with-temp-buffer
               (insert-file-contents (jaunder-inventory-local-path local))
               (list (jaunder--inventory-buffer-property "JAUNDER_SYNCED")
                     (jaunder--inventory-buffer-property "JAUNDER_SYNCED_AT")
                     (jaunder--inventory-buffer-property "JAUNDER_LOCAL_AHEAD")))
           (error (list nil nil nil)))))
    (append markers
            (list
             (condition-case nil
                 (file-attribute-modification-time
                  (file-attributes (jaunder-inventory-local-path local)))
               (error nil))))))

(defun jaunder--reconcile-member-outcome (member)
  "Use MEMBER's Collection ETag, or fetch it and retain row-local errors."
  (let ((etag (jaunder-inventory-member-etag member)))
    (if etag
        ;; Preserve the existing classification prerequisites, but do not mistake
        ;; this preview evidence for a fresh Member response at mutation time.
        (list :response (list :status 200 :headers (list (cons "etag" etag))))
      (condition-case err
          (list :response (jaunder--http-request
                           "GET" (jaunder-inventory-member-edit-uri member)))
        (error (list :error err))))))

(defun jaunder--reconcile-match-row (match)
  "Fetch and classify one MATCH without letting its failure hide other rows."
  (let* ((markers (jaunder--reconcile-local-markers
                   (jaunder-inventory-match-local match))))
    (jaunder--classify-match
     match (jaunder--reconcile-member-outcome (jaunder-inventory-match-member match))
     (nth 0 markers) (nth 1 markers) (nth 3 markers) (nth 2 markers))))

(defun jaunder--reconcile-build-report (root inventory)
  "Build a total reconciliation report for ROOT from D1 INVENTORY."
  (let ((rows
         (append
          (mapcar #'jaunder--reconcile-match-row (jaunder-inventory-matched inventory))
          (mapcar (lambda (local) (jaunder--make-reconcile-row
                                   :state 'orphan :local local))
                  (jaunder-inventory-orphans inventory))
          (mapcar (lambda (local) (jaunder--make-reconcile-row
                                   :state 'local-draft :local local))
                  (jaunder-inventory-local-drafts inventory))
          (mapcar (lambda (member) (jaunder--make-reconcile-row
                                    :state 'server-only :member member))
                  (jaunder-inventory-server-only inventory))
          (mapcar (lambda (conflict) (jaunder--make-reconcile-row
                                      :state 'inventory-conflict :conflict conflict))
                  (jaunder-inventory-conflicts inventory)))))
    (jaunder--make-reconcile-report :root root :inventory inventory :rows rows)))

(defun jaunder--reconcile-row-label (row)
  "Return the deterministic human label for ROW."
  (let ((local (jaunder-reconcile-row-local row))
        (member (jaunder-reconcile-row-member row)))
    (cond (local (jaunder-inventory-local-path local))
          (member (format "%s (%s)" (jaunder-inventory-member-slug member)
                          (jaunder-inventory-member-id member)))
          (t "conflict group"))))

(defun jaunder--reconcile-render-conflict (conflict)
  "Insert deterministic details for one inventory CONFLICT."
  (insert (format "  kinds: %s\n" (mapconcat #'symbol-name
                                             (jaunder-inventory-conflict-kinds conflict) ", ")))
  (dolist (local (jaunder-inventory-conflict-locals conflict))
    (insert (format "  local: %s id=%s\n" (jaunder-inventory-local-path local)
                    (or (jaunder-inventory-local-id local) ""))))
  (dolist (member (jaunder-inventory-conflict-members conflict))
    (insert (format "  Member: %s id=%s slug=%s\n"
                    (jaunder-inventory-member-edit-uri member)
                    (jaunder-inventory-member-id member)
                    (jaunder-inventory-member-slug member)))))

(defun jaunder--reconcile-render-result (result)
  "Insert one terminal batch RESULT."
  (insert (format "- %s %s: %s"
                  (jaunder-reconcile-result-action result)
                  (jaunder-reconcile-result-row-key result)
                  (jaunder-reconcile-result-outcome result)))
  (dolist (field `(("Post ID" . ,(jaunder-reconcile-result-post-id result))
                   ("slug" . ,(jaunder-reconcile-result-slug result))
                   ("ETag" . ,(jaunder-reconcile-result-etag result))
                   ("synced at" . ,(jaunder-reconcile-result-synced-at result))
                   ("HTTP" . ,(jaunder-reconcile-result-http-status result))
                   ("local effect" . ,(jaunder-reconcile-result-local-effect result))))
    (when (cdr field) (insert (format "; %s=%s" (car field) (cdr field)))))
  (if (jaunder-reconcile-result-reason result)
      (progn
        (insert (format "; %s" (jaunder-reconcile-result-reason result)))
        (when (jaunder-reconcile-result-detail result)
          (insert (format " (%s)" (jaunder-reconcile-result-detail result)))))
    (when (jaunder-reconcile-result-detail result)
      (insert (format "; %s" (jaunder-reconcile-result-detail result)))))
  (insert "\n"))

(defun jaunder--render-reconcile-report (report &optional target)
  "Render REPORT and TARGET's retained batch summary into its persistent buffer."
  (let ((buffer (or target (get-buffer-create "*Jaunder Reconcile*"))))
    (with-current-buffer buffer
      (let* ((row (get-text-property (point) 'jaunder-reconcile-row))
             (row-key (and row (jaunder--reconcile-stable-row-key row)))
             (column (current-column))
             (inhibit-read-only t))
        (unless (derived-mode-p 'jaunder-reconcile-report-mode)
          (jaunder-reconcile-report-mode))
        (setq-local jaunder-reconcile-report report)
        (unless jaunder-reconcile-marks
          (setq-local jaunder-reconcile-marks (make-hash-table :test #'equal)))
        (erase-buffer)
        (insert (format "Jaunder reconciliation: %s\n\n"
                        (jaunder-reconcile-report-root report)))
        (dolist (state jaunder--reconcile-state-order)
          (let ((rows (cl-remove-if-not
                       (lambda (row) (eq (jaunder-reconcile-row-state row) state))
                       (jaunder-reconcile-report-rows report))))
            (insert (format "%s (%d)\n" state (length rows)))
            (dolist (row rows)
              (let ((start (point))
                    (marked (gethash (jaunder--reconcile-stable-row-key row)
                                     jaunder-reconcile-marks)))
                (insert (format "%s %s" (if marked "*" "-")
                                (jaunder--reconcile-row-label row)))
                (when (jaunder-reconcile-row-reason row)
                  (insert (format ": %s" (jaunder-reconcile-row-reason row)))
                  (when (jaunder-reconcile-row-detail row)
                    (insert (format " (%s)" (jaunder-reconcile-row-detail row)))))
                (insert "\n")
                (add-text-properties start (point)
                                     `(jaunder-reconcile-row ,row
                                                             jaunder-reconcile-row-key
                                                             ,(jaunder--reconcile-stable-row-key row))))
              (when (eq state 'inventory-conflict)
                (jaunder--reconcile-render-conflict
                 (jaunder-reconcile-row-conflict row))))
            (insert "\n")))
        (when jaunder-reconcile-last-batch-results
          (insert "Last batch\n")
          (dolist (result jaunder-reconcile-last-batch-results)
            (jaunder--reconcile-render-result result))
          (insert "\n"))
        (if row-key
            (let ((row-position (jaunder--reconcile-row-key-position row-key)))
              (if row-position
                  (progn
                    (goto-char row-position)
                    (move-to-column column))
                (goto-char (point-min))))
          (goto-char (point-min)))))
    buffer))

(defun jaunder--reconcile-terminal-result (action row value)
  "Convert VALUE from one ROW operation into the required terminal result shape."
  (let ((result (jaunder--make-reconcile-result
                 :action action :row-key (jaunder--reconcile-stable-row-key row)
                 :outcome (or (plist-get value :outcome) 'success)
                 :post-id (plist-get value :post-id) :slug (plist-get value :slug)
                 :etag (plist-get value :etag)
                 :synced-at (unless (eq action 'delete) (plist-get value :synced-at))
                 :http-status (plist-get value :http-status)
                 :local-effect (or (plist-get value :local-effect) 'unchanged)
                 :reason (plist-get value :reason) :detail (plist-get value :detail))))
    result))

(defun jaunder--reconcile-pruned-marks (report marks)
  "Return MARKS restricted to stable row identities present in REPORT."
  (let ((present (make-hash-table :test #'equal))
        (retained (make-hash-table :test #'equal)))
    (dolist (row (jaunder-reconcile-report-rows report))
      (puthash (jaunder--reconcile-stable-row-key row) t present))
    (maphash (lambda (key value)
               (when (gethash key present)
                 (puthash key value retained)))
             marks)
    retained))

(defun jaunder--reconcile-refresh-buffer (buffer)
  "Rebuild BUFFER's report from fresh inventory without discarding its results."
  (jaunder--with-debug-operation "report.refresh" nil
                                 (with-current-buffer buffer
                                   (let* ((root (jaunder-reconcile-report-root jaunder-reconcile-report))
                                          ;; Build before changing the report buffer, so a failed inventory leaves its
                                          ;; existing reviewable state available to the User.
                                          (report (jaunder--call-with-blog
                                                   root
                                                   (lambda ()
                                                     (jaunder--reconcile-build-report
                                                      root (jaunder--inventory-for-root root)))))
                                          (marks (jaunder--reconcile-pruned-marks report jaunder-reconcile-marks))
                                          (text (buffer-substring (point-min) (point-max)))
                                          (point (point))
                                          (previous-report jaunder-reconcile-report)
                                          (previous-marks jaunder-reconcile-marks)
                                          (previous-results jaunder-reconcile-last-batch-results)
                                          (modified (buffer-modified-p)))
                                     (condition-case err
                                         (progn
                                           (setq-local jaunder-reconcile-marks marks)
                                           (jaunder--render-reconcile-report report buffer))
                                       (error
                                        (let ((inhibit-read-only t)
                                              (inhibit-modification-hooks t))
                                          (erase-buffer)
                                          (insert text))
                                        (setq-local jaunder-reconcile-report previous-report)
                                        (setq-local jaunder-reconcile-marks previous-marks)
                                        (setq-local jaunder-reconcile-last-batch-results previous-results)
                                        (goto-char point)
                                        (set-buffer-modified-p modified)
                                        (signal (car err) (cdr err))))))))

(defun jaunder--reconcile-with-progress (success failure work)
  "Display synchronous WORK before blocking; report SUCCESS or FAILURE at exit."
  (message "Jaunder reconcile: fetching and classifying Posts...")
  ;; A message alone may remain unpainted until synchronous curl returns.
  (redisplay)
  (condition-case err
      (prog1 (funcall work)
        (message "Jaunder reconcile: %s" success))
    (error
     (message "Jaunder reconcile: %s" failure)
     (signal (car err) (cdr err)))
    (quit
     (message "Jaunder reconcile: %s" failure)
     (signal (car err) (cdr err)))))

(defun jaunder-reconcile-refresh ()
  "Refresh the current reconciliation report from local and remote state."
  (interactive)
  (jaunder--reconcile-with-progress
   "report ready" "report refresh failed"
   (lambda () (jaunder--reconcile-refresh-buffer (current-buffer)))))

(defun jaunder--reconcile-show-results-and-refresh (buffer)
  "Show BUFFER's terminal results before a fallible fresh inventory refresh.
A failed refresh must not erase the ordered recovery evidence.  The stale
report remains visibly reviewable, and the User must refresh before retrying."
  (with-current-buffer buffer
    (jaunder--render-reconcile-report jaunder-reconcile-report buffer))
  (condition-case err
      (let ((jaunder--inventory-page-progress
             (when jaunder--reconcile-batch-refresh-progress
               (lambda (page)
                 (message "Jaunder pull: report refresh Collection page %d complete" page)
                 (redisplay)))))
        (jaunder--reconcile-refresh-buffer buffer))
    (error
     (message "Jaunder reconcile: refresh failed; Last batch retained: %s"
              (error-message-string err))
     'refresh-failed)))

(defun jaunder--reconcile-debug-action (action)
  "Return a literal diagnostic action for native ACTION without serializing it."
  (pcase action
    ('push "push") ('pull "pull") ('delete "delete")
    ('keep-local "keep-local") ('keep-remote "keep-remote") ('merge "merge")
    (_ "unknown")))

(defun jaunder--reconcile-debug-decision (value)
  "Map native row VALUE to a literal diagnostic decision, never its details."
  (pcase (plist-get value :outcome)
    ('success "proceed") ('blocked "blocked") ('no-op "no-op")
    ('partial "partial") ('unknown "remote-unknown") (_ "unknown")))

(defun jaunder--reconcile-debug-batch-decision (buffer status)
  "Summarize BUFFER's retained native results and batch STATUS without payloads."
  (let ((outcomes (with-current-buffer buffer
                    (mapcar #'jaunder-reconcile-result-outcome jaunder-reconcile-last-batch-results))))
    (cond
     ((eq status 'refresh-failed) "partial")
     ((eq status 'cancelled) (if outcomes "partial" "no-op"))
     ((memq 'unknown outcomes) "remote-unknown")
     ((memq 'partial outcomes) "partial")
     ((cl-some (lambda (outcome) (not (memq outcome '(success blocked no-op)))) outcomes) "unknown")
     ((memq 'blocked outcomes) (if (memq 'success outcomes) "partial" "blocked"))
     ((memq 'success outcomes) "proceed")
     (t "no-op"))))

(defmacro jaunder--with-reconcile-row-debug (action &rest body)
  "Run actual row BODY once with literal ACTION and a native decision projection."
  (declare (indent 1) (debug (form body)))
  (let ((value (make-symbol "value")))
    `(jaunder--with-debug-operation "reconcile.row" (action ,action decision "unknown")
                                    (let ((,value (progn ,@body)))
                                      (jaunder--debug-fields decision (jaunder--reconcile-debug-decision ,value))
                                      ,value))))

(defmacro jaunder--with-reconcile-batch-debug (action buffer &rest body)
  "Run actual batch BODY once, projecting only native ACTION and BUFFER outcomes."
  ;; Both projections execute in deferred diagnostic thunks, not caller context.
  (declare (indent 2) (debug ([&define def-form] [&define def-form] body)))
  (let ((value (make-symbol "value")))
    `(jaunder--with-debug-operation "reconcile.batch"
                                    (action (jaunder--reconcile-debug-action ,action) decision "unknown")
                                    (let ((,value (progn ,@body)))
                                      (jaunder--debug-fields decision (jaunder--reconcile-debug-batch-decision ,buffer ,value))
                                      ,value))))

(defun jaunder--reconcile-execute-batch (buffer rows action operation &optional cancelled-p)
  "Run OPERATION for ROWS sequentially, retaining every terminal result in BUFFER.
CANCELLED-P is checked only between completed items.  OPERATION receives one
row and returns a result plist; its independent errors become failed results."
  (jaunder--with-reconcile-batch-debug action buffer
                                       (let* ((root (with-current-buffer buffer
                                                      (jaunder-reconcile-report-root
                                                       jaunder-reconcile-report)))
                                              )
                                         (with-current-buffer buffer
                                           (setq-local jaunder-reconcile-last-batch-results nil)
                                           (setq rows (cl-remove-if-not
                                                       (lambda (displayed-row)
                                                         (memq displayed-row rows))
                                                       (jaunder--reconcile-displayed-rows jaunder-reconcile-report))))
                                         (let ((total (length rows)) (completed 0) cancelled)
                                           (jaunder--call-with-reconcile-operation
                                            root (jaunder--active-base-url) (jaunder--active-username)
                                            (lambda ()
                                              (dolist (row rows)
                                                (unless cancelled
                                                  (if (or (and cancelled-p (funcall cancelled-p)) quit-flag)
                                                      (setq cancelled t)
                                                    (setq completed (1+ completed))
                                                    (let* ((jaunder--reconcile-progress-context
                                                            (when (eq action 'pull)
                                                              (list completed total (or (jaunder--reconcile-row-post-id row) "unknown"))))
                                                           (jaunder--reconcile-progress-stage nil)
                                                           quit-requested value)
                                                      (if jaunder--reconcile-progress-context
                                                          (jaunder--reconcile-pull-progress "starting")
                                                        (message "Jaunder %s: %d/%d" action completed total))
                                                      (setq value
                                                            (condition-case err
                                                                (let ((inhibit-quit t))
                                                                  (jaunder--call-with-operation-write-receipt
                                                                   (lambda ()
                                                                     (condition-case err
                                                                         (prog1 (funcall operation row)
                                                                           (setq quit-requested quit-flag quit-flag nil))
                                                                       (error (jaunder--operation-write-failure
                                                                               err (jaunder--reconcile-pull-error-detail err)))
                                                                       (quit
                                                                        (if (memq (jaunder--operation-write-phase) '(unknown confirmed))
                                                                            (progn
                                                                              (setq quit-requested t quit-flag nil)
                                                                              (jaunder--operation-write-failure err))
                                                                          (signal (car err) (cdr err))))))))
                                                              (error (list :outcome 'failed :reason 'operation-failed
                                                                           :detail (jaunder--reconcile-pull-error-detail err)))))
                                                      (with-current-buffer buffer
                                                        (setq-local jaunder-reconcile-last-batch-results
                                                                    (append jaunder-reconcile-last-batch-results
                                                                            (list (jaunder--reconcile-terminal-result action row value)))))
                                                      (when jaunder--reconcile-progress-context
                                                        (jaunder--reconcile-pull-progress
                                                         (format "%s%s" (or (plist-get value :outcome) 'failed)
                                                                 (if (plist-get value :reason)
                                                                     (format " (%s)" (plist-get value :reason)) ""))))
                                                      (when (or quit-requested
                                                                (and cancelled-p (funcall cancelled-p)) quit-flag)
                                                        (setq cancelled t))))))
                                              ))
                                           (when cancelled (setq quit-flag nil))
                                           (when (eq action 'pull)
                                             (message "Jaunder pull: refreshing report after %d/%d Posts" completed total)
                                             (redisplay))
                                           (let* ((jaunder--reconcile-batch-refresh-progress (eq action 'pull))
                                                  (refresh (jaunder--call-without-reconcile-operation
                                                            (lambda ()
                                                              (jaunder--reconcile-show-results-and-refresh buffer)))))
                                             (if (eq refresh 'refresh-failed)
                                                 'refresh-failed
                                               (when (eq action 'pull)
                                                 (message "Jaunder pull: %s after %d/%d Posts"
                                                          (if cancelled "cancelled" "batch complete") completed total))
                                               (if cancelled 'cancelled 'completed)))))))

(defun jaunder--reconcile-row-post-id (row)
  "Return ROW's remote Post ID, when it has an unambiguous Member."
  (let ((member (jaunder-reconcile-row-member row)))
    (and member (jaunder-inventory-member-id member))))

(defun jaunder--reconcile-row-slug (row)
  "Return ROW's reviewed server slug, when it has one."
  (let ((member (jaunder-reconcile-row-member row)))
    (and member (jaunder-inventory-member-slug member))))

(defun jaunder--reconcile-blocked (row reason &optional detail)
  "Return a complete blocked operation value for ROW naming REASON and DETAIL."
  (list :outcome 'blocked
        :post-id (jaunder--reconcile-row-post-id row)
        :slug (jaunder--reconcile-row-slug row)
        :local-effect 'unchanged :reason reason :detail detail))

(defun jaunder--reconcile-current-local-id (row)
  "Re-read ROW's local file identity without trusting its report snapshot."
  (let ((local (jaunder-reconcile-row-local row)))
    (and local (file-regular-p (jaunder-inventory-local-path local))
         (condition-case nil
             (jaunder--canonical-post-id
              (jaunder--read-local-id (jaunder-inventory-local-path local)))
           (error nil)))))

(defun jaunder--reconcile-call-with-source-buffer (path function)
  "Call FUNCTION in PATH's buffer and retain only a buffer the user already had.
A reconcile-owned buffer is discarded even after a failed operation; its
internal metadata writes are either already checkpointed or deliberately
uncommitted."
  (let* ((existing (get-file-buffer path))
         (buffer (or existing (find-file-noselect path))))
    (unwind-protect
        (with-current-buffer buffer (funcall function))
      (unless existing
        (when (buffer-live-p buffer)
          (with-current-buffer buffer (set-buffer-modified-p nil))
          (kill-buffer buffer))))))

(defun jaunder--reconcile-local-mutation-safety-reason (row)
  "Return a stale or modified local safety reason immediately before mutation."
  (let* ((local (jaunder-reconcile-row-local row))
         (path (and local (jaunder-inventory-local-path local)))
         (expected (if (eq (jaunder-reconcile-row-state row) 'local-draft)
                       nil (jaunder--reconcile-row-post-id row))))
    (when local
      (jaunder--reconcile-call-with-source-buffer
       path
       (lambda ()
         (cond
          ((buffer-modified-p) 'local-buffer-modified)
          ((not (equal (jaunder--canonical-post-id
                        (jaunder--buffer-property "JAUNDER_ID")) expected))
           (if expected 'matched-identity-changed 'draft-identity-changed))
          ((not (equal (jaunder--reconcile-current-local-id row) expected))
           (if expected 'matched-identity-changed 'draft-identity-changed))))))))

(defun jaunder--reconcile-push-row (row)
  "Push an eligible ROW through the durable ordinary publish path."
  (jaunder--with-reconcile-row-debug "push"
                                     (pcase (jaunder-reconcile-row-state row)
                                       ('unchanged
                                        (list :outcome 'no-op :post-id (jaunder--reconcile-row-post-id row)
                                              :slug (jaunder--reconcile-row-slug row) :local-effect 'unchanged
                                              :reason 'unchanged))
                                       ((or 'local-draft 'local-ahead)
                                        (let* ((local (jaunder-reconcile-row-local row))
                                               (path (and local (jaunder-inventory-local-path local)))
                                               (identity-reason (jaunder--reconcile-local-mutation-safety-reason row)))
                                          (cond
                                           ((not (and path (file-regular-p path)))
                                            (jaunder--reconcile-blocked row 'local-file-missing))
                                           (identity-reason (jaunder--reconcile-blocked row identity-reason))
                                           (t
                                            (jaunder--reconcile-call-with-source-buffer
                                             path
                                             (lambda ()
                                               (let* ((published (jaunder-publish))
                                                      (destination (buffer-file-name)))
                                                 (list :outcome (if (jaunder--operation-created-identity-unresolved-p) 'partial 'success)
                                                       :reason (when (jaunder--operation-created-identity-unresolved-p)
                                                                 'create-identity-unresolved)
                                                       :post-id (unless (jaunder--operation-created-identity-unresolved-p)
                                                                  (jaunder--buffer-property "JAUNDER_ID"))
                                                       :slug (jaunder--buffer-property "JAUNDER_SLUG")
                                                       :etag (jaunder--buffer-property "JAUNDER_SYNCED")
                                                       :synced-at (jaunder--buffer-property "JAUNDER_SYNCED_AT")
                                                       :http-status (plist-get published :http-status)
                                                       :local-effect
                                                       (if (eq (jaunder-reconcile-row-state row) 'local-draft)
                                                           'created 'updated)
                                                       :detail (when (and destination (not (equal path destination)))
                                                                 (format "renamed %s -> %s" path destination))))))))))
                                       (_ (jaunder--reconcile-blocked row 'push-ineligible
                                                                      (jaunder-reconcile-row-state row))))))

(defun jaunder--reconcile-delete-etag (row)
  "Fetch ROW's current strong ETag for explicit remote deletion.
Return a plist suitable for a terminal result; no DELETE is sent here."
  (let* ((member (jaunder-reconcile-row-member row))
         (id (jaunder--reconcile-row-post-id row))
         (slug (jaunder--reconcile-row-slug row)))
    (condition-case err
        (progn
          (when (and (jaunder-reconcile-row-local row)
                     (jaunder--operation-unresolved-create-path-p
                      (jaunder-inventory-local-path (jaunder-reconcile-row-local row))))
            (error "jaunder: Post create identity is unresolved"))
          (let* ((response (jaunder--http-request
                            "GET" (jaunder-inventory-member-edit-uri member)))
                 (status (plist-get response :status))
                 (etag (jaunder--response-header response "ETag")))
            (if (and (integerp status) (<= 200 status 299)
                     (jaunder--strong-etag-p etag))
                (list :post-id id :slug slug :etag etag :http-status status)
              (list :outcome 'blocked :post-id id :slug slug :etag etag
                    :http-status status :local-effect 'unchanged
                    :reason (if (and (integerp status) (<= 200 status 299))
                                'current-etag-invalid 'member-http-error)))))
      (error (list :outcome 'blocked :post-id id :slug slug
                   :local-effect 'unchanged :reason 'member-transport-error
                   :detail (error-message-string err))))))

(defun jaunder--reconcile-delete-preflight (row)
  "Return a blocking reason when ROW cannot be deleted safely right now."
  (and (jaunder-reconcile-row-local row)
       (jaunder--reconcile-local-mutation-safety-reason row)))

(defun jaunder--reconcile-delete-local-file (row)
  "Remove ROW's matched local file after 204, or return a preservation result."
  (let ((local (jaunder-reconcile-row-local row)))
    (if (null local)
        (list :local-effect 'unchanged)
      (let* ((path (jaunder-inventory-local-path local))
             (buffer (get-file-buffer path)))
        (cond
         ((and (buffer-live-p buffer) (buffer-modified-p buffer))
          (list :local-effect 'preserved :reason 'local-buffer-modified))
         ((not (equal (jaunder--reconcile-current-local-id row)
                      (jaunder--reconcile-row-post-id row)))
          (list :local-effect 'preserved :reason 'matched-identity-changed))
         (t
          (delete-file path)
          (if (and (buffer-live-p buffer) (not (kill-buffer buffer)))
              (list :local-effect 'removed-buffer-retained)
            (list :local-effect 'removed))))))))

(defun jaunder--reconcile-delete-row (row reviewed)
  "Delete ROW with its pre-confirmation REVIEWED strong ETag."
  (jaunder--with-reconcile-row-debug "delete"
                                     (if (plist-get reviewed :outcome)
                                         reviewed
                                       (let ((id (plist-get reviewed :post-id))
                                             (slug (plist-get reviewed :slug))
                                             (etag (plist-get reviewed :etag)))
                                         (let ((preflight (jaunder--reconcile-delete-preflight row)))
                                           (if preflight
                                               (jaunder--reconcile-blocked row preflight)
                                             (condition-case err
                                                 (let* ((response (jaunder--operation-send-post-write
                                                                   "DELETE" (jaunder--member-url id) nil nil
                                                                   (list (cons "If-Match" etag))) )
                                                        (status (plist-get response :status)))
                                                   (if (eq status 204)
                                                       (let ((local (jaunder--reconcile-delete-local-file row)))
                                                         (list :outcome (if (and (eq (jaunder--operation-write-phase) 'confirmed)
                                                                                 (eq (plist-get local :local-effect) 'preserved))
                                                                            'partial 'success)
                                                               :post-id id :slug slug :etag etag
                                                               :http-status status
                                                               :local-effect (plist-get local :local-effect)
                                                               :reason (plist-get local :reason)))
                                                     (list :outcome (pcase (jaunder--operation-write-phase)
                                                                      ('unknown 'unknown) ('confirmed 'partial) (_ 'failed))
                                                           :post-id id :slug slug :etag etag
                                                           :http-status status :local-effect 'unchanged
                                                           :reason (if (eq (jaunder--operation-write-phase) 'unknown)
                                                                       'remote-outcome-unknown
                                                                     (if (eq status 412) 'etag-stale 'delete-http-error))
                                                           :detail (when (eq (jaunder--operation-write-phase) 'unknown)
                                                                     (format "DELETE outcome unknown (HTTP %s); reconcile before retrying" status)))))
                                               (error (if (memq (jaunder--operation-write-phase) '(confirmed unknown))
                                                          (append (jaunder--operation-write-failure err)
                                                                  (list :slug slug :etag etag))
                                                        (list :outcome 'failed :post-id id :slug slug :etag etag
                                                              :local-effect 'unchanged :reason 'delete-transport-error
                                                              :detail (error-message-string err)))))))))))

(defun jaunder--reconcile-pull-destination (_row slug)
  "Return the report root's canonical direct-root destination for server SLUG."
  (jaunder--pull-destination
   (jaunder-reconcile-report-root jaunder-reconcile-report) slug))

(defun jaunder--reconcile-pull-preflight (row staged)
  "Return a blocking reason unless ROW remains safe for STAGED replacement."
  (jaunder--with-debug-operation "pull.preflight" ()
                                 (let* ((local (jaunder-reconcile-row-local row))
                                        (path (and local (jaunder-inventory-local-path local)))
                                        (id (jaunder--reconcile-row-post-id row))
                                        (expected-hash (jaunder-reconcile-row-local-sha256 row))
                                        (destination (jaunder--reconcile-pull-destination row (plist-get staged :slug)))
                                        (buffer (and path (get-file-buffer path))))
                                   (cond
                                    ((not (and path (file-regular-p path))) 'local-file-missing)
                                    ((not (and expected-hash
                                               (equal expected-hash (jaunder--reconcile-file-sha256 path))))
                                     'local-bytes-changed)
                                    ((and (buffer-live-p buffer) (buffer-modified-p buffer)) 'local-buffer-modified)
                                    ((and (buffer-live-p buffer)
                                          (not (jaunder--reconcile-visiting-buffer-matches-bytes-p
                                                buffer expected-hash))) 'local-buffer-stale)
                                    ((and (buffer-live-p buffer)
                                          (not (equal (jaunder--canonical-post-id
                                                       (with-current-buffer buffer
                                                         (jaunder--buffer-property "JAUNDER_ID"))) id)))
                                     'matched-identity-changed)
                                    ((not (equal (jaunder--reconcile-current-local-id row) id))
                                     'matched-identity-changed)
                                    ((and (not (equal path destination))
                                          (jaunder--pull-destination-exists-p destination))
                                     'pull-destination-occupied)))))

(defun jaunder--reconcile-pull-remote-revalidation (row etag)
  "Return structured current-ETag evidence for ROW against reviewed ETAG."
  (jaunder--with-debug-operation "pull.revalidate" ()
                                 (condition-case err
                                     (let* ((response (jaunder--http-request
                                                       "GET" (jaunder-inventory-member-edit-uri
                                                              (jaunder-reconcile-row-member row))))
                                            (status (plist-get response :status))
                                            (current (jaunder--response-header response "ETag")))
                                       (cond ((not (and (integerp status) (<= 200 status 299)))
                                              (list :reason 'pull-revalidation-http-error :http-status status :etag current))
                                             ((not (jaunder--strong-etag-p current))
                                              (list :reason 'pull-revalidation-etag-invalid :http-status status :etag current))
                                             ((not (equal etag current))
                                              (list :reason 'etag-stale :http-status status :etag current))
                                             (t (list :ok t :http-status status :etag current))))
                                   (error (list :reason 'pull-revalidation-transport-error
                                                :detail (jaunder--reconcile-pull-error-detail err))))))

(defun jaunder--reconcile-pull-unique-match (row)
  "Return current uniqueness proof for reviewed ROW without changing its authority."
  (let ((jaunder--inventory-page-progress
         (when jaunder--reconcile-progress-context
           (lambda (page)
             (jaunder--reconcile-pull-progress
              (format "Collection page %d complete" page))
             (setq jaunder--reconcile-progress-stage
                   (format "verifying Collection after page %d" page))))))
    (let ((match (jaunder--operation-unique-match
                  (jaunder-reconcile-report-root jaunder-reconcile-report)
                  (jaunder--reconcile-row-post-id row)
                  (jaunder-inventory-local-path (jaunder-reconcile-row-local row)))))
      (when (eq (plist-get match :reason) 'fresh-inventory-failed)
        (setq match (plist-put match :detail
                               (jaunder--reconcile-pull-error-detail
                                (plist-get match :detail)))))
      match)))

(defun jaunder--reconcile-pull-final-local-unique-match (row)
  "Return final local uniqueness proof for ROW immediately before replacement."
  (jaunder--operation-local-unique-match
   (jaunder-reconcile-report-root jaunder-reconcile-report)
   (jaunder--reconcile-row-post-id row)
   (jaunder-inventory-local-path (jaunder-reconcile-row-local row))))

(defun jaunder--reconcile-conflict-preflight (row)
  "Return fresh evidence for reviewed conflict ROW, or a blocked result.
No server or local Post mutation is authorized by the preview ETag alone."
  (let* ((member (jaunder-reconcile-row-member row))
         (local (jaunder-reconcile-row-local row))
         (reviewed (jaunder-reconcile-row-remote-etag row)))
    (cond
     ((not (eq (jaunder-reconcile-row-state row) 'conflict))
      (jaunder--reconcile-blocked row 'conflict-ineligible))
     ((not (and local member))
      (jaunder--reconcile-blocked row 'matched-identity-changed))
     ((not (jaunder--strong-etag-p reviewed))
      (jaunder--reconcile-blocked row 'reviewed-etag-invalid))
     (t
      (let ((local-reason
             (jaunder--reconcile-pull-preflight
              row (list :slug (jaunder-inventory-member-slug member)))))
        (if local-reason
            (jaunder--reconcile-blocked row local-reason)
          (let ((match (jaunder--reconcile-pull-unique-match row)))
            (if (not (plist-get match :ok))
                (jaunder--reconcile-blocked row (plist-get match :reason)
                                            (plist-get match :detail))
              (condition-case err
                  (let* ((response (jaunder--http-request
                                    "GET" (jaunder-inventory-member-edit-uri member)))
                         (status (plist-get response :status))
                         (etag (jaunder--response-header response "ETag")))
                    (cond
                     ((not (and (integerp status) (<= 200 status 299)))
                      (jaunder--reconcile-blocked row 'member-http-error status))
                     ((not (jaunder--strong-etag-p etag))
                      (jaunder--reconcile-blocked row 'current-etag-invalid))
                     ((not (equal reviewed etag))
                      (jaunder--reconcile-blocked row 'etag-stale))
                     (t
                      (let* ((body (plist-get response :body))
                             (identity
                              (condition-case nil
                                  (jaunder--pull-response-identity body)
                                (error nil)))
                             (edit-uris
                              (and identity
                                   (cdr (assq 'edit-uris
                                              (jaunder--harvest-response-fields body))))))
                        (if (and (equal identity
                                        (cons (jaunder-inventory-member-id member)
                                              (jaunder-inventory-member-slug member)))
                                 (equal edit-uris
                                        (list (jaunder-inventory-member-edit-uri member))))
                            (list :ok t :etag etag :http-status status)
                          (jaunder--reconcile-blocked row 'member-identity-changed))))))
                (error (jaunder--reconcile-blocked
                        row 'member-transport-error (error-message-string err))))))))))))

(defun jaunder--reconcile-replace-pulled-file (path destination bytes &optional staged-synced-at)
  "Atomically replace PATH then rename to DESTINATION, reporting committed state.
STAGED-SYNCED-AT renews the sync checkpoint immediately before installation."
  (jaunder--with-debug-operation "pull.install" ()
                                 (let ((temporary nil)
                                       synced-at)
                                   (unwind-protect
                                       (progn
                                         (setq temporary (make-temp-file
                                                          (expand-file-name ".jaunder-pull-" (file-name-directory path))))
                                         (setq synced-at (jaunder--pull-write-checkpoint
                                                          temporary bytes staged-synced-at))
                                         (rename-file temporary path t)
                                         (setq temporary nil)
                                         (let ((buffer (get-file-buffer path)))
                                           (when (buffer-live-p buffer)
                                             (with-current-buffer buffer (revert-buffer t t) (set-buffer-modified-p nil))))
                                         (if (equal path destination)
                                             (list :path path :local-effect 'replaced :synced-at synced-at)
                                           (condition-case err
                                               (progn
                                                 (rename-file path destination nil)
                                                 (let ((buffer (get-file-buffer path)))
                                                   (when (buffer-live-p buffer)
                                                     (with-current-buffer buffer
                                                       (set-visited-file-name destination t t)
                                                       (set-buffer-modified-p nil))))
                                                 (list :path destination :local-effect 'renamed :synced-at synced-at))
                                             (error (list :path path :local-effect 'replaced-at-old-path :synced-at synced-at
                                                          :detail (error-message-string err))))))
                                     (when (and temporary (file-exists-p temporary)) (delete-file temporary))))))

(defun jaunder--reconcile-preserve-legacy-audience (path bytes)
  "Carry PATH's exact leading audience header lines into staged Org BYTES.
Only used when a valid legacy service omits the Member audience.  The remote
Post body and all other metadata still come from the staged response."
  (with-temp-buffer
    (insert-file-contents path)
    (goto-char (point-min))
    (let ((case-fold-search t)
          lines)
      (while (looking-at-p org-keyword-regexp)
        (when (looking-at
               "^[ \t]*#\\+PROPERTY:[ \t]+JAUNDER_AUDIENCE\\(?:[ \t].*\\)?$")
          (push (buffer-substring-no-properties
                 (line-beginning-position) (line-end-position)) lines))
        (forward-line 1))
      (if (null lines)
          bytes
        (unless (string-match
                 "^#\\+PROPERTY: JAUNDER_STATUS [^\n]*\n" bytes)
          (error "jaunder: staged Post has no status header"))
        (let ((position (match-end 0)))
          (concat (substring bytes 0 position)
                  (mapconcat #'identity (nreverse lines) "\n") "\n"
                  (substring bytes position)))))))

(defun jaunder--reconcile-original-proof (root row)
  "Prove ROW's Org Media destinations under ROOT for its reviewed final slug.
The proof is intentionally derived only from the matched local Post, never
from a root search.  A later final check repeats its filesystem predicates.
Return nil for non-Org local source, preserving ordinary localization."
  (let* ((local (jaunder-reconcile-row-local row))
         (path (and local (jaunder-inventory-local-path local))))
    (when (and path (string-suffix-p ".org" path))
      (if (not (file-regular-p path))
          ;; The ordinary matched preflight reports this as a blocked Post;
          ;; staging must not turn it into an unrelated Media-proof failure.
          (jaunder--make-pull-media-original-proof :originals nil)
        (with-temp-buffer
          (insert-file-contents-literally path)
          ;; Posts are UTF-8 files; literal extraction avoids mode hooks, then
          ;; explicit decoding keeps authored Unicode destinations as characters.
          (decode-coding-region (point-min) (point-max) 'utf-8-unix)
          (org-mode)
          (let ((body (buffer-substring-no-properties
                       (jaunder--body-start) (point-max))))
            ;; Avoid filesystem proof work when no Org link can possibly select
            ;; an original; retain an empty proof so matched staging still owns
            ;; ordinary fallback installation.
            (if (string-match-p "\\[\\[" body)
                (jaunder--pull-media-prove-original-destinations
                 root path
                 (jaunder--pull-destination
                  root (jaunder--reconcile-row-slug row))
                 body)
              (jaunder--make-pull-media-original-proof :originals nil))))))))

(defun jaunder--reconcile-reuse-still-eligible-p (root reuse)
  "Return non-nil when REUSE's original still proves its expected literal bytes."
  (let* ((original (jaunder-pull-media-reuse-original reuse))
         (hash (jaunder-pull-media-original-destination-hash original)))
    (and (jaunder--pull-media-original-safe-regular-p
          root (jaunder-pull-media-original-destination-source-path original))
         (jaunder--pull-media-original-safe-regular-p
          root (jaunder-pull-media-original-destination-final-path original))
         (equal hash (jaunder--pull-media-file-sha256
                      (jaunder-pull-media-original-destination-source-path original)))
         (equal hash (jaunder--pull-media-file-sha256
                      (jaunder-pull-media-original-destination-final-path original))))))

(defun jaunder--reconcile-finalize-staged-media (root staged)
  "Revalidate STAGED reuse after every fallback installation, then rerender it.
This runs only at a matched consumer's boundary.  Rejected reuse consumes its
already verified bytes; it never fetches again.  Each pass removes at least one
reuse, so a finite stage settles without leaving a checked original stale after
arbitrary fallback work."
  (let ((media (plist-get staged :media-staged)))
    (if (null media)
        staged
      ;; Ordinary fallback installation can be lengthy, so it must happen before
      ;; the first original check and each newly rejected reuse triggers another
      ;; check of every surviving original.
      (jaunder--pull-media-finalize-staged root media)
      (let (rejected)
        (while (setq rejected
                     (cl-remove-if
                      (lambda (reuse)
                        (jaunder--reconcile-reuse-still-eligible-p root reuse))
                      (jaunder-pull-media-staged-reuses media)))
          (setq media
                (jaunder--pull-media-staged-with-reuse-fallbacks media rejected))
          (jaunder--pull-media-finalize-staged root media))
        (setq staged (plist-put (copy-sequence staged) :media-staged media))
        (plist-put staged :bytes
                   (jaunder--render-pulled-member
                    (plist-get staged :pulled-member)
                    (jaunder--pull-media-apply-plan
                     (jaunder-pull-media-staged-plan media))))))))

(defun jaunder--reconcile-pull-install-staged (row staged remote path)
  "Install STAGED ROW bytes after successful REMOTE revalidation at PATH."
  (let ((preflight (jaunder--reconcile-pull-preflight row staged)))
    (if preflight
        (jaunder--reconcile-blocked row preflight)
      (let* ((staged (if (plist-get staged :media-staged)
                         (jaunder--reconcile-finalize-staged-media
                          (jaunder-reconcile-report-root jaunder-reconcile-report) staged)
                       staged))
             ;; Media fallback can take time and create durable files, so a
             ;; local-only uniqueness scan precedes the final replacement guard.
             (final-local (jaunder--reconcile-pull-final-local-unique-match row))
             (after-media (and (plist-get final-local :ok)
                               (jaunder--reconcile-pull-preflight row staged))))
        (cond
         ((not (plist-get final-local :ok))
          (jaunder--reconcile-blocked row (plist-get final-local :reason)
                                      (plist-get final-local :detail)))
         (after-media
          (jaunder--reconcile-blocked row after-media))
         (t
          (let* ((destination (jaunder--reconcile-pull-destination row (plist-get staged :slug)))
                 (bytes (if (plist-get staged :audience-omitted)
                            (jaunder--reconcile-preserve-legacy-audience
                             path (plist-get staged :bytes))
                          (plist-get staged :bytes)))
                 (installed (jaunder--reconcile-replace-pulled-file
                             path destination bytes (plist-get staged :synced-at)))
                 (committed (eq (plist-get installed :local-effect) 'replaced-at-old-path)))
            (append (list :outcome (if committed 'failed 'success)
                          :post-id (plist-get staged :id) :slug (plist-get staged :slug)
                          :etag (plist-get staged :etag) :synced-at (plist-get installed :synced-at)
                          :http-status (plist-get remote :http-status)
                          :local-effect (plist-get installed :local-effect))
                    (when committed
                      (list :reason 'pull-rename-failed :detail (plist-get installed :detail)))))))))))

(defun jaunder--reconcile-pull-server-ahead-row (row)
  "Stage, inventory, and revalidate ROW before one final local preflight.
The order is Member and Media staging, fresh unique-match inventory, final
remote strong-ETag revalidation, one local preflight, then replacement."
  (let* ((reviewed-etag (jaunder-reconcile-row-remote-etag row))
         (member (jaunder-reconcile-row-member row))
         (report jaunder-reconcile-report)
         (root (jaunder-reconcile-report-root report))
         (path (jaunder-inventory-local-path (jaunder-reconcile-row-local row))))
    ;; Pull staging switches buffers while preserving report-owned review evidence.
    (let ((jaunder-reconcile-report report))
      (if (not (jaunder--strong-etag-p reviewed-etag))
          (jaunder--reconcile-blocked row 'reviewed-etag-invalid)
        (condition-case err
            (let* ((jaunder--pull-link-inventory
                    (jaunder-reconcile-report-inventory jaunder-reconcile-report))
                   (jaunder--pull-original-proof (jaunder--reconcile-original-proof root row))
                   (staged (progn
                             (jaunder--reconcile-pull-progress "staging Member")
                             (jaunder--pull-stage-member root member)))
                   (inventory (progn
                                (jaunder--reconcile-pull-progress "verifying fresh Collection")
                                (jaunder--reconcile-pull-unique-match row))))
              (if (not (plist-get inventory :ok))
                  (jaunder--reconcile-blocked row (plist-get inventory :reason)
                                              (plist-get inventory :detail))
                (jaunder--reconcile-pull-progress "revalidating Member")
                (let ((remote (jaunder--reconcile-pull-remote-revalidation row reviewed-etag)))
                  (cond
                   ((not (plist-get remote :ok))
                    (append (jaunder--reconcile-blocked row (plist-get remote :reason)
                                                        (plist-get remote :detail))
                            (list :etag (plist-get remote :etag)
                                  :http-status (plist-get remote :http-status))))
                   ((not (equal reviewed-etag (plist-get staged :etag)))
                    (append (jaunder--reconcile-blocked row 'etag-stale)
                            (list :etag (plist-get remote :etag)
                                  :http-status (plist-get remote :http-status))))
                   (t (jaunder--reconcile-pull-progress "installing local Post")
                      (jaunder--reconcile-pull-install-staged row staged remote path))))))
          (jaunder-pull-stage-identity-changed
           (let ((evidence (car (cdr err))))
             (list :outcome 'blocked
                   :post-id (or (plist-get evidence :post-id)
                                (jaunder--reconcile-row-post-id row))
                   :slug (or (plist-get evidence :slug)
                             (jaunder--reconcile-row-slug row))
                   :etag (plist-get evidence :etag)
                   :http-status (plist-get evidence :http-status)
                   :local-effect 'unchanged :reason 'staged-identity-changed
                   :detail (plist-get evidence :detail))))
          (error (list :outcome 'failed :post-id (jaunder--reconcile-row-post-id row)
                       :slug (jaunder--reconcile-row-slug row) :etag reviewed-etag
                       :local-effect 'unchanged :reason 'pull-failed
                       :detail (jaunder--reconcile-pull-error-detail err))))))))

(defun jaunder--reconcile-pull-row (row)
  "Pull one explicitly selected ROW under the server-only and matched contracts."
  (jaunder--with-reconcile-row-debug "pull"
                                     (pcase (jaunder-reconcile-row-state row)
                                       ('unchanged (list :outcome 'no-op :post-id (jaunder--reconcile-row-post-id row)
                                                         :slug (jaunder--reconcile-row-slug row) :local-effect 'unchanged
                                                         :reason 'unchanged))
                                       ('server-only
                                        (jaunder--reconcile-pull-progress "staging Member")
                                        (condition-case err
                                            (let ((result (jaunder--pull-member
                                                           (jaunder-reconcile-report-root jaunder-reconcile-report)
                                                           (jaunder-reconcile-row-member row))))
                                              (list :outcome (if (eq (jaunder-pull-result-status result) 'pulled)
                                                                 'success 'blocked)
                                                    :post-id (or (jaunder-pull-result-id result)
                                                                 (jaunder--reconcile-row-post-id row))
                                                    :slug (or (jaunder-pull-result-slug result)
                                                              (jaunder--reconcile-row-slug row))
                                                    :etag (jaunder-pull-result-etag result)
                                                    :synced-at (jaunder-pull-result-synced-at result)
                                                    :http-status (jaunder-pull-result-http-status result)
                                                    :local-effect (or (jaunder-pull-result-local-effect result)
                                                                      (if (eq (jaunder-pull-result-status result) 'pulled)
                                                                          'created 'unchanged))
                                                    :reason (unless (eq (jaunder-pull-result-status result) 'pulled)
                                                              'pull-destination-occupied)))
                                          (error (list :outcome 'failed :post-id (jaunder--reconcile-row-post-id row)
                                                       :slug (jaunder--reconcile-row-slug row) :local-effect 'unchanged
                                                       :reason 'pull-failed :detail (jaunder--reconcile-pull-error-detail err)))))
                                       ('server-ahead (jaunder--reconcile-pull-server-ahead-row row))
                                       (_ (jaunder--reconcile-blocked row 'pull-ineligible
                                                                      (jaunder-reconcile-row-state row))))))

(defun jaunder--reconcile-confirm (prompt count &optional reviewed-etags)
  "Ask once with PROMPT, selected operation COUNT, and REVIEWED-ETAGS."
  (y-or-n-p (concat (format prompt count) reviewed-etags)))

(defun jaunder-reconcile-push-selected ()
  "Explicitly push selected safe rows after one preview confirmation."
  (interactive)
  (let ((rows (jaunder-reconcile-selected-rows))
        (buffer (current-buffer)))
    (unless rows (user-error "No reconciliation rows selected"))
    (when (jaunder--reconcile-confirm "Push %d selected Post(s)? " (length rows))
      (jaunder--call-with-blog
       (jaunder-reconcile-report-root jaunder-reconcile-report)
       (lambda ()
         (jaunder--reconcile-execute-batch buffer rows 'push
                                           #'jaunder--reconcile-push-row))))))

(defun jaunder-reconcile-pull-selected ()
  "Explicitly pull selected safe rows after one preview confirmation."
  (interactive)
  (let ((rows (jaunder-reconcile-selected-rows))
        (buffer (current-buffer)))
    (unless rows (user-error "No reconciliation rows selected"))
    (when (jaunder--reconcile-confirm "Pull %d selected Post(s)? " (length rows))
      (jaunder--call-with-blog
       (jaunder-reconcile-report-root jaunder-reconcile-report)
       (lambda ()
         (jaunder--reconcile-execute-batch buffer rows 'pull
                                           #'jaunder--reconcile-pull-row))))))

(defun jaunder--reconcile-keep-local-send (row path xml &optional merged-bytes audience-capable)
  "Send reviewed XML for ROW, then checkpoint PATH only on confirmed commit.
When MERGED-BYTES is present, atomically install that authored result before
server-confirmed metadata write-back.  AUDIENCE-CAPABLE is the service evidence
bound during preparation; never install a Post before the PUT."
  (let* ((etag (jaunder-reconcile-row-remote-etag row))
         (response
          (condition-case err
              (list :value (jaunder--send-reviewed-update
                            (jaunder-inventory-member-edit-uri
                             (jaunder-reconcile-row-member row)) etag xml))
            (error (list :error err))))
         (lost (plist-get response :error))
         (received (plist-get response :value))
         (status (plist-get received :status)))
    (cond
     (lost
      (if (eq (jaunder--operation-write-phase) 'confirmed)
          (append (jaunder--operation-write-failure lost)
                  (list :slug (jaunder--reconcile-row-slug row) :etag etag))
        (list :outcome 'unknown :post-id (jaunder--reconcile-row-post-id row)
              :slug (jaunder--reconcile-row-slug row) :etag etag
              :local-effect 'unchanged :reason 'remote-outcome-unknown
              :detail (format "PUT response lost; reconcile before retrying: %s"
                              (error-message-string lost)))))
     ((eq status 412)
      (append (jaunder--reconcile-blocked row 'etag-stale)
              (list :etag etag :http-status status)))
     ((eq (jaunder--operation-write-phase) 'unknown)
      (list :outcome 'unknown :post-id (jaunder--reconcile-row-post-id row)
            :slug (jaunder--reconcile-row-slug row) :etag etag
            :http-status status :local-effect 'unchanged :reason 'remote-outcome-unknown
            :detail "PUT outcome unknown; reconcile before retrying"))
     ((not (memq status '(200 201)))
      (list :outcome 'failed :post-id (jaunder--reconcile-row-post-id row)
            :slug (jaunder--reconcile-row-slug row) :etag etag
            :http-status status :local-effect 'unchanged :reason 'publish-http-error))
     (t
      (let ((drift (jaunder--reconcile-pull-preflight
                    row (list :slug (jaunder--reconcile-row-slug row)))))
        (if drift
            (list :outcome 'partial :post-id (jaunder--reconcile-row-post-id row)
                  :slug (jaunder--reconcile-row-slug row)
                  :etag (jaunder--response-header received "ETag")
                  :http-status status :local-effect 'unchanged
                  :reason 'local-changed-after-commit
                  :detail (format "Remote PUT committed; %s; reconcile before retrying" drift))
          (let ((checkpoint
                 (condition-case err
                     (progn
                       (when merged-bytes
                         (jaunder--reconcile-replace-pulled-file
                          path path merged-bytes))
                       (list :value
                             (jaunder--reconcile-call-with-source-buffer
                              path
                              (lambda ()
                                ;; Record timezone only after confirmed commitment.
                                (jaunder--ensure-date-tz)
                                (let ((slug (jaunder--write-back
                                             received nil nil t audience-capable)))
                                  (list :slug slug :path (jaunder--rename-to-slug slug)))))))
                   (error (list :error err)))))
            (if (plist-get checkpoint :error)
                (list :outcome 'partial :post-id (jaunder--reconcile-row-post-id row)
                      :slug (jaunder--reconcile-row-slug row)
                      :etag (jaunder--response-header received "ETag")
                      :http-status status :local-effect 'checkpoint-uncertain
                      :reason 'local-write-back-failed
                      :detail (format "Remote PUT committed; inspect local Post before retrying: %s"
                                      (error-message-string (plist-get checkpoint :error))))
              (let ((installed (plist-get checkpoint :value)))
                (list :outcome 'success :post-id (jaunder--reconcile-row-post-id row)
                      :slug (plist-get installed :slug)
                      :etag (jaunder--response-header received "ETag")
                      :http-status status
                      :local-effect (if (equal path (plist-get installed :path))
                                        'updated 'renamed)))))))))))

(defun jaunder--reconcile-keep-local-row (row)
  "Publish reviewed ROW locally authored content without any pre-PUT Post write."
  (jaunder--with-reconcile-row-debug "keep-local"
                                     (let ((initial (jaunder--reconcile-conflict-preflight row)))
                                       (if (not (plist-get initial :ok))
                                           initial
                                         (let* ((path (jaunder-inventory-local-path (jaunder-reconcile-row-local row)))
                                                (prepared
                                                 (condition-case err
                                                     (jaunder--reconcile-call-with-source-buffer
                                                      path #'jaunder--prepare-reviewed-update)
                                                   (error (list :error err)))))
                                           (if (plist-get prepared :error)
                                               (list :outcome 'failed :post-id (jaunder--reconcile-row-post-id row)
                                                     :slug (jaunder--reconcile-row-slug row) :local-effect 'unchanged
                                                     :reason 'publish-preparation-failed
                                                     :detail (error-message-string (plist-get prepared :error)))
                                             (let ((final (jaunder--reconcile-conflict-preflight row)))
                                               (if (not (plist-get final :ok))
                                                   final
                                                 (jaunder--reconcile-keep-local-send
                                                  row path (plist-get prepared :xml) nil
                                                  (plist-get prepared :audience-capable))))))))))

(defun jaunder-reconcile-keep-local-selected ()
  "Publish reviewed local content for selected conflicts after confirmation."
  (interactive)
  (jaunder--with-debug-operation "conflict.local" nil
                                 (let ((rows (jaunder-reconcile-selected-rows))
                                       (buffer (current-buffer)))
                                   (unless rows (user-error "No reconciliation rows selected"))
                                   (when (jaunder--reconcile-confirm
                                          "Keep local for %d selected Post(s)? " (length rows))
                                     (jaunder--call-with-blog
                                      (jaunder-reconcile-report-root jaunder-reconcile-report)
                                      (lambda ()
                                        (jaunder--reconcile-execute-batch
                                         buffer rows 'keep-local #'jaunder--reconcile-keep-local-row)))))))

(defun jaunder--reconcile-merge-scratch-name (row root)
  "Name the independent scratch for ROW and ROOT without cross-blog collision."
  (format "*Jaunder Merge Result %s %s*"
          (jaunder--reconcile-row-post-id row)
          (substring (secure-hash 'sha256 root) 0 8)))

(defun jaunder--reconcile-stage-matches-review-p (row staged)
  "Return non-nil when STAGED Member bytes and identity match reviewed ROW."
  (let ((member (jaunder-reconcile-row-member row)))
    (and (equal (plist-get staged :id) (jaunder-inventory-member-id member))
         (equal (plist-get staged :slug) (jaunder-inventory-member-slug member))
         (equal (plist-get staged :etag) (jaunder-reconcile-row-remote-etag row))
         (stringp (plist-get staged :bytes)))))

(defun jaunder--reconcile-merge-stage (row)
  "Return reviewed staged remote data for ROW, or a terminal blocked result.
Do not create an editable result until both before/after-staging guards pass."
  (jaunder--with-debug-operation "merge.stage" nil
				 (let ((initial (jaunder--reconcile-conflict-preflight row)))
				   (if (not (plist-get initial :ok))
				       initial
				     (condition-case err
					 (let* ((root (jaunder-reconcile-report-root jaunder-reconcile-report))
						(member (jaunder-reconcile-row-member row))
						(jaunder--pull-link-inventory
						 (jaunder-reconcile-report-inventory jaunder-reconcile-report))
						(jaunder--pull-original-proof (jaunder--reconcile-original-proof root row))
						(staged (jaunder--pull-stage-member root member)))
					   (cond
					    ((not (jaunder--reconcile-stage-matches-review-p row staged))
					     (jaunder--reconcile-blocked row 'staged-identity-changed))
					    (t
					     (setq staged (jaunder--reconcile-finalize-staged-media root staged))
					     (let ((final (jaunder--reconcile-conflict-preflight row)))
					       (if (plist-get final :ok)
						   (list :staged staged)
						 final)))))
				       (jaunder-pull-stage-identity-changed
					(jaunder--reconcile-blocked row 'staged-identity-changed))
				       (error
					(list :outcome 'failed :post-id (jaunder--reconcile-row-post-id row)
					      :slug (jaunder--reconcile-row-slug row) :local-effect 'unchanged
					      :reason 'merge-staging-failed :detail (error-message-string err))))))))

(defun jaunder--reconcile-merge-record (session-or-row report-buffer value)
  "Record VALUE for SESSION-OR-ROW in REPORT-BUFFER and refresh the report."
  (let ((row (if (jaunder-reconcile-merge-session-p session-or-row)
                 (jaunder-reconcile-merge-session-row session-or-row)
               session-or-row)))
    (with-current-buffer report-buffer
      (setq-local jaunder-reconcile-last-batch-results
                  (list (jaunder--reconcile-terminal-result 'merge row value)))
      (jaunder--reconcile-show-results-and-refresh report-buffer))))

(defun jaunder--reconcile-merge-snapshot (name bytes)
  "Create a read-only Org snapshot named NAME of reviewed literal BYTES."
  (let ((buffer (generate-new-buffer name))
        complete)
    (unwind-protect
        (progn
          (with-current-buffer buffer
            (org-mode)
            (insert bytes)
            (set-buffer-modified-p nil)
            (setq buffer-read-only t))
          (setq complete t)
          buffer)
      (unless complete (when (buffer-live-p buffer) (kill-buffer buffer))))))

(defconst jaunder--reconcile-merge-owned-properties
  '("JAUNDER_ID" "JAUNDER_SLUG" "JAUNDER_SYNCED" "JAUNDER_SYNCED_AT"
    "JAUNDER_LOCAL_AHEAD" "JAUNDER_DATE_TZ" "JAUNDER_DATE_UTC"
    "JAUNDER_FORMAT" "JAUNDER_CREATE_KEY" "JAUNDER_CREATE_DIGEST"
    "JAUNDER_CREATE_ATTEMPT_AT")
  "Client-managed local Post metadata never taken from an editable merge scratch.")

(defun jaunder--reconcile-merge-authorized-bytes (session)
  "Return SESSION scratch with authored fields and reviewed local owned metadata.
Strip every client-managed JAUNDER property from the editable leading header,
then restore the supported keys from the reviewed local Post.  In particular,
editing an identity or sync marker in scratch cannot change the sent Post."
  (let ((owned
         (with-temp-buffer
           (insert-file-contents-literally (jaunder-reconcile-merge-session-path session))
           (org-mode)
           (mapcar (lambda (key) (cons key (jaunder--buffer-property key)))
                   jaunder--reconcile-merge-owned-properties))))
    (with-temp-buffer
      (insert (with-current-buffer (jaunder-reconcile-merge-session-scratch session)
                (buffer-substring-no-properties (point-min) (point-max))))
      (org-mode)
      (let ((case-fold-search t))
        (goto-char (point-min))
        (while (re-search-forward
                "^[ \t]*#\\+PROPERTY:[ \t]+\\(JAUNDER_[[:alnum:]_]+\\)\\(?:[ \t].*\\)?\\(?:\n\\|\\'\\)"
                (jaunder--body-start) t)
          (unless (member (upcase (match-string 1))
                          '("JAUNDER_STATUS" "JAUNDER_AUDIENCE"))
            (replace-match ""))))
      (dolist (property owned)
        (when (cdr property)
          (jaunder--set-property (car property) (cdr property))))
      (buffer-string))))

(defun jaunder--reconcile-merge-prepared-update (path bytes)
  "Prepare authorized Org BYTES and service evidence without writing PATH."
  (let ((default-directory (file-name-directory path)))
    (with-temp-buffer
      (insert bytes)
      (org-mode)
      ;; Only the transient preparation buffer sees PATH for relative Media
      ;; and Local Post Link resolution.  It never visits or saves PATH.
      (setq-local buffer-file-name path)
      (jaunder--prepare-reviewed-update))))

(defun jaunder--reconcile-merge-close-views (session)
  "Release SESSION's read-only Ediff snapshots after Ediff has finished."
  (setf (jaunder-reconcile-merge-session-ediff-active session) nil)
  (dolist (buffer (list (jaunder-reconcile-merge-session-local-view session)
                        (jaunder-reconcile-merge-session-remote-view session)))
    (when (buffer-live-p buffer) (kill-buffer buffer))))

(defun jaunder--reconcile-merge-cleanup (session)
  "Retire SESSION scratch, leaving any active Ediff snapshots until quit."
  (unless (jaunder-reconcile-merge-session-ediff-active session)
    (jaunder--reconcile-merge-close-views session))
  (let ((scratch (jaunder-reconcile-merge-session-scratch session)))
    (when (buffer-live-p scratch)
      (with-current-buffer scratch
        (setq-local jaunder-reconcile-merge-allow-kill t)
        (set-buffer-modified-p nil))
      (kill-buffer scratch))))

(defun jaunder-reconcile-merge-discard ()
  "Discard this merge scratch only after an explicit User confirmation."
  (interactive)
  (jaunder--with-debug-operation "merge.discard" nil
                                 (let ((session jaunder-reconcile-merge-session))
                                   (unless session
                                     (user-error "This buffer is not a reconciliation merge result"))
                                   (when (y-or-n-p "Discard the editable merge result permanently? ")
                                     (jaunder--reconcile-merge-cleanup session)))))

(defun jaunder-reconcile-merge-finish ()
  "Explicitly publish this scratch after revalidating the reviewed conflict.
Never infer approval from exiting Ediff.  Preserve scratch after a blocked,
unknown, failed, or partial result; only confirmed success retires it."
  (interactive)
  (jaunder--with-debug-operation "merge.finish" nil
                                 (let* ((session jaunder-reconcile-merge-session)
                                        (row (and session (jaunder-reconcile-merge-session-row session)))
                                        (report-buffer (and session (jaunder-reconcile-merge-session-report-buffer session))))
                                   (unless (and row (buffer-live-p report-buffer))
                                     (user-error "This merge has no live reconciliation report; retain its scratch"))
                                   (unless (jaunder-reconcile-merge-session-ediff-ready session)
                                     (user-error "Ediff did not open; this scratch cannot be published"))
                                   (when (y-or-n-p
                                          (format "Publish merged Post %s against reviewed ETag %s? "
                                                  (jaunder--reconcile-row-post-id row)
                                                  (jaunder-reconcile-row-remote-etag row)))
                                     (let* ((path (jaunder-reconcile-merge-session-path session))
                                            (reviewed-text (buffer-substring-no-properties (point-min) (point-max)))
                                            (result
                                             (with-current-buffer report-buffer
                                               (jaunder--call-with-blog
                                                (jaunder-reconcile-report-root jaunder-reconcile-report)
                                                (lambda ()
                                                  (let ((initial (jaunder--reconcile-conflict-preflight row)))
                                                    (if (not (plist-get initial :ok))
                                                        initial
                                                      (let ((prepared
                                                             (condition-case err
                                                                 (let* ((bytes (jaunder--reconcile-merge-authorized-bytes
                                                                                session))
                                                                        (update (jaunder--reconcile-merge-prepared-update
                                                                                 path bytes)))
                                                                   (list :bytes bytes :xml (plist-get update :xml)
                                                                         :audience-capable
                                                                         (plist-get update :audience-capable)))
                                                               (error (list :error err)))))
                                                        (if (plist-get prepared :error)
                                                            (list :outcome 'failed
                                                                  :post-id (jaunder--reconcile-row-post-id row)
                                                                  :slug (jaunder--reconcile-row-slug row)
                                                                  :local-effect 'unchanged
                                                                  :reason 'merge-preparation-failed
                                                                  :detail (error-message-string
                                                                           (plist-get prepared :error)))
                                                          (let ((final (jaunder--reconcile-conflict-preflight row)))
                                                            (if (not (plist-get final :ok))
                                                                final
                                                              (jaunder--reconcile-keep-local-send
                                                               row path (plist-get prepared :xml)
                                                               (plist-get prepared :bytes)
                                                               (plist-get prepared :audience-capable)))))))))))))
                                       (setq-local jaunder-reconcile-merge-last-result result)
                                       (jaunder--reconcile-merge-record session report-buffer result)
                                       (if (eq (plist-get result :outcome) 'success)
                                           (if (equal reviewed-text (buffer-substring-no-properties
                                                                     (point-min) (point-max)))
                                               (jaunder--reconcile-merge-cleanup session)
                                             (message "Remote merge committed; scratch changed during send and was retained"))
                                         (message "Merge %s: %s; scratch retained for fresh review"
                                                  (plist-get result :outcome) (plist-get result :reason)))
                                       result)))))

(defun jaunder-reconcile-merge-cancel ()
  "Retain the editable scratch; leaving Ediff never publishes it."
  (interactive)
  (jaunder--with-debug-operation "merge.cancel" nil
                                 (unless jaunder-reconcile-merge-session
                                   (user-error "This buffer is not a reconciliation merge result"))
                                 (message (if (jaunder-reconcile-merge-session-ediff-ready
                                               jaunder-reconcile-merge-session)
                                              "Merge cancelled; %s retained for later completion or explicit discard"
                                            "Ediff failed; %s retained for inspection or explicit discard; start a fresh merge")
                                          (buffer-name))))

(defun jaunder--reconcile-merge-bind-ediff-result (session name)
  "Make Ediff's actual merge output the editable scratch for SESSION.
Called from Ediff's control-buffer startup hook after its two-way merge output
is initialized.  Ediff's copy commands and direct Org edits then write the
same buffer; no local Post file is associated with that buffer."
  (let ((result ediff-buffer-C)
        complete)
    (unless (and (buffer-live-p result) (not (buffer-file-name result)))
      (error "Ediff did not provide a non-file-visiting merge output"))
    (unwind-protect
        (progn
          (with-current-buffer result
            (rename-buffer name)
            (jaunder-reconcile-merge-mode)
            (setq-local jaunder-reconcile-merge-session session))
          (setf (jaunder-reconcile-merge-session-scratch session) result)
          ;; Never let a user-wide Ediff autostore preference save or retire
          ;; this result on quit; only explicit finish may publish.
          (setq-local ediff-autostore-merges nil)
          (add-hook 'ediff-quit-hook
                    (lambda () (jaunder--reconcile-merge-close-views session)) nil t)
          (setq complete t))
      (unless complete
        (setf (jaunder-reconcile-merge-session-scratch session) nil)
        (when (buffer-live-p result)
          (with-current-buffer result
            (setq-local jaunder-reconcile-merge-allow-kill t)
            (set-buffer-modified-p nil))
          (kill-buffer result))))))

(defun jaunder-reconcile-merge-selected ()
  "Stage the reviewed conflict at point and open a two-way Ediff merge.
Marks and the active region do not affect which Post is merged.
Ediff's actual merge output is the independent editable Org scratch; copy
operations write there, never to either Post.  `C-c C-c' explicitly finishes."
  (interactive)
  (jaunder--with-debug-operation "conflict.merge" nil
                                 (let* ((row (get-text-property (point) 'jaunder-reconcile-row))
                                        (report-buffer (current-buffer))
                                        (root (jaunder-reconcile-report-root jaunder-reconcile-report)))
                                   (unless row
                                     (user-error "Move point to a conflict row for Ediff"))
                                   (let ((name (jaunder--reconcile-merge-scratch-name row root)))
                                     (when (get-buffer name)
                                       (user-error "Merge scratch %s already exists; finish or discard it first" name))
                                     (jaunder--call-with-blog
                                      root
                                      (lambda ()
                                        (let ((review (jaunder--reconcile-merge-stage row)))
                                          (if (plist-get review :outcome)
                                              (progn
                                                (jaunder--reconcile-merge-record row report-buffer review)
                                                nil)
                                            (let* ((path (jaunder-inventory-local-path
                                                          (jaunder-reconcile-row-local row)))
                                                   (session (jaunder--make-reconcile-merge-session
                                                             :row row :report-buffer report-buffer :path path))
                                                   (setup-error
                                                    (condition-case err
                                                        (jaunder--with-debug-operation "merge.stage" nil
                                                                                       (progn
                                                                                         (setf (jaunder-reconcile-merge-session-local-view session)
                                                                                               (jaunder--reconcile-merge-snapshot
                                                                                                " *Jaunder Merge Local*"
                                                                                                (with-temp-buffer
                                                                                                  (insert-file-contents-literally path)
                                                                                                  (buffer-string))))
                                                                                         (setf (jaunder-reconcile-merge-session-remote-view session)
                                                                                               (jaunder--reconcile-merge-snapshot
                                                                                                " *Jaunder Merge Remote*"
                                                                                                (plist-get (plist-get review :staged) :bytes)))
                                                                                         (setf (jaunder-reconcile-merge-session-ediff-active session) t)
                                                                                         (ediff-merge-buffers
                                                                                          (jaunder-reconcile-merge-session-local-view session)
                                                                                          (jaunder-reconcile-merge-session-remote-view session)
                                                                                          (list (lambda ()
                                                                                                  (jaunder--reconcile-merge-bind-ediff-result
                                                                                                   session name))))
                                                                                         (unless (buffer-live-p
                                                                                                  (jaunder-reconcile-merge-session-scratch session))
                                                                                           (error "Ediff did not expose its merge output"))
                                                                                         (setf (jaunder-reconcile-merge-session-ediff-ready session) t)
                                                                                         nil))
                                                      (error
                                                       (setf (jaunder-reconcile-merge-session-ediff-active session) nil)
                                                       err))))
                                              (if setup-error
                                                  (let ((result (list :outcome 'failed
                                                                      :post-id (jaunder--reconcile-row-post-id row)
                                                                      :slug (jaunder--reconcile-row-slug row)
                                                                      :local-effect 'unchanged
                                                                      :reason 'ediff-unavailable
                                                                      :detail (error-message-string setup-error))))
                                                    (jaunder--reconcile-merge-close-views session)
                                                    (jaunder--reconcile-merge-record session report-buffer result)
                                                    ;; If Ediff had already created C, retain that work for
                                                    ;; inspection but never allow a failed session to publish.
                                                    (when (buffer-live-p (jaunder-reconcile-merge-session-scratch session))
                                                      (display-buffer (jaunder-reconcile-merge-session-scratch session)))
                                                    (message "Merge could not open: %s" (error-message-string setup-error))
                                                    nil)
                                                (display-buffer (jaunder-reconcile-merge-session-scratch session))
                                                (message "Two-way Ediff merge opened; edit %s, then C-c C-c to complete"
                                                         name)
                                                (jaunder-reconcile-merge-session-scratch session)))))))))))

(defun jaunder--reconcile-keep-remote-row (row)
  "Install ROW's reviewed remote Post, or return a structured blocked result."
  (jaunder--with-reconcile-row-debug "keep-remote"
				     (let ((initial (jaunder--reconcile-conflict-preflight row)))
				       (if (not (plist-get initial :ok))
					   initial
					 (condition-case err
					     (let* ((root (jaunder-reconcile-report-root jaunder-reconcile-report))
						    (member (jaunder-reconcile-row-member row))
						    (path (jaunder-inventory-local-path (jaunder-reconcile-row-local row)))
						    (jaunder--pull-link-inventory
						     (jaunder-reconcile-report-inventory jaunder-reconcile-report))
						    (jaunder--pull-original-proof (jaunder--reconcile-original-proof root row))
						    (staged (jaunder--pull-stage-member root member)))
					       (if (not (jaunder--reconcile-stage-matches-review-p row staged))
						   (jaunder--reconcile-blocked row 'staged-identity-changed)
						 (let ((final (jaunder--reconcile-conflict-preflight row)))
						   (if (not (plist-get final :ok))
						       final
						     (let ((installed (jaunder--reconcile-pull-install-staged
								       row staged final path)))
						       (when (and (eq (plist-get installed :outcome) 'failed)
								  (eq (plist-get installed :local-effect)
								      'replaced-at-old-path))
							 ;; The Post was atomically replaced; only its canonical
							 ;; rename failed.  The old path remains recoverable.
							 (setq installed (plist-put installed :outcome 'partial)))
						       installed)))))
					   (jaunder-pull-stage-identity-changed
					    (jaunder--reconcile-blocked row 'staged-identity-changed))
					   (error (list :outcome 'failed :post-id (jaunder--reconcile-row-post-id row)
							:slug (jaunder--reconcile-row-slug row) :local-effect 'unchanged
							:reason 'pull-failed :detail (error-message-string err))))))))

(defun jaunder-reconcile-keep-remote-selected ()
  "Accept the reviewed remote Post for selected conflicts after confirmation."
  (interactive)
  (jaunder--with-debug-operation "conflict.remote" nil
                                 (let ((rows (jaunder-reconcile-selected-rows))
                                       (buffer (current-buffer)))
                                   (unless rows (user-error "No reconciliation rows selected"))
                                   (when (jaunder--reconcile-confirm
                                          "Keep remote for %d selected Post(s)? " (length rows))
                                     (jaunder--call-with-blog
                                      (jaunder-reconcile-report-root jaunder-reconcile-report)
                                      (lambda ()
                                        (jaunder--reconcile-execute-batch
                                         buffer rows 'keep-remote #'jaunder--reconcile-keep-remote-row)))))))

(defun jaunder-reconcile-delete-selected ()
  "Explicitly soft-delete selected remote Posts after reviewing fresh ETags."
  (interactive)
  (let* ((rows (jaunder-reconcile-selected-rows))
         (buffer (current-buffer))
         (reviews (make-hash-table :test #'equal)))
    (unless rows (user-error "No reconciliation rows selected"))
    (jaunder--call-with-blog
     (jaunder-reconcile-report-root jaunder-reconcile-report)
     (lambda ()
       (dolist (row rows)
         (puthash (jaunder--reconcile-stable-row-key row)
                  (if (memq (jaunder-reconcile-row-state row)
                            '(server-only unchanged local-ahead server-ahead))
                      (let ((preflight (jaunder--reconcile-delete-preflight row)))
                        (if preflight
                            (jaunder--reconcile-blocked row preflight)
                          (jaunder--reconcile-delete-etag row)))
                    (jaunder--reconcile-blocked row 'delete-ineligible
                                                (jaunder-reconcile-row-state row)))
                  reviews))
       (when (jaunder--reconcile-confirm
              "SOFT-DELETE %d selected remote Post(s)? This retains server tombstones. "
              (length rows)
              (mapconcat
               (lambda (row)
                 (let ((review (gethash (jaunder--reconcile-stable-row-key row) reviews)))
                   (format "%s ETag=%s"
                           (or (plist-get review :post-id) "unavailable")
                           (or (plist-get review :etag) "unavailable"))))
               rows "; "))
         (jaunder--reconcile-execute-batch
          buffer rows 'delete
          (lambda (row)
            (jaunder--reconcile-delete-row
             row (gethash (jaunder--reconcile-stable-row-key row) reviews)))))))))

(defun jaunder-reconcile (root)
  "Reconcile ROOT with its configured AtomPub Collection without resolving it."
  (interactive (list default-directory))
  (jaunder--with-debug-operation "report.open" nil
                                 (jaunder--reconcile-with-progress
                                  "report ready" "report failed"
                                  (lambda ()
                                    (jaunder--call-with-blog
                                     root
                                     (lambda ()
                                       (let* ((configured-root (car (jaunder--blog-entry-for root)))
                                              (inventory (jaunder--inventory-for-root configured-root))
                                              (report (jaunder--reconcile-build-report configured-root inventory))
                                              (buffer (jaunder--render-reconcile-report report)))
                                         (with-current-buffer buffer
                                           (setq-local jaunder-reconcile-last-batch-results nil)
                                           (setq-local jaunder-reconcile-marks (make-hash-table :test #'equal))
                                           (jaunder--render-reconcile-report report buffer))
                                         (display-buffer buffer)
                                         report)))))))



(provide 'jaunder-reconcile)
;;; jaunder-reconcile.el ends here
