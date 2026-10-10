;;; jaunder-reconcile-operation.el --- Operation-owned reconciliation evidence -*- lexical-binding: t; -*-

;; Copyright (C) 2026 Jaunder contributors

;;; Commentary:
;; One short-lived owner for complete remote discovery and fresh local proof.
;; Discovery does not replace reviewed ETags or authorize mutations.  The private
;; dynamic state stays inside this module; consumers request inventory or proof
;; with scalar root/identity/path arguments.  Write observation precedes local
;; completion; affected proof is restored targetedly, never as mutation authority.
;; Report refresh does not consume operation discovery.

;;; Code:

(require 'cl-lib)
(require 'jaunder-config)
(require 'jaunder-inventory)

(defvar jaunder--operation-evidence nil
  "Private evidence owned by the current reconciliation operation.")

(defvar jaunder--operation-scopes nil
  "Private active owners; nested writes invalidate same-User ancestors.")

(defvar jaunder--operation-write-receipt nil
  "Private write phase for one row, including every keyed create attempt.")

(defun jaunder--call-with-reconcile-operation (root origin username work)
  "Run WORK in independent evidence scope for ROOT, ORIGIN and USERNAME."
  (let* ((jaunder--operation-evidence
          (list :root (file-name-as-directory (expand-file-name root))
                :origin origin :username username :state 'unacquired
                :changes (make-hash-table :test #'equal) :lineages nil))
         (jaunder--operation-scopes (cons jaunder--operation-evidence jaunder--operation-scopes)))
    (funcall work)))

(defun jaunder--call-without-reconcile-operation (work)
  "Run WORK without inheriting an enclosing operation's discovery."
  (let ((jaunder--operation-evidence nil))
    (funcall work)))

(defun jaunder--operation-active-p ()
  "Return non-nil inside an operation evidence scope."
  (not (null jaunder--operation-evidence)))

(defun jaunder--operation-check-scope (root)
  "Reject evidence use outside its captured ROOT, origin or User."
  (unless (and jaunder--operation-evidence
               (equal (file-name-as-directory (expand-file-name root))
                      (plist-get jaunder--operation-evidence :root))
               (equal (jaunder--active-base-url)
                      (plist-get jaunder--operation-evidence :origin))
               (equal (jaunder--active-username)
                      (plist-get jaunder--operation-evidence :username)))
    (error "jaunder reconcile: operation Collection evidence scope mismatch")))

(defun jaunder--operation-remote-members (root)
  "Return ROOT's complete discovery, or signal the retained acquisition failure."
  (jaunder--operation-check-scope root)
  (pcase (plist-get jaunder--operation-evidence :state)
    ((or 'complete 'complete-empty) (plist-get jaunder--operation-evidence :members))
    ('failed (let ((condition (plist-get jaunder--operation-evidence :condition)))
               (signal (car condition) (cdr condition))))
    ('unacquired
     (condition-case err
         (let ((members (jaunder--fetch-collection-members)))
           (setq jaunder--operation-evidence
                 (plist-put jaunder--operation-evidence :members members))
           (setq jaunder--operation-evidence
                 (plist-put jaunder--operation-evidence :state
                            (if members 'complete 'complete-empty)))
           members)
       (error
        (setq jaunder--operation-evidence
              (plist-put jaunder--operation-evidence :condition err))
        (setq jaunder--operation-evidence
              (plist-put jaunder--operation-evidence :state 'failed))
        (signal (car err) (cdr err)))))))

(defun jaunder--operation-current-inventory (root)
  "Join current local files in ROOT to operation discovery when scoped.
Unscoped consumers retain standalone complete acquisition semantics."
  (if (jaunder--operation-active-p)
      (progn
        (jaunder--operation-check-scope root)
        (jaunder--join-inventory (jaunder--scan-root-locals root)
                                 (jaunder--operation-effective-members root)))
    (jaunder--inventory-for-root root)))

(defun jaunder--operation-post-link-evidence (root)
  "Return remote and current local link proof for ROOT without mutation authority."
  (if (not (jaunder--operation-active-p))
      (jaunder--inventory-post-link-evidence (jaunder--inventory-for-root root))
    (jaunder--operation-remote-members root)
    (let ((locals (cl-remove-if
                   (lambda (local) (jaunder--operation-unresolved-create-path-p
                                    (jaunder-inventory-local-path local)))
                   (jaunder--scan-root-locals root))))
      ;; A changed target may have a new canonical href absent from discovery.
      ;; Only locally present targets can authorize inverse localization.
      (dolist (local locals)
        (jaunder--operation-restore-member root (jaunder-inventory-local-id local) t))
      (list (jaunder--operation-effective-members root) locals))))

(defun jaunder--operation-unique-match (root id path)
  "Return current unique-match proof for ID at PATH in ROOT.
A duplicate local or remote Post ID is an actionable blocked result, rather
than an exception flattened into a generic pull failure."
  (condition-case err
      (progn
        (when (jaunder--operation-active-p)
          (when (jaunder--operation-unresolved-create-path-p path)
            (error "jaunder: Post create identity is unresolved"))
          (jaunder--operation-restore-member root id))
        (let* ((inventory (jaunder--operation-current-inventory root))
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
                         :detail "fresh inventory no longer has the reviewed unique match")))))
    (jaunder-inventory-duplicate-remote-id
     (list :reason 'duplicate-remote-id
           :detail (format "fresh inventory has duplicate remote Post ID %s" id)))
    (error (list :reason 'fresh-inventory-failed
                 :detail (error-message-string err)))))

(defun jaunder--operation-local-unique-match (root id path)
  "Return final local uniqueness proof for ID at PATH in ROOT."
  (when (jaunder--operation-active-p)
    (jaunder--operation-check-scope root))
  (let* ((locals (jaunder--scan-root-locals root))
         (same-id (cl-remove-if-not
                   (lambda (local) (equal (jaunder-inventory-local-id local) id)) locals)))
    (cond ((> (length same-id) 1)
           (list :reason 'duplicate-local-id
                 :detail (format "final local scan has duplicate local Post ID %s" id)))
          ((/= (length same-id) 1)
           (list :reason 'matched-identity-changed
                 :detail "final local scan no longer has the reviewed local Post"))
          ((not (equal (jaunder-inventory-local-path (car same-id)) path))
           (list :reason 'matched-identity-changed
                 :detail "final local scan no longer has the reviewed local Post"))
          (t (list :ok t)))))

(defun jaunder--call-with-operation-write-receipt (work)
  "Run row WORK with an independent send-phase receipt."
  (let ((jaunder--operation-write-receipt (list :phase 'not-sent)))
    (funcall work)))

(defun jaunder--operation-created-identity-unresolved-p ()
  "Return non-nil when a confirmed create lacks authoritative identity correlation."
  (plist-get jaunder--operation-write-receipt :create-identity-unresolved))

(defun jaunder--operation-write-phase ()
  "Return the current row's remote write phase, not mutation permission."
  (plist-get jaunder--operation-write-receipt :phase))

(defun jaunder--operation-write-failure (condition &optional detail)
  "Project original CONDITION and optional DETAIL with honest remote phase."
  (let ((phase (jaunder--operation-write-phase)))
    (list :outcome (pcase phase ('confirmed 'partial) ('unknown 'unknown) (_ 'failed))
          :reason (pcase phase ('confirmed (if (plist-get jaunder--operation-write-receipt :checkpoint-condition)
                                               'response-identity-invalid 'local-write-back-failed))
                         ('unknown 'remote-outcome-unknown) (_ 'operation-failed))
          :post-id (plist-get jaunder--operation-write-receipt :id)
          :http-status (plist-get jaunder--operation-write-receipt :status)
          :local-effect (if (and (eq phase 'confirmed)
                                 (not (plist-get jaunder--operation-write-receipt :checkpoint-condition)))
                            'checkpoint-uncertain 'unchanged)
          :detail (concat (pcase phase ('confirmed "Remote Post committed; inspect local completion before retrying: ")
                                 ('unknown "Remote outcome unknown; reconcile before retrying: ") (_ ""))
                          (or detail (error-message-string condition))
                          (let ((earlier (plist-get jaunder--operation-write-receipt :condition)))
                            (if (and (eq phase 'unknown) earlier (not (equal earlier condition)))
                                (format "; earlier write failure: %s" (error-message-string earlier))
                              ""))))))

(defun jaunder--operation-same-user-p (owner)
  "Return non-nil when OWNER names the currently active origin and User."
  (and (equal (plist-get owner :origin) (jaunder--active-base-url))
       (equal (plist-get owner :username) (jaunder--active-username))))

(defun jaunder--operation-member-proof (response expected-uri &optional collection-url)
  "Parse RESPONSE's strict identity and optional link proof at EXPECTED-URI.
Identity and link cardinality are independent.  Validation rejection returns
nil; unexpected XML decode errors retain their original condition at the caller."
  (let* ((collection-url (or collection-url (jaunder--collection-url)))
         (xml (plist-get response :body))
         (dom (with-temp-buffer (insert xml) (car (xml-parse-region (point-min) (point-max)))))
         (namespaces (jaunder--atom-namespace-context dom nil))
         (fields (jaunder--harvest-response-fields xml))
         (edits (cdr (assq 'edit-uris fields)))
         (slugs (cdr (assq 'slugs fields)))
         (alternates (cdr (assq 'alternate-uris fields)))
         (uri (car edits))
         (id (jaunder--collection-edit-id uri collection-url))
         (alternate (jaunder--inventory-alternate-outcome
                     (mapcar (lambda (href) (list 'link (list (cons 'href href)))) alternates)
                     collection-url)))
    (when (and (eq (jaunder--atom-local-name (car dom)) 'entry)
               (equal (jaunder--atom-element-namespace dom namespaces) jaunder--atom-ns)
               (= (length edits) 1) (= (length slugs) 1)
               id (equal uri (concat collection-url "/" id))
               (or (null expected-uri) (equal uri expected-uri))
               (stringp (car slugs)) (not (string-empty-p (car slugs))))
      (jaunder--make-inventory-member
       :id id :edit-uri uri :slug (car slugs)
       :etag (jaunder--response-header response "ETag")
       :alternate-href (car alternate) :alternate-invalid-reason (cadr alternate)))))

(defun jaunder--operation-check-put-response-identity (response uri)
  "Reject contradictory supplied PUT identity at known URI, not omitted metadata.
Alternate href usability is independent of the local identity checkpoint."
  (let* ((fields (jaunder--harvest-response-fields (plist-get response :body)))
         (edits (cdr (assq 'edit-uris fields)))
         (slugs (cdr (assq 'slugs fields)))
         (location (jaunder--response-header response "Location")))
    (when (or (and edits (not (equal edits (list uri))))
              (and slugs (or (/= (length slugs) 1)
                             (not (stringp (car slugs))) (string-empty-p (car slugs))))
              (and location (not (equal location uri))))
      (error "jaunder: confirmed PUT response identity contradicts request Member; checkpoint refused"))))

(defun jaunder--operation-file-identity (path)
  "Return PATH's filesystem lineage identity without interpreting Post headers."
  (when (and path (file-regular-p path))
    (let ((attributes (file-attributes path)))
      (when attributes
        (list (file-attribute-device-number attributes)
              (file-attribute-inode-number attributes))))))

(defun jaunder--call-with-operation-checkpoint-save (work)
  "Run owned checkpoint WORK, observing its disk effect before fallible save hooks.
Capture matching lineage before the effect; later context drift cannot erase it.
Only matching creating-file lineages advance; later hook replacements cannot
inherit them."
  (if (null jaunder--operation-scopes)
      (funcall work)
    (let* ((path (buffer-file-name))
           (identity (jaunder--operation-file-identity path))
           (origin (if jaunder--operation-evidence
                       (plist-get jaunder--operation-evidence :origin) (jaunder--active-base-url)))
           (username (if jaunder--operation-evidence
                         (plist-get jaunder--operation-evidence :username) (jaunder--active-username)))
           lineages)
      (dolist (owner jaunder--operation-scopes)
        (when (and (equal origin (plist-get owner :origin))
                   (equal username (plist-get owner :username)))
          (dolist (lineage (plist-get owner :lineages))
            (when (and identity (equal path (plist-get lineage :path))
                       (equal identity (plist-get lineage :file-identity)))
              (push lineage lineages)))))
      (if (null lineages)
          (funcall work)
        ;; after-save-hook runs after the disk write.  Prepending observes an
        ;; atomic inode replacement before an ordinary hook can fail or replace it.
        (let ((after-save-hook
               (cons (lambda ()
                       (let ((saved-identity (jaunder--operation-file-identity path)))
                         (dolist (lineage lineages)
                           (setf (plist-get lineage :file-identity) saved-identity))))
                     after-save-hook)))
          (funcall work))))))

(defun jaunder--call-with-operation-local-rename (old new work)
  "Run OLD to NEW rename WORK with pre-effect creating-file ownership captured.
Only matching lineages follow a successful move retaining the source identity;
mutable ambient context or an unrelated destination cannot change ownership."
  (if (null jaunder--operation-evidence)
      (funcall work)
    (let ((identity (jaunder--operation-file-identity old))
          (origin (plist-get jaunder--operation-evidence :origin))
          (username (plist-get jaunder--operation-evidence :username))
          lineages)
      (dolist (owner jaunder--operation-scopes)
        (when (and (equal origin (plist-get owner :origin))
                   (equal username (plist-get owner :username)))
          (dolist (lineage (plist-get owner :lineages))
            (when (and identity (equal old (plist-get lineage :path))
                       (equal identity (plist-get lineage :file-identity)))
              (push lineage lineages)))))
      (prog1 (funcall work)
        (when (and identity (equal identity (jaunder--operation-file-identity new)))
          (dolist (lineage lineages)
            (setf (plist-get lineage :path) new)))))))

(defun jaunder--operation-unresolved-create-path-p (path)
  "Return non-nil if PATH still belongs to a create lacking correlated identity."
  (let ((identity (jaunder--operation-file-identity path)))
    (and identity
         (cl-some
          (lambda (owner)
            (and (jaunder--operation-same-user-p owner)
                 (cl-some (lambda (lineage)
                            (and (equal path (plist-get lineage :path))
                                 (equal identity (plist-get lineage :file-identity))))
                          (plist-get owner :lineages))))
          jaunder--operation-scopes))))

(defun jaunder--operation-send-post-write (method uri xml content-type headers)
  "Observe each actual Post METHOD at URI, then return its unchanged HTTP response.
Observation precedes fallible local completion.  Keyed retry callers retain
uncertainty from earlier attempts even if the last request is rejected."
  (if (null jaunder--operation-scopes)
      (jaunder--http-request method uri xml content-type headers)
    (let* ((creating (equal method "POST"))
           (id (unless creating (jaunder--collection-edit-id uri (jaunder--collection-url))))
           (source (buffer-file-name))
           (source-identity (and creating (jaunder--operation-file-identity source)))
           (request-collection (jaunder--collection-url))
           (prior (jaunder--operation-write-phase))
           (send-markers (mapcar (lambda (owner) (cons owner (and id (list :state 'dirty :uri uri))))
                                 (cl-remove-if-not #'jaunder--operation-same-user-p jaunder--operation-scopes)))
           response)
      (when (and (not creating) source (jaunder--operation-unresolved-create-path-p source))
        (error "jaunder: Post create identity is unresolved; local header cannot authorize an update"))
      (when jaunder--operation-evidence
        (jaunder--operation-check-scope (plist-get jaunder--operation-evidence :root)))
      (when jaunder--operation-write-receipt
        (setf (plist-get jaunder--operation-write-receipt :phase) 'unknown
              (plist-get jaunder--operation-write-receipt :id) id))
      (dolist (pair send-markers)
        (let ((owner (car pair)))
          (if id
              (puthash id (cdr pair) (plist-get owner :changes))
            (when (and source (not (cl-find source (plist-get owner :lineages)
                                            :key (lambda (lineage) (plist-get lineage :path)) :test #'equal)))
              (push (list :path source :file-identity source-identity)
                    (plist-get owner :lineages))))))
      (condition-case condition
          (setq response (jaunder--http-request method uri xml content-type headers))
        ((error quit)
         (when jaunder--operation-write-receipt
           (setf (plist-get jaunder--operation-write-receipt :condition) condition))
         (signal (car condition) (cdr condition))))
      (let* ((status (plist-get response :status))
             (confirmed (and (integerp status) (<= 200 status 299)))
             (rejected (and (integerp status) (<= 400 status 499)))
             (checkpoint-condition
              (when (and confirmed (equal method "PUT"))
                (condition-case condition
                    (progn (jaunder--operation-check-put-response-identity response uri) nil)
                  (error condition))))
             (proof (if checkpoint-condition (list :condition checkpoint-condition)
                      (when (and confirmed (not (equal method "DELETE")))
                        (condition-case condition
                            (list :member (jaunder--operation-member-proof response (unless creating uri)
                                                                           request-collection))
                          (error (list :condition condition))))))
             (member (plist-get proof :member))
             (created-id (and creating member
                              (equal (jaunder--response-header response "Location")
                                     (jaunder-inventory-member-edit-uri member))
                              (jaunder-inventory-member-id member))))
        (when jaunder--operation-write-receipt
          (setf (plist-get jaunder--operation-write-receipt :checkpoint-condition) checkpoint-condition)
          (setf (plist-get jaunder--operation-write-receipt :create-identity-unresolved)
                (and creating confirmed (null created-id)))
          (setf (plist-get jaunder--operation-write-receipt :status) status
                (plist-get jaunder--operation-write-receipt :id) (or id created-id)
                (plist-get jaunder--operation-write-receipt :phase)
                (cond (confirmed 'confirmed) ((and rejected (not (eq prior 'unknown))) 'rejected) (t 'unknown))))
        (dolist (pair send-markers)
          (let* ((owner (car pair)) (changes (plist-get owner :changes)) (known-id (or id created-id)))
            (cond
             (confirmed
              (when (and creating (null created-id))
                (dolist (lineage (plist-get owner :lineages))
                  (when (equal source (plist-get lineage :path))
                    (setf (plist-get lineage :condition) (plist-get proof :condition)))))
              (when known-id
                (let* ((current (gethash known-id changes))
                       (owns-marker (if id (eq current (cdr pair)) (null current))))
                  (puthash known-id
                           ;; Delivery order is not commit order.  Superseded
                           ;; responses require current targeted proof, not their
                           ;; returned Member or a child's cached representation.
                           (cond ((not owns-marker)
                                  (list :state 'dirty :uri (concat request-collection "/" known-id)
                                        :condition (or (plist-get proof :condition) (plist-get current :condition))
                                        :original-condition (or (plist-get current :condition)
                                                                (plist-get current :original-condition))))
                                 ((equal method "DELETE") (list :state 'absent))
                                 ((and (eq owner jaunder--operation-evidence) member)
                                  (list :state 'valid :member member :uri (jaunder-inventory-member-edit-uri member)))
                                 (t (list :state 'dirty :uri (concat request-collection "/" known-id)
                                          :condition (plist-get proof :condition)))) changes)))
              (when created-id
                (setf (plist-get owner :lineages)
                      (cl-remove source (plist-get owner :lineages)
                                 :key (lambda (lineage) (plist-get lineage :path)) :test #'equal))))
             (rejected
              ;; Rejection does not prove cached discovery current.  Retain dirty
              ;; proof for targeted restoration, or absence on 404, without
              ;; overwriting a newer nested effect with this send's marker.
              (when (and id (eq (gethash id changes) (cdr pair)) (eq status 404))
                (puthash id (list :state 'absent) changes))
              (when (and creating (not (eq prior 'unknown)))
                (setf (plist-get owner :lineages)
                      (cl-remove source (plist-get owner :lineages)
                                 :key (lambda (lineage) (plist-get lineage :path)) :test #'equal)))))))
        (when checkpoint-condition (signal (car checkpoint-condition) (cdr checkpoint-condition)))
        (when jaunder--operation-evidence
          (jaunder--operation-check-scope (plist-get jaunder--operation-evidence :root)))
        response))))

(defun jaunder--operation-restore-member (root id &optional require-alternate)
  "Restore affected ID in ROOT targetedly when its needed proof is unavailable.
Fresh evidence never changes a row's reviewed ETag or its write-phase receipt."
  (jaunder--operation-check-scope root)
  (let* ((changes (plist-get jaunder--operation-evidence :changes))
         (change (gethash id changes))
         (member (plist-get change :member)))
    (when (or (eq (plist-get change :state) 'dirty)
              (and require-alternate (eq (plist-get change :state) 'valid)
                   (jaunder-inventory-member-alternate-invalid-reason member)))
      (condition-case condition
          (let* ((uri (plist-get change :uri))
                 (response (jaunder--http-request "GET" uri))
                 (status (plist-get response :status)))
            (cond
             ((eq status 404) (puthash id (list :state 'absent) changes))
             ((and (integerp status) (<= 200 status 299))
              (let ((current (jaunder--operation-member-proof response uri)))
                (puthash id (if current (list :state 'restored :member current :uri uri
                                              :original-condition (or (plist-get change :original-condition)
                                                                      (plist-get change :condition)))
                              (list :state 'invalid :uri uri :detail "targeted Member identity is invalid"
                                    :original-condition (or (plist-get change :original-condition)
                                                            (plist-get change :condition)))) changes)))
             (t (error "jaunder reconcile: targeted Member GET failed (HTTP %s)" status))))
        (error
         (puthash id (list :state 'failed :condition condition
                           :original-condition (or (plist-get change :original-condition)
                                                   (plist-get change :condition))) changes)
         (signal (car condition) (cdr condition)))))
    (let ((current (gethash id changes)))
      (when (eq (plist-get current :state) 'failed)
        (let ((condition (plist-get current :condition))) (signal (car condition) (cdr condition)))))))

(defun jaunder--operation-effective-members (root)
  "Return ROOT's discovery with affected identities replaced or excluded."
  (let* ((members (jaunder--operation-remote-members root))
         (changes (plist-get jaunder--operation-evidence :changes))
         (effective (cl-remove-if (lambda (member) (gethash (jaunder-inventory-member-id member) changes)) members)))
    (maphash (lambda (_id change)
               (when (memq (plist-get change :state) '(valid restored))
                 (push (plist-get change :member) effective))) changes)
    effective))

(defun jaunder--operation-publish-link-members (root locals)
  "Return current remote proof for the known target LOCALS in ROOT.
Unresolved create correlation cannot be established from a local header ID."
  (jaunder--operation-check-scope root)
  (jaunder--operation-remote-members root)
  (dolist (local locals)
    (when (jaunder--operation-unresolved-create-path-p (jaunder-inventory-local-path local))
      (error "jaunder: Local Post Link target create identity is unresolved"))
    (jaunder--operation-restore-member root (jaunder-inventory-local-id local) t))
  ;; Remote acquisition/restoration may change the exact authored target.
  (let* ((members (jaunder--operation-effective-members root))
         (current-locals (jaunder--scan-root-locals root))
         (by-id (jaunder--index-by current-locals #'jaunder-inventory-local-id)))
    (dolist (local locals)
      (let ((current (gethash (jaunder-inventory-local-id local) by-id)))
        (unless (= (length current) 1)
          (error "jaunder: Local Post Link target local identity is ambiguous"))
        (unless (and (equal (jaunder-inventory-local-path (car current))
                            (jaunder-inventory-local-path local))
                     (equal (jaunder-inventory-local-slug (car current))
                            (jaunder-inventory-local-slug local)))
          (error "jaunder: Local Post Link target local identity changed"))))
    (jaunder--operation-check-scope root)
    members))

(provide 'jaunder-reconcile-operation)
;;; jaunder-reconcile-operation.el ends here
