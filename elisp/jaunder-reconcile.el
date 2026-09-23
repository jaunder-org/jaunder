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
(require 'jaunder-publish)

(declare-function jaunder--pull-destination "jaunder-pull")
(declare-function jaunder--pull-destination-exists-p "jaunder-pull")
(declare-function jaunder--pull-member "jaunder-pull")
(declare-function jaunder--pull-stage-member "jaunder-pull")
(declare-function jaunder--pull-response-identity "jaunder-pull")
(declare-function jaunder-pull-result-status "jaunder-pull")
(declare-function jaunder-pull-result-id "jaunder-pull")
(declare-function jaunder-pull-result-slug "jaunder-pull")
(declare-function jaunder-pull-result-etag "jaunder-pull")
(declare-function jaunder-pull-result-synced-at "jaunder-pull")
(declare-function jaunder-pull-result-http-status "jaunder-pull")
(declare-function jaunder-pull-result-local-effect "jaunder-pull")

(defvar jaunder--pull-link-inventory)

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
  "Major mode for selecting rows in a Jaunder reconciliation report."
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
         (signal (car err) (cdr err)))))))

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
      (jaunder--reconcile-refresh-buffer buffer)
    (error
     (message "Jaunder reconcile: refresh failed; Last batch retained: %s"
              (error-message-string err))
     'refresh-failed)))

(defun jaunder--reconcile-execute-batch (buffer rows action operation &optional cancelled-p)
  "Run OPERATION for ROWS sequentially, retaining every terminal result in BUFFER.
CANCELLED-P is checked only between completed items.  OPERATION receives one
row and returns a result plist; its independent errors become failed results."
  (with-current-buffer buffer
    (setq-local jaunder-reconcile-last-batch-results nil)
    (setq rows (cl-remove-if-not
                (lambda (displayed-row)
                  (memq displayed-row rows))
                (jaunder--reconcile-displayed-rows jaunder-reconcile-report))))
  (let ((total (length rows)) (completed 0) cancelled)
    (dolist (row rows)
      (unless cancelled
        (if (or (and cancelled-p (funcall cancelled-p)) quit-flag)
            (setq cancelled t)
          (setq completed (1+ completed))
          (message "Jaunder %s: %d/%d" action completed total)
          (let (quit-requested value)
            (setq value
                  (condition-case err
                      (let ((inhibit-quit t))
                        (prog1 (funcall operation row)
                          (setq quit-requested quit-flag
                                quit-flag nil)))
                    (error (list :outcome 'failed :reason 'operation-failed
                                 :detail (error-message-string err)))))
            (with-current-buffer buffer
              (setq-local jaunder-reconcile-last-batch-results
                          (append jaunder-reconcile-last-batch-results
                                  (list (jaunder--reconcile-terminal-result action row value)))))
            (when (or quit-requested
                      (and cancelled-p (funcall cancelled-p)) quit-flag)
              (setq cancelled t))))))
    (when cancelled (setq quit-flag nil))
    (let ((refresh (jaunder--reconcile-show-results-and-refresh buffer)))
      (if (eq refresh 'refresh-failed)
          'refresh-failed
        (if cancelled 'cancelled 'completed)))))

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
              (list :outcome 'success
                    :post-id (jaunder--buffer-property "JAUNDER_ID")
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
                                   (jaunder-reconcile-row-state row)))))

(defun jaunder--reconcile-delete-etag (row)
  "Fetch ROW's current strong ETag for explicit remote deletion.
Return a plist suitable for a terminal result; no DELETE is sent here."
  (let* ((member (jaunder-reconcile-row-member row))
         (id (jaunder--reconcile-row-post-id row))
         (slug (jaunder--reconcile-row-slug row)))
    (condition-case err
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
                              'current-etag-invalid 'member-http-error))))
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
  (if (plist-get reviewed :outcome)
      reviewed
    (let ((id (plist-get reviewed :post-id))
          (slug (plist-get reviewed :slug))
          (etag (plist-get reviewed :etag)))
      (let ((preflight (jaunder--reconcile-delete-preflight row)))
        (if preflight
            (jaunder--reconcile-blocked row preflight)
          (condition-case err
              (let* ((response (jaunder--http-request
                                "DELETE" (jaunder--member-url id) nil nil
                                (list (cons "If-Match" etag))) )
                     (status (plist-get response :status)))
                (if (eq status 204)
                    (let ((local (jaunder--reconcile-delete-local-file row)))
                      (list :outcome 'success :post-id id :slug slug :etag etag
                            :http-status status
                            :local-effect (plist-get local :local-effect)
                            :reason (plist-get local :reason)))
                  (list :outcome 'failed :post-id id :slug slug :etag etag
                        :http-status status :local-effect 'unchanged
                        :reason (if (eq status 412) 'etag-stale 'delete-http-error))))
            (error (list :outcome 'failed :post-id id :slug slug :etag etag
                         :local-effect 'unchanged :reason 'delete-transport-error
                         :detail (error-message-string err)))))))))

