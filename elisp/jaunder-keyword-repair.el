;;; jaunder-keyword-repair.el --- Read-only tag repair preview -*- lexical-binding: t; -*-

;; Copyright (C) 2026 Jaunder contributors

;;; Commentary:
;; Preview is a separate authority seam, not reconciliation or publication.
;; Only configured-root inventory and fresh current Members establish targets;
;; sync markers and mtimes are never write authority.  No apply exists here.
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
  "Return the original typed source of a preview failure CONDITION.
For deliberate structured diagnostics only: never automatically format this
source, which can contain filesystem paths, remote content or credentials."
  (let ((source (get (plist-get (cdr condition) :source) 'jaunder-source-condition)))
    (if (eq (car source) 'jaunder-inventory-decode-error)
        (jaunder-inventory-decode-error-source source)
      source)))

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
      (unless (= (length controls) 1) (jaunder--keyword-repair-reject 'invalid-member-control))
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
          (jaunder--keyword-repair-reject 'invalid-member-draft))))
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
not create a visual UI or implement application.  Expected eligibility refusals
become blocked rows; unexpected failures abort with a contextual
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

(provide 'jaunder-keyword-repair)
;;; jaunder-keyword-repair.el ends here
