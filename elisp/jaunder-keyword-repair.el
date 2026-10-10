;;; jaunder-keyword-repair.el --- Reviewed conditional tag repair -*- lexical-binding: t; -*-

;; Copyright (C) 2026 Jaunder contributors

;;; Commentary:
;; Preview is a separate authority seam, not reconciliation or publication.
;; Only configured-root inventory and fresh current Members establish targets;
;; sync markers and mtimes are never write authority.  Apply consumes only a
;; selected, confirmed snapshot, without local publication or write-back.
;; The opaque report retains an immutable snapshot.  Its public data accessor
;; returns detached inspection data, including literal source digests and the
;; complete remote XML needed for later reviewed conditional application.

;;; Code:

(require 'cl-lib)
(require 'jaunder-config)
(require 'jaunder-entry)
(require 'jaunder-org)
(require 'jaunder-inventory)
(require 'jaunder-atom)
(require 'jaunder-transport)
(require 'jaunder-pull)

(define-error 'jaunder-keyword-repair-ineligible "Ineligible Jaunder repair" 'error)
(define-error 'jaunder-keyword-repair-preview-error "Jaunder repair preview failed" 'error)
(define-error 'jaunder-keyword-repair-http-error "Jaunder Member read failed" 'error)

(defun jaunder-keyword-repair-error-source (condition)
  "Return the original typed source of a repair failure CONDITION.
For an apply result plist, pass (cons \='error RESULT).
For deliberate structured diagnostics only: never automatically format this
source, which can contain filesystem paths, remote content or credentials."
  (let ((source (get (plist-get (cdr condition) :source) 'jaunder-source-condition)))
    (cond ((eq (car source) 'jaunder-inventory-decode-error)
           (jaunder-inventory-decode-error-source source))
          ((eq (car source) 'jaunder-keyword-repair-preview-error)
           (jaunder-keyword-repair-error-source source))
          (t source))))

(defun jaunder--keyword-repair-call (stage id thunk)
  "Call THUNK at stable STAGE for Post ID, preserving unexpected failures.
The uninterned source handle retains the original condition without printing
its sensitive data in ordinary `error-message-string' or telemetry output."
  (condition-case source
      (funcall thunk)
    ((jaunder-keyword-repair-ineligible jaunder-pull-validation-error jaunder-invalid-keyword
                                        jaunder-inventory-duplicate-remote-id)
     (signal (car source) (cdr source)))
    (jaunder-keyword-repair-preview-error (signal (car source) (cdr source)))
    (error
     (let ((handle (make-symbol "redacted-source")))
       (put handle 'jaunder-source-condition source)
       (signal 'jaunder-keyword-repair-preview-error
               (list :stage stage :id id :source handle))))))

(defun jaunder--keyword-repair-reject (reason)
  "Signal expected eligibility rejection with stable REASON."
  (signal 'jaunder-keyword-repair-ineligible (list reason)))

(defconst jaunder--keyword-repair-max-tags 256
  "Distinct-tag bound shared with Rust MAX_TAGS_PER_POST.")

(cl-defstruct (jaunder-keyword-repair-report
               (:constructor jaunder--make-keyword-repair-report)
               (:conc-name jaunder--keyword-repair-report-))
  "Opaque read-only repair snapshot; inspect with the public data accessor."
  (data nil :read-only t))

(defun jaunder--keyword-repair-copy (value)
  "Deep-copy inspection VALUE, including all strings."
  (cond ((stringp value) (copy-sequence value))
        ((consp value) (cons (jaunder--keyword-repair-copy (car value))
                             (jaunder--keyword-repair-copy (cdr value))))
        (t value)))

(defun jaunder-keyword-repair-report-data (report)
  "Return detached plist inspection data for immutable REPORT.
Keys are :root, :base-url, :username, and deterministic :rows.  Each row
binds :id, :path, :sha256, :etag, :member-xml, :existing-labels,
:additions (missing canonical identities' labels), :union, :status and
:reason.  Status is eligible, unchanged, or blocked.  Eligibility is not
permission to apply; a later application must revalidate and confirm."
  (jaunder--keyword-repair-copy (jaunder--keyword-repair-report-data report)))

(defun jaunder--keyword-repair-labels (labels)
  "Validate and deduplicate LABELS without changing existing spelling."
  (let ((seen (make-hash-table :test #'equal)) out)
    (dolist (label labels (nreverse out))
      (unless (and (jaunder--valid-tag-label-p label)
                   (equal label (string-trim label)))
        (jaunder--keyword-repair-reject 'invalid-remote-label))
      (unless (gethash (downcase label) seen)
        (puthash (downcase label) t seen)
        (push label out)))))

(defun jaunder--keyword-repair-remote-fields (xml id etag)
  "Validate current Member XML for ID and ETAG without installing anything.
Use the strict pull parser, then validate preservation-sensitive wire shape
that the pull projection does not need to preserve.  Never normalize server
labels, or substitute local source for an unrepresentable remote field."
  (let* ((dom (jaunder--keyword-repair-call
               'member-decode id
               (lambda ()
                 (with-temp-buffer
                   (insert xml)
                   (car (xml-parse-region (point-min) (point-max)))))))
         (namespaces (jaunder--atom-namespace-context dom nil))
         (fields (jaunder--harvest-response-fields xml)))
    (unless (and (eq (jaunder--atom-local-name (car dom)) 'entry)
                 (equal (jaunder--atom-element-namespace dom namespaces) jaunder--atom-ns))
      (jaunder--keyword-repair-reject 'invalid-member-root))
    ;; These fields must not be silently ignored or flattened on replacement.
    (dolist (child (cl-remove-if-not #'listp (dom-children dom)))
      (let* ((name (jaunder--atom-local-name (car child)))
             (namespace (jaunder--atom-element-namespace
                         child (jaunder--atom-namespace-context child namespaces)))
             (expected (cond ((memq name '(title summary content category published link)) jaunder--atom-ns)
                             ((memq name '(slug audience)) jaunder--atompub-ns)
                             ((eq name 'control) jaunder--app-ns))))
        (when (and expected (not (equal namespace expected)))
          (jaunder--keyword-repair-reject 'foreign-member-namespace))
        (when (memq name '(title summary slug audience published))
          (unless (cl-every #'stringp (dom-children child))
            (jaunder--keyword-repair-reject 'member-scalar-markup)))
        (when (memq name '(title summary))
          (unless (member (dom-attr child 'type) '(nil "text"))
            (jaunder--keyword-repair-reject 'unsupported-metadata-type)))
        (when (eq name 'content)
          (when (dom-attr child 'src) (jaunder--keyword-repair-reject 'external-member-content))
          (unless (equal (dom-attr child 'type) "xhtml")
            (unless (cl-every #'stringp (dom-children child))
              (jaunder--keyword-repair-reject 'member-text-markup))))
        (when (eq name 'category)
          (when (or (dom-attr child 'scheme) (dom-attr child 'label)
                    (cl-some #'listp (dom-children child)))
            (jaunder--keyword-repair-reject 'unsupported-category-semantics)))))
    (let ((controls (jaunder--atom-direct-elements-in-namespace
                     dom 'control jaunder--app-ns namespaces)))
      ;; Published/scheduled server Members omit control.  The strict pull
      ;; parser below still requires a valid published instant in that shape.
      (when (> (length controls) 1) (jaunder--keyword-repair-reject 'invalid-member-control))
      (when controls
        (let* ((control (car controls))
               (drafts (jaunder--atom-direct-elements-in-namespace
                        control 'draft jaunder--app-ns
                        (jaunder--atom-namespace-context control namespaces))))
          (unless (and (= (length drafts) 1)
                       (cl-every (lambda (child)
                                   (or (eq child (car drafts))
                                       (and (stringp child)
                                            (string-match-p "\\`[ \t\r\n]*\\'" child))))
                                 (dom-children control))
                       (cl-every #'stringp (dom-children (car drafts))))
            (jaunder--keyword-repair-reject 'invalid-member-draft)))))
    ;; An explicit audience is required: unknown legacy omission cannot prove
    ;; preservation of the current audience on a replacement.
    (jaunder--parse-pulled-member xml etag (current-time) "UTC" t)
    (unless (equal (cdr (assq 'edit-uris fields)) (list (jaunder--member-url id)))
      (jaunder--keyword-repair-reject 'foreign-member))
    fields))

(defun jaunder--keyword-repair-local-snapshot (path)
  "Read PATH once, returning its byte digest, actual Org identity and terms."
  (let ((bytes (with-temp-buffer
                 (set-buffer-multibyte nil)
                 (insert-file-contents-literally path)
                 (buffer-string))))
    (with-temp-buffer
      (insert (decode-coding-string bytes 'utf-8))
      (let ((org-mode-hook nil)) (org-mode))
      (let* ((limit (jaunder--body-start))
             (properties (save-restriction
                           (narrow-to-region (point-min) limit)
                           (jaunder--collect-properties (org-collect-keywords '("PROPERTY")))))
             (ids (cl-remove-if-not (lambda (property) (equal (upcase (car property)) "JAUNDER_ID"))
                                    properties)))
        (list :sha256 (secure-hash 'sha256 bytes)
              :ids (mapcar (lambda (property) (jaunder--canonical-post-id (cdr property))) ids)
              :terms (jaunder--split-keywords (jaunder--category-keyword-values)))))))

(defun jaunder--keyword-repair-row (local members locals collection-reason)
  "Build one preview row for LOCAL from MEMBERS and LOCALS evidence.
COLLECTION-REASON blocks every target when enumeration did not complete."
  (let* ((path (jaunder-inventory-local-path local))
         (id (jaunder--canonical-post-id (jaunder-inventory-local-id local)))
         (targets (cl-remove-if-not (lambda (member) (equal id (jaunder-inventory-member-id member))) members))
         (row (list :id id :path path :sha256 nil :etag nil :member-xml nil
                    :existing-labels nil :additions nil :union nil :status 'blocked :reason nil))
         (reason collection-reason))
    (condition-case rejection
        (let* ((snapshot (jaunder--keyword-repair-call
                          'local-read id (lambda () (jaunder--keyword-repair-local-snapshot path))))
               (labels nil))
          (setq row (plist-put row :sha256 (plist-get snapshot :sha256)))
          (condition-case nil
              (setq labels (mapcar #'jaunder-normalize-keyword (plist-get snapshot :terms)))
            (jaunder-invalid-keyword (setq reason (or reason 'invalid-local-keyword))))
          (setq labels (jaunder--keyword-repair-labels labels))
          (setq reason
                (or reason
                    (cond
                     ((cl-some (lambda (buffer)
                                 (and (buffer-file-name buffer)
                                      (not (file-remote-p (buffer-file-name buffer)))
                                      (equal (file-truename (buffer-file-name buffer)) (file-truename path))
                                      (buffer-modified-p buffer))) (buffer-list)) 'modified-buffer)
                     ((not id) 'missing-or-invalid-local-id)
                     ((not (equal (plist-get snapshot :ids) (list id))) 'ambiguous-local-id)
                     ((> (cl-count id locals :key #'jaunder-inventory-local-id :test #'equal) 1) 'duplicate-local-id)
                     ((null targets) 'remote-missing)
                     ((cdr targets) 'duplicate-remote-id)
                     ((> (cl-count (jaunder-inventory-member-slug (car targets)) members
                                   :key #'jaunder-inventory-member-slug :test #'equal) 1) 'duplicate-target-slug)
                     ((not (equal (jaunder-inventory-member-edit-uri (car targets)) (jaunder--member-url id))) 'foreign-member))))
          (unless reason
            (let* ((response (jaunder--keyword-repair-call
                              'member-read id
                              (lambda ()
                                (let* ((response (jaunder--http-request "GET" (jaunder--member-url id)))
                                       (status (plist-get response :status)))
                                  (unless (and (integerp status)
                                               (or (<= 200 status 299) (memq status '(403 404 410))))
                                    (signal 'jaunder-keyword-repair-http-error
                                            (list (and (integerp status) status))))
                                  response))))
                   (status (plist-get response :status))
                   (etag (jaunder--response-header response "etag"))
                   (xml (plist-get response :body)))
              (setq reason (cond ((memq status '(404 410)) 'remote-missing)
                                 ((equal status 403) 'foreign-member)
                                 ((not (and (= (length (jaunder--pull-media-header-values response "etag")) 1)
                                            (jaunder--strong-etag-p etag))) 'current-etag-invalid)))
              (unless reason
                (setq row (plist-put row :etag etag)
                      row (plist-put row :member-xml xml))
                (let* ((fields (jaunder--keyword-repair-call
                                'member-validate id
                                (lambda () (jaunder--keyword-repair-remote-fields xml id etag))))
                       (existing (jaunder--keyword-repair-labels (cdr (assq 'categories fields))))
                       (identities (mapcar #'downcase existing))
                       (additions (cl-remove-if (lambda (label) (member (downcase label) identities)) labels))
                       (union (append existing additions)))
                  (setq row (plist-put row :existing-labels existing)
                        row (plist-put row :additions additions)
                        row (plist-put row :union union)
                        reason (cond ((equal (cdr (assq 'drafts fields)) '("yes")) 'remote-draft)
                                     ((> (length union) jaunder--keyword-repair-max-tags) 'over-tag-cap)))
                  (unless reason
                    (setq row (plist-put row :status (if additions 'eligible 'unchanged)))))))))
      (jaunder-keyword-repair-ineligible (setq reason (cadr rejection)))
      (jaunder-pull-validation-error (setq reason 'invalid-member)))
    (plist-put row :reason reason)))

(defun jaunder-keyword-repair-preview (&optional directory)
  "Return an immutable read-only repair report for DIRECTORY's configured root.
Only direct root Org files participate, as in the existing inventory.  All
network requests are GETs.  No local bytes, buffers, Media or sync markers are
written.  Inspect with `jaunder-keyword-repair-report-data'; this command does
not create a visual UI.  Apply selected indices with
`jaunder-keyword-repair-apply' only after reviewing this exact report.
Expected eligibility refusals become blocked rows; unexpected failures
abort with a contextual
`jaunder-keyword-repair-preview-error'.  Its source is available only through
`jaunder-keyword-repair-error-source', not ordinary diagnostic formatting."
  (interactive)
  (let* ((directory (or directory default-directory))
         (entry (jaunder--blog-entry-for directory)))
    (unless entry (user-error "No configured blog root"))
    (let ((root (file-name-as-directory (expand-file-name (car entry)))))
      (jaunder--call-with-blog
       root
       (lambda ()
         (let* ((locals (jaunder--keyword-repair-call
                         'local-inventory nil (lambda () (jaunder--scan-root-locals root))))
                (members nil)
                (reason nil))
           (condition-case nil
               (setq members (jaunder--keyword-repair-call
                              'collection nil #'jaunder--fetch-collection-members))
             (jaunder-inventory-duplicate-remote-id (setq reason 'duplicate-remote-id)))
           (jaunder--make-keyword-repair-report
            :data (jaunder--keyword-repair-copy
                   (list :root root :base-url (jaunder--active-base-url)
                         :username (jaunder--active-username)
                         :rows (mapcar (lambda (local)
                                         (jaunder--keyword-repair-call
                                          'row-validate (jaunder-inventory-local-id local)
                                          (lambda () (jaunder--keyword-repair-row local members locals reason))))
                                       locals))))))))))

(defvar jaunder-keyword-repair-last-application nil
  "Last application plist with ordered :results and :cancelled.
Updated after every terminal Post outcome, including interrupted writes.
Inspection data contains no XML; source handles require deliberate access via
`jaunder-keyword-repair-error-source'.  No rollback or automatic retry occurs.")

(defun jaunder--keyword-repair-binding-valid-p (data path)
  "Check DATA's reviewed configuration still governs PATH."
  (let ((entry (jaunder--blog-entry-for path))
        (config (jaunder--resolve-blog path)))
    (and entry
         (equal (file-name-as-directory (expand-file-name (car entry)))
                (plist-get data :root))
         (equal (plist-get config :base-url) (plist-get data :base-url))
         (equal (plist-get config :username) (plist-get data :username)))))

(defun jaunder--keyword-repair-check-local (row)
  "Revalidate ROW's literal local proof without altering a visiting buffer."
  (let ((path (plist-get row :path)))
    (unless (and (file-regular-p path) (not (file-symlink-p path)))
      (jaunder--keyword-repair-reject 'local-path-changed))
    (when (cl-some (lambda (buffer)
                     (and (buffer-file-name buffer)
                          (not (file-remote-p (buffer-file-name buffer)))
                          (equal (file-truename (buffer-file-name buffer)) (file-truename path))
                          (buffer-modified-p buffer))) (buffer-list))
      (jaunder--keyword-repair-reject 'modified-buffer))
    (let ((snapshot (jaunder--keyword-repair-call
                     'local-read (plist-get row :id)
                     (lambda () (jaunder--keyword-repair-local-snapshot path)))))
      (unless (and (equal (plist-get snapshot :sha256) (plist-get row :sha256))
                   (equal (plist-get snapshot :ids) (list (plist-get row :id))))
        (jaunder--keyword-repair-reject 'local-proof-changed)))))

(defun jaunder--keyword-repair-payload (xml labels)
  "Replace only Atom category children in remote XML with validated LABELS.
Keep qualified names, namespace declarations, attributes and nested markup.
Each new category declares its own namespace, independent of prefix bindings."
  (let* ((dom (with-temp-buffer
                (insert xml)
                (car (xml-parse-region (point-min) (point-max)))))
         (context (jaunder--atom-namespace-context dom nil))
         (categories (jaunder--atom-direct-elements-in-namespace
                      dom 'category jaunder--atom-ns context)))
    (setcdr (cdr dom)
            (append (cl-remove-if (lambda (child) (memq child categories))
                                  (dom-children dom))
                    (mapcar (lambda (label)
                              (list 'category (list (cons 'xmlns jaunder--atom-ns)
                                                    (cons 'term label)))) labels)))
    (with-temp-buffer (dom-print dom) (buffer-string))))

(defun jaunder--keyword-repair-apply-row (data row)
  "Apply one reviewed ROW bound to DATA, returning a terminal result.
Read/preflight failures block.  Once the HTTP seam is entered a thrown
condition cannot prove non-dispatch, so conservatively report unknown."
  ;; The executor protects the return-to-checkpoint interval, not in-flight
  ;; work: keyboard quits during reads/PUT must still reach this handler.
  (let ((inhibit-quit nil)
        (id (plist-get row :id))
        (stage 'preflight)
        (dispatched nil)
        (result (list :id (plist-get row :id) :path (plist-get row :path))))
    (condition-case source
        (progn
          (unless (jaunder--keyword-repair-binding-valid-p data (plist-get row :path))
            (jaunder--keyword-repair-reject 'configuration-changed))
          (cond
           ((eq (plist-get row :status) 'blocked)
            (jaunder--keyword-repair-reject (plist-get row :reason)))
           (t
            (jaunder--call-with-blog
             (plist-get row :path)
             (lambda ()
               (jaunder--keyword-repair-check-local row)
               ;; Enumerate the complete CURRENT Collection for every Post.
               ;; The original inventory never proves current uniqueness.
               (let* ((locals (jaunder--keyword-repair-call
                               'local-inventory id
                               (lambda () (jaunder--scan-root-locals (plist-get data :root)))))
                      (members (jaunder--keyword-repair-call
                                'collection id #'jaunder--fetch-collection-members))
                      (local (cl-find (plist-get row :path) locals
                                      :key #'jaunder-inventory-local-path :test #'equal)))
                 (unless local (jaunder--keyword-repair-reject 'local-path-changed))
                 (let ((fresh (jaunder--keyword-repair-row local members locals nil)))
                   (when (eq (plist-get fresh :status) 'blocked)
                     (jaunder--keyword-repair-reject (plist-get fresh :reason)))
                   (unless (equal (plist-get fresh :etag) (plist-get row :etag))
                     (jaunder--keyword-repair-reject 'remote-etag-changed))
                   (unless (and (equal (plist-get fresh :sha256) (plist-get row :sha256))
                                (equal (plist-get fresh :union) (plist-get row :union))
                                (equal (plist-get fresh :additions) (plist-get row :additions)))
                     (jaunder--keyword-repair-reject 'reviewed-intent-changed))
                   (setq stage 'payload)
                   (let ((payload (when (plist-get row :additions)
                                    (jaunder--keyword-repair-payload
                                     (plist-get fresh :member-xml) (plist-get row :union)))))
                     ;; Final local/config checks follow the network reads.
                     (setq stage 'preflight)
                     (let ((current-locals
                            (jaunder--keyword-repair-call
                             'local-inventory id
                             (lambda () (jaunder--scan-root-locals (plist-get data :root))))))
                       (unless (= (cl-count id current-locals :key #'jaunder-inventory-local-id
                                            :test #'equal) 1)
                         (jaunder--keyword-repair-reject 'duplicate-local-id)))
                     (jaunder--keyword-repair-check-local row)
                     (unless (jaunder--keyword-repair-binding-valid-p data (plist-get row :path))
                       (jaunder--keyword-repair-reject 'configuration-changed))
                     ;; A reviewed no-op shares all proof checks, but never
                     ;; acquires write authority from newly missing identities.
                     (if (not (plist-get row :additions))
                         (plist-put result :status 'unchanged)
                       (setq stage 'put dispatched t)
                       (let* ((response (jaunder--http-request
                                         "PUT" (jaunder--member-url id) payload "application/atom+xml"
                                         (list (cons "If-Match" (plist-get row :etag)))))
                              (status (plist-get response :status)))
                         (setq result (plist-put result :http-status (and (integerp status) status)))
                         (plist-put result :status
                                    (cond ((not (and (integerp status) (<= 100 status 599)))
                                           'unknown-remote-outcome)
                                          ((<= 200 status 299) 'repaired)
                                          ((= status 412) 'blocked)
                                          (t 'failed)))))))))))))
      (quit
       (setq result (plist-put result :cancelled t))
       (plist-put result :status (if dispatched 'unknown-remote-outcome 'blocked)))
      ((jaunder-keyword-repair-ineligible jaunder-pull-validation-error
                                          jaunder-inventory-duplicate-remote-id)
       (setq result (plist-put result :reason
                               (if (eq (car source) 'jaunder-keyword-repair-ineligible)
                                   (cadr source) (car source))))
       (plist-put result :status (if dispatched 'unknown-remote-outcome 'blocked)))
      (error
       (let ((handle (make-symbol "redacted-source")))
         (put handle 'jaunder-source-condition source)
         (setq result (plist-put result :source handle)
               result (plist-put result :stage
                                 (if (eq (car source) 'jaunder-keyword-repair-preview-error)
                                     (plist-get (cdr source) :stage) stage)))
         (plist-put result :status (cond (dispatched 'unknown-remote-outcome)
                                         ((eq stage 'payload) 'failed)
                                         (t 'blocked))))))))

(defun jaunder-keyword-repair-apply (report selection)
  "Confirm and apply exact REPORT rows at ordered zero-based SELECTION indices.
SELECTION must be an explicit nonempty list of distinct indices into the
public report data's :rows.  It never re-runs a selection filter or trusts
caller-edited row data.  Ask `yes-or-no-p' before any mutation.  Return
(:results ORDERED-TERMINAL-RESULTS :cancelled BOOLEAN), also retained in
`jaunder-keyword-repair-last-application'.  Quit stops before the next Post;
an interrupted possible PUT records unknown-remote-outcome, never retries.
No local Post, synchronization marker or Media is written."
  (unless (jaunder-keyword-repair-report-p report) (user-error "Not a repair report"))
  (let* ((data (jaunder-keyword-repair-report-data report))
         (rows (plist-get data :rows))
         (selected nil)
         (results nil)
         (cancelled nil))
    (unless (and (consp selection) (proper-list-p selection)
                 (= (length selection) (length (delete-dups (copy-sequence selection))))
                 (cl-every (lambda (index) (and (integerp index) (<= 0 index) (< index (length rows))))
                           selection))
      (user-error "Select distinct reviewed row indices"))
    (setq selected (mapcar (lambda (index) (nth index rows)) selection)
          jaunder-keyword-repair-last-application (list :results nil :cancelled nil))
    (condition-case nil
        (if (not (yes-or-no-p (format "Apply reviewed tag additions to %d selected Posts? " (length selected))))
            (setq cancelled t)
          (while (and selected (not cancelled))
            ;; Own pending input until the completed row is checkpointed.
            ;; The row itself enables quit, including inside its HTTP seam.
            (let ((inhibit-quit t))
              (let ((result (jaunder--keyword-repair-apply-row data (pop selected))))
                (setq results (append results (list result))
                      cancelled (or (plist-get result :cancelled) quit-flag)
                      jaunder-keyword-repair-last-application
                      (list :results (jaunder--keyword-repair-copy results) :cancelled cancelled))
                ;; Honoring cancellation cannot drop the completed outcome or
                ;; begin the next Post.  Explicit in-row quit remains unknown
                ;; after possible PUT dispatch, as recorded by the row handler.
                (when cancelled (setq quit-flag nil))))))
      (quit (setq cancelled t)))
    (setq jaunder-keyword-repair-last-application
          (list :results (jaunder--keyword-repair-copy results) :cancelled cancelled))
    (jaunder--keyword-repair-copy jaunder-keyword-repair-last-application)))

(provide 'jaunder-keyword-repair)
;;; jaunder-keyword-repair.el ends here