(defun jaunder--reconcile-pull-destination (_row slug)
  "Return the report root's canonical direct-root destination for server SLUG."
  (jaunder--pull-destination
   (jaunder-reconcile-report-root jaunder-reconcile-report) slug))

(defun jaunder--reconcile-pull-preflight (row staged)
  "Return a blocking reason unless ROW remains safe for STAGED replacement."
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
      'pull-destination-occupied))))

(defun jaunder--reconcile-pull-remote-revalidation (row etag)
  "Return structured current-ETag evidence for ROW against reviewed ETAG."
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
                 :detail (error-message-string err)))))

(defun jaunder--reconcile-pull-unique-match (row)
  "Return structured fresh-inventory evidence that ROW remains uniquely matched.
A duplicate local or remote Post ID is an actionable blocked result, rather
than an exception flattened into a generic pull failure."
  (let* ((root (jaunder-reconcile-report-root jaunder-reconcile-report))
         (id (jaunder--reconcile-row-post-id row))
         (path (jaunder-inventory-local-path (jaunder-reconcile-row-local row))))
    (condition-case err
        (let* ((inventory (jaunder--inventory-for-root root))
               (duplicate-local
                (cl-find-if
                 (lambda (conflict)
                   (and (memq 'duplicate-local-id
                              (jaunder-inventory-conflict-kinds conflict))
                        (cl-some (lambda (local)
                                   (and (equal (jaunder-inventory-local-id local) id)
                                        (equal (jaunder-inventory-local-path local) path)))
                                 (jaunder-inventory-conflict-locals conflict))))
                 (jaunder-inventory-conflicts inventory)))
               (matches (cl-remove-if-not
                         (lambda (match)
                           (and (equal (jaunder-inventory-local-path
                                        (jaunder-inventory-match-local match)) path)
                                (equal (jaunder-inventory-local-id
                                        (jaunder-inventory-match-local match)) id)
                                (equal (jaunder-inventory-member-id
                                        (jaunder-inventory-match-member match)) id)))
                         (jaunder-inventory-matched inventory)))
               (members (cl-remove-if-not
                         (lambda (member)
                           (equal (jaunder-inventory-member-id member) id))
                         (append (jaunder-inventory-server-only inventory)
                                 (mapcar #'jaunder-inventory-match-member
                                         (jaunder-inventory-matched inventory))))))
          (cond (duplicate-local
                 (list :reason 'duplicate-local-id
                       :detail (format "fresh inventory has duplicate local Post ID %s" id)))
                ((and (= (length matches) 1) (= (length members) 1))
                 (list :ok t))
                (t (list :reason 'matched-identity-changed
                         :detail "fresh inventory no longer has the reviewed unique match"))))
      (jaunder-inventory-duplicate-remote-id
       (list :reason 'duplicate-remote-id
             :detail (format "fresh inventory has duplicate remote Post ID %s" id)))
      (error (list :reason 'fresh-inventory-failed
                   :detail (error-message-string err))))))

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

(defun jaunder--reconcile-replace-pulled-file (path destination bytes)
  "Atomically replace PATH then rename to DESTINATION, reporting committed state."
  (let ((temporary nil))
    (unwind-protect
        (progn
          (setq temporary (make-temp-file
                           (expand-file-name ".jaunder-pull-" (file-name-directory path))))
          (let ((coding-system-for-write 'utf-8-unix))
            (write-region bytes nil temporary nil 'silent))
          (rename-file temporary path t)
          (setq temporary nil)
          (let ((buffer (get-file-buffer path)))
            (when (buffer-live-p buffer)
              (with-current-buffer buffer (revert-buffer t t) (set-buffer-modified-p nil))))
          (if (equal path destination)
              (list :path path :local-effect 'replaced)
            (condition-case err
                (progn
                  (rename-file path destination nil)
                  (let ((buffer (get-file-buffer path)))
                    (when (buffer-live-p buffer)
                      (with-current-buffer buffer
                        (set-visited-file-name destination t t)
                        (set-buffer-modified-p nil))))
                  (list :path destination :local-effect 'renamed))
              (error (list :path path :local-effect 'replaced-at-old-path
                           :detail (error-message-string err))))))
      (when (and temporary (file-exists-p temporary)) (delete-file temporary)))))

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

(defun jaunder--reconcile-pull-install-staged (row staged remote path)
  "Install STAGED ROW bytes after successful REMOTE revalidation at PATH."
  (let ((preflight (jaunder--reconcile-pull-preflight row staged)))
    (if preflight
        (jaunder--reconcile-blocked row preflight)
      (let* ((destination (jaunder--reconcile-pull-destination row (plist-get staged :slug)))
             (bytes (if (plist-get staged :audience-omitted)
                        (jaunder--reconcile-preserve-legacy-audience
                         path (plist-get staged :bytes))
                      (plist-get staged :bytes)))
             (installed (jaunder--reconcile-replace-pulled-file
                         path destination bytes))
             (committed (eq (plist-get installed :local-effect) 'replaced-at-old-path)))
        (append (list :outcome (if committed 'failed 'success)
                      :post-id (plist-get staged :id) :slug (plist-get staged :slug)
                      :etag (plist-get staged :etag) :synced-at (plist-get staged :synced-at)
                      :http-status (plist-get remote :http-status)
                      :local-effect (plist-get installed :local-effect))
                (when committed
                  (list :reason 'pull-rename-failed :detail (plist-get installed :detail))))))))

(defun jaunder--reconcile-pull-server-ahead-row (row)
  "Stage, inventory, and revalidate ROW before one final local preflight.
The order is Member and Media staging, fresh unique-match inventory, final
remote strong-ETag revalidation, one local preflight, then replacement."
  (let* ((reviewed-etag (jaunder-reconcile-row-remote-etag row))
         (member (jaunder-reconcile-row-member row))
         (path (jaunder-inventory-local-path (jaunder-reconcile-row-local row))))
    (if (not (jaunder--strong-etag-p reviewed-etag))
        (jaunder--reconcile-blocked row 'reviewed-etag-invalid)
      (condition-case err
          (let* ((jaunder--pull-link-inventory
                  (jaunder-reconcile-report-inventory jaunder-reconcile-report))
                 (staged (jaunder--pull-stage-member
                          (jaunder-reconcile-report-root jaunder-reconcile-report) member))
                 (inventory (jaunder--reconcile-pull-unique-match row)))
            (if (not (plist-get inventory :ok))
                (jaunder--reconcile-blocked row (plist-get inventory :reason)
                                            (plist-get inventory :detail))
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
                 (t (jaunder--reconcile-pull-install-staged row staged remote path))))))
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
                     :detail (error-message-string err)))))))

(defun jaunder--reconcile-pull-row (row)
  "Pull one explicitly selected ROW under the server-only and matched contracts."
  (pcase (jaunder-reconcile-row-state row)
    ('unchanged (list :outcome 'no-op :post-id (jaunder--reconcile-row-post-id row)
                      :slug (jaunder--reconcile-row-slug row) :local-effect 'unchanged
                      :reason 'unchanged))
    ('server-only
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
                    :reason 'pull-failed :detail (error-message-string err)))))
    ('server-ahead (jaunder--reconcile-pull-server-ahead-row row))
    (_ (jaunder--reconcile-blocked row 'pull-ineligible
                                   (jaunder-reconcile-row-state row)))))

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
      (jaunder--reconcile-execute-batch buffer rows 'push
                                        #'jaunder--reconcile-push-row))))

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

(defun jaunder--reconcile-keep-local-send (row path xml &optional merged-bytes)
  "Send reviewed XML for ROW, then checkpoint PATH only on confirmed commit.
When MERGED-BYTES is present, atomically install that authored result before
server-confirmed metadata write-back.  Never install it before the PUT."
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
      (list :outcome 'unknown :post-id (jaunder--reconcile-row-post-id row)
            :slug (jaunder--reconcile-row-slug row) :etag etag
            :local-effect 'unchanged :reason 'remote-outcome-unknown
            :detail (format "PUT response lost; reconcile before retrying: %s"
                            (error-message-string lost))))
     ((eq status 412)
      (append (jaunder--reconcile-blocked row 'etag-stale)
              (list :etag etag :http-status status)))
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
                                (let ((slug (jaunder--write-back received nil nil t)))
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
  (let ((initial (jaunder--reconcile-conflict-preflight row)))
    (if (not (plist-get initial :ok))
        initial
      (let* ((path (jaunder-inventory-local-path (jaunder-reconcile-row-local row)))
             (prepared
              (condition-case err
                  (list :xml (jaunder--reconcile-call-with-source-buffer
                              path #'jaunder--prepare-reviewed-update))
                (error (list :error err)))))
        (if (plist-get prepared :error)
            (list :outcome 'failed :post-id (jaunder--reconcile-row-post-id row)
                  :slug (jaunder--reconcile-row-slug row) :local-effect 'unchanged
                  :reason 'publish-preparation-failed
                  :detail (error-message-string (plist-get prepared :error)))
          (let ((final (jaunder--reconcile-conflict-preflight row)))
            (if (not (plist-get final :ok))
                final
              (jaunder--reconcile-keep-local-send row path (plist-get prepared :xml)))))))))

(defun jaunder-reconcile-keep-local-selected ()
  "Publish reviewed local content for selected conflicts after confirmation."
  (interactive)
  (let ((rows (jaunder-reconcile-selected-rows))
        (buffer (current-buffer)))
    (unless rows (user-error "No reconciliation rows selected"))
    (when (jaunder--reconcile-confirm
           "Keep local for %d selected Post(s)? " (length rows))
      (jaunder--call-with-blog
       (jaunder-reconcile-report-root jaunder-reconcile-report)
       (lambda ()
         (jaunder--reconcile-execute-batch
          buffer rows 'keep-local #'jaunder--reconcile-keep-local-row))))))

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
  (let ((initial (jaunder--reconcile-conflict-preflight row)))
    (if (not (plist-get initial :ok))
        initial
      (condition-case err
          (let* ((root (jaunder-reconcile-report-root jaunder-reconcile-report))
                 (member (jaunder-reconcile-row-member row))
                 (jaunder--pull-link-inventory
                  (jaunder-reconcile-report-inventory jaunder-reconcile-report))
                 (staged (jaunder--pull-stage-member root member)))
            (cond
             ((not (jaunder--reconcile-stage-matches-review-p row staged))
              (jaunder--reconcile-blocked row 'staged-identity-changed))
             (t
              (let ((final (jaunder--reconcile-conflict-preflight row)))
                (if (plist-get final :ok)
                    (list :staged staged)
                  final)))))
        (jaunder-pull-stage-identity-changed
         (jaunder--reconcile-blocked row 'staged-identity-changed))
        (error
         (list :outcome 'failed :post-id (jaunder--reconcile-row-post-id row)
               :slug (jaunder--reconcile-row-slug row) :local-effect 'unchanged
               :reason 'merge-staging-failed :detail (error-message-string err)))))))

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

(defun jaunder--reconcile-merge-prepared-xml (path bytes)
  "Prepare the authorized Org BYTES for PATH without writing that local Post."
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
  (let ((session jaunder-reconcile-merge-session))
    (unless session
      (user-error "This buffer is not a reconciliation merge result"))
    (when (y-or-n-p "Discard the editable merge result permanently? ")
      (jaunder--reconcile-merge-cleanup session))))

(defun jaunder-reconcile-merge-finish ()
  "Explicitly publish this scratch after revalidating the reviewed conflict.
Never infer approval from exiting Ediff.  Preserve scratch after a blocked,
unknown, failed, or partial result; only confirmed success retires it."
  (interactive)
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
                                         (xml (jaunder--reconcile-merge-prepared-xml
                                               path bytes)))
                                    (list :bytes bytes :xml xml))
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
                                (plist-get prepared :bytes)))))))))))))
        (setq-local jaunder-reconcile-merge-last-result result)
        (jaunder--reconcile-merge-record session report-buffer result)
        (if (eq (plist-get result :outcome) 'success)
            (if (equal reviewed-text (buffer-substring-no-properties
                                      (point-min) (point-max)))
                (jaunder--reconcile-merge-cleanup session)
              (message "Remote merge committed; scratch changed during send and was retained"))
          (message "Merge %s: %s; scratch retained for fresh review"
                   (plist-get result :outcome) (plist-get result :reason)))
        result))))

(defun jaunder-reconcile-merge-cancel ()
  "Retain the editable scratch; leaving Ediff never publishes it."
  (interactive)
  (unless jaunder-reconcile-merge-session
    (user-error "This buffer is not a reconciliation merge result"))
  (message (if (jaunder-reconcile-merge-session-ediff-ready
                jaunder-reconcile-merge-session)
               "Merge cancelled; %s retained for later completion or explicit discard"
             "Ediff failed; %s retained for inspection or explicit discard; start a fresh merge")
           (buffer-name)))

(defun jaunder--reconcile-merge-bind-ediff-result (session name)
  "Make Ediff's actual merge output the editable scratch for SESSION.
Called from Ediff's control-buffer startup hook after its two-way merge output
is initialized.  Ediff's copy commands and direct Org edits then write the
same buffer; no local Post file is associated with that buffer."
  (let ((result ediff-buffer-C))
    (unless (and (buffer-live-p result) (not (buffer-file-name result)))
      (error "Ediff did not provide a non-file-visiting merge output"))
    (with-current-buffer result
      (rename-buffer name)
      (jaunder-reconcile-merge-mode)
      (setq-local jaunder-reconcile-merge-session session))
    (setf (jaunder-reconcile-merge-session-scratch session) result)
    ;; Never let a user-wide Ediff autostore preference save or retire this
    ;; client-managed result on Ediff quit; only explicit finish may publish.
    (setq-local ediff-autostore-merges nil)
    (add-hook 'ediff-quit-hook
              (lambda () (jaunder--reconcile-merge-close-views session)) nil t)))

(defun jaunder-reconcile-merge-selected ()
  "Stage exactly one reviewed conflict and open a two-way Ediff merge.
Ediff's actual merge output is the independent editable Org scratch; copy
operations write there, never to either Post.  `C-c C-c' explicitly finishes."
  (interactive)
  (let* ((rows (jaunder-reconcile-selected-rows))
         (report-buffer (current-buffer))
         (root (jaunder-reconcile-report-root jaunder-reconcile-report)))
    (unless (= (length rows) 1)
      (user-error "Select exactly one conflict row for Ediff"))
    (let* ((row (car rows))
           (name (jaunder--reconcile-merge-scratch-name row root)))
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
                           nil)
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
                 (jaunder-reconcile-merge-session-scratch session))))))))))

(defun jaunder--reconcile-keep-remote-row (row)
  "Install ROW's reviewed remote Post, or return a structured blocked result."
  (let ((initial (jaunder--reconcile-conflict-preflight row)))
    (if (not (plist-get initial :ok))
        initial
      (condition-case err
          (let* ((root (jaunder-reconcile-report-root jaunder-reconcile-report))
                 (member (jaunder-reconcile-row-member row))
                 (path (jaunder-inventory-local-path (jaunder-reconcile-row-local row)))
                 (jaunder--pull-link-inventory
                  (jaunder-reconcile-report-inventory jaunder-reconcile-report))
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
                     :reason 'pull-failed :detail (error-message-string err)))))))

(defun jaunder-reconcile-keep-remote-selected ()
  "Accept the reviewed remote Post for selected conflicts after confirmation."
  (interactive)
  (let ((rows (jaunder-reconcile-selected-rows))
        (buffer (current-buffer)))
    (unless rows (user-error "No reconciliation rows selected"))
    (when (jaunder--reconcile-confirm
           "Keep remote for %d selected Post(s)? " (length rows))
      (jaunder--call-with-blog
       (jaunder-reconcile-report-root jaunder-reconcile-report)
       (lambda ()
         (jaunder--reconcile-execute-batch
          buffer rows 'keep-remote #'jaunder--reconcile-keep-remote-row))))))

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
          report))))))



(provide 'jaunder-reconcile)
;;; jaunder-reconcile.el ends here
