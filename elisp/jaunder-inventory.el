;;; jaunder-inventory.el --- Collection and local Post inventory -*- lexical-binding: t; -*-

;; Copyright (C) 2026 Jaunder contributors

;;; Commentary:
;; The read-only Member and local Post inventory contract shared by reconciliation
;; and Local Post Link preflight.  It owns Collection harvesting, local evidence,
;; and total inventory joining; callers own their respective mutations.

;;; Code:

(require 'cl-lib)
(require 'dom)
(require 'url-parse)
(require 'jaunder-atom)
(require 'jaunder-config)
(require 'jaunder-org)
(require 'jaunder-transport)
(require 'jaunder-datetime)

(cl-defstruct (jaunder-inventory-member
               (:constructor jaunder--make-inventory-member))
  "One Post advertised by an AtomPub Collection."
  id slug edit-uri alternate-href alternate-invalid-reason)

(cl-defstruct (jaunder-inventory-local
               (:constructor jaunder--make-inventory-local))
  "One root-level local Org file."
  path id slug)

(cl-defstruct (jaunder-inventory-match
               (:constructor jaunder--make-inventory-match))
  "A unique local/server pair with the same Post ID."
  local member)

(cl-defstruct (jaunder-inventory-conflict
               (:constructor jaunder--make-inventory-conflict))
  "A connected set of inventory inputs requiring human resolution."
  kinds locals members)

(cl-defstruct (jaunder-inventory
               (:constructor jaunder--make-inventory))
  "The exhaustive partition of one root and one Collection."
  local-drafts server-only matched orphans conflicts)


(define-error 'jaunder-inventory-duplicate-remote-id
              "Collection contains duplicate Post ID" 'error)

(defun jaunder--inventory-error (invariant)
  "Signal an inventory error naming broken INVARIANT without response details."
  (error "jaunder inventory: %s" invariant))

(defun jaunder--parse-collection-xml (xml)
  "Parse Collection XML, naming malformed wire data without echoing it."
  (condition-case nil
      (with-temp-buffer
        (insert xml)
        (libxml-parse-xml-region (point-min) (point-max)))
    (error (jaunder--inventory-error "malformed Collection XML"))))

(defun jaunder--parse-collection-xml-namespaced (xml)
  "Parse Collection XML with namespace declarations retained for alternate links."
  (condition-case nil
      (with-temp-buffer
        (insert xml)
        (car (xml-parse-region (point-min) (point-max))))
    (error (jaunder--inventory-error "malformed Collection XML"))))

(defun jaunder--direct-elements (node tag)
  "Return NODE's direct child elements named TAG, in document order."
  (cl-remove-if-not (lambda (child) (and (listp child) (eq (car child) tag)))
                    (dom-children node)))

(defun jaunder--single-element (elements invariant)
  "Return the sole item in ELEMENTS or signal INVARIANT."
  (if (= (length elements) 1)
      (car elements)
    (jaunder--inventory-error invariant)))

(defun jaunder--collection-edit-id (href collection-url)
  "Extract the canonical Post ID from HREF below COLLECTION-URL's exact path."
  (when (and (stringp href) (stringp collection-url))
    (let* ((collection (url-generic-parse-url collection-url))
           (edit (url-generic-parse-url href))
           (collection-path (url-filename collection))
           (edit-path (url-filename edit)))
      (when (and (stringp collection-path) (stringp edit-path)
                 (string-match
                  (concat "\\`" (regexp-quote collection-path) "/\\([0-9]+\\)\\'")
                  edit-path))
        (let ((id (match-string 1 edit-path)))
          (and (equal id (jaunder--canonical-post-id id)) id))))))

(defun jaunder--inventory-same-origin-p (candidate origin)
  "Return non-nil when parsed CANDIDATE has ORIGIN's HTTP(S) origin."
  (and (member (downcase (or (url-type candidate) "")) '("http" "https"))
       (equal (downcase (or (url-type candidate) ""))
              (downcase (or (url-type origin) "")))
       (equal (downcase (or (url-host candidate) ""))
              (downcase (or (url-host origin) "")))
       (= (url-port candidate) (url-port origin))))

(defun jaunder--inventory-url-syntax-valid-p (href)
  "Return non-nil when HREF has valid URI characters, authority, and escapes."
  (when (and (stringp href)
             (not (string-match-p "[[:space:][:cntrl:]<>\"{}|\\\\^`]" href))
             (not (string-search
                   "%" (replace-regexp-in-string
                        "%[[:xdigit:]][[:xdigit:]]" "" href t t)))
             (string-match "\\`[[:alpha:]][[:alnum:]+.-]*://\\([^/?#]*\\)" href))
    (let* ((authority (match-string 1 href))
           (host-port (if (string-match ".*@" authority)
                          (substring authority (match-end 0))
                        authority))
           (port (and (string-match ":\\([0-9]+\\)\\'" host-port)
                      (string-to-number (match-string 1 host-port)))))
      (and (if (string-prefix-p "[" host-port)
               (string-match-p "\\`\\[[^][]+\\]\\(?::[0-9]+\\)?\\'" host-port)
             (string-match-p "\\`[^:]+\\(?::[0-9]+\\)?\\'" host-port))
           (or (null port) (<= port 65535))))))

(defun jaunder--inventory-alternate-outcome (links collection-url)
  "Return the authoritative alternate outcome for LINKS at COLLECTION-URL.
The result is (HREF REASON), where exactly one member is non-nil."
  (cond
   ((null links) (list nil 'alternate-missing))
   ((cdr links) (list nil 'alternate-duplicate))
   (t
    (let* ((href (dom-attr (car links) 'href))
           (candidate (and (stringp href)
                           (condition-case nil (url-generic-parse-url href) (error nil))))
           (origin (condition-case nil
                       (url-generic-parse-url collection-url)
                     (error nil))))
      (cond
       ((not (and candidate (url-type candidate) (url-host candidate)
                  (jaunder--inventory-url-syntax-valid-p href)))
        (list nil 'alternate-malformed))
       ((or (url-user candidate) (url-password candidate))
        (list nil 'alternate-user-info))
       ((or (string-search "?" href) (string-search "#" href))
        (list nil 'alternate-query-or-fragment))
       ((not (jaunder--inventory-same-origin-p candidate origin))
        (list nil 'alternate-cross-origin))
       (t (list href nil)))))))

(defun jaunder--parse-collection-member
    (entry collection-url &optional alternate-entry inherited-namespaces)
  "Parse one Collection ENTRY beneath COLLECTION-URL into an inventory Member.
ALTERNATE-ENTRY and INHERITED-NAMESPACES retain the Atom namespace context.
ENTRY itself uses the established libxml direct-child parsing for Member fields."
  (let* ((alternate-entry (or alternate-entry entry))
         (entry-namespaces
          (jaunder--atom-namespace-context alternate-entry inherited-namespaces))
         (edit (jaunder--single-element
                (cl-remove-if-not
                 (lambda (link) (equal (dom-attr link 'rel) "edit"))
                 (jaunder--direct-elements entry 'link))
                "Member must have exactly one rel=edit link"))
         (href (dom-attr edit 'href))
         (id (jaunder--collection-edit-id href collection-url))
         (slug-node (jaunder--single-element (jaunder--direct-elements entry 'slug)
                                             "Member must have exactly one j:slug"))
         (slug (dom-inner-text slug-node))
         (alternate (jaunder--inventory-alternate-outcome
                     (cl-remove-if-not
                      (lambda (link) (equal (dom-attr link 'rel) "alternate"))
                      (jaunder--atom-direct-elements-in-namespace
                       alternate-entry 'link jaunder--atom-ns entry-namespaces))
                     collection-url)))
    (unless id
      (jaunder--inventory-error "Member edit URI must name a decimal Post ID"))
    (unless (and (stringp slug) (not (string= slug "")))
      (jaunder--inventory-error "Member j:slug must be non-empty"))
    (jaunder--make-inventory-member
     :id id :slug slug :edit-uri href
     :alternate-href (car alternate) :alternate-invalid-reason (cadr alternate))))

(defun jaunder--parse-collection-page (xml collection-url)
  "Parse Collection XML beneath COLLECTION-URL into (:members MEMBERS :next URI).
Signals on malformed page-level or Member invariants; no partial page is
returned."
  (let ((feed (jaunder--parse-collection-xml xml))
        (namespaced-feed (jaunder--parse-collection-xml-namespaced xml)))
    (unless (and (eq (car feed) 'feed) (eq (car namespaced-feed) 'feed))
      (jaunder--inventory-error "Collection document must have a feed root"))
    (let* ((feed-namespaces (jaunder--atom-namespace-context namespaced-feed nil))
           (next-links (cl-remove-if-not
                        (lambda (link) (equal (dom-attr link 'rel) "next"))
                        (jaunder--direct-elements feed 'link))))
      (when (> (length next-links) 1)
        (jaunder--inventory-error "Collection page has multiple rel=next links"))
      (let ((next (when next-links (dom-attr (car next-links) 'href))))
        (when (and next (or (not (stringp next)) (string= next "")))
          (jaunder--inventory-error "Collection rel=next URI must be non-empty"))
        (let ((entries (jaunder--direct-elements feed 'entry))
              (alternate-entries (jaunder--direct-elements namespaced-feed 'entry)))
          (unless (= (length entries) (length alternate-entries))
            (jaunder--inventory-error "Collection Member namespace parse mismatch"))
          (list :members (cl-mapcar
                          (lambda (entry alternate-entry)
                            (jaunder--parse-collection-member
                             entry collection-url alternate-entry feed-namespaces))
                          entries alternate-entries)
                :next next))))))


(defun jaunder--collection-url ()
  "Return the active blog's Posts Collection URL."
  (jaunder--build-url (jaunder--active-base-url) "atompub"
                      (jaunder--active-username) "posts"))

(defun jaunder--fetch-collection-members ()
  "Enumerate the active blog's Collection, preserving page and Entry order."
  (let* ((collection-url (jaunder--collection-url))
         (url collection-url)
         (seen (make-hash-table :test #'equal))
         (ids (make-hash-table :test #'equal))
         members)
    (while url
      (when (gethash url seen)
        (jaunder--inventory-error "Collection rel=next URI cycle"))
      (puthash url t seen)
      (let ((response (jaunder--http-request "GET" url)))
        (unless (and (integerp (plist-get response :status))
                     (<= 200 (plist-get response :status) 299))
          (jaunder--inventory-error "Collection page returned non-2xx status"))
        (let ((page (jaunder--parse-collection-page
                     (plist-get response :body) collection-url)))
          (dolist (member (plist-get page :members))
            (when (gethash (jaunder-inventory-member-id member) ids)
              (signal 'jaunder-inventory-duplicate-remote-id
                      (list (jaunder-inventory-member-id member))))
            (puthash (jaunder-inventory-member-id member) t ids))
          (setq members (nconc members (plist-get page :members))
                url (plist-get page :next)))))
    members))

(defun jaunder--read-local-properties (path)
  "Read PATH's Post ID and slug through the shared Org property reader."
  (with-temp-buffer
    (insert-file-contents path)
    ;; Delay mode-specific hooks and suppress the generic hooks which run
    ;; immediately; this temporary buffer must not execute user configuration.
    (let ((change-major-mode-hook nil)
          (after-change-major-mode-hook nil))
      (delay-mode-hooks (org-mode)))
    (list (jaunder--buffer-property "JAUNDER_ID")
          (jaunder--buffer-property "JAUNDER_SLUG"))))

(defun jaunder--read-local-id (path)
  "Read PATH's Post ID through the shared local property reader."
  (car (jaunder--read-local-properties path)))

(defun jaunder--scan-root-locals (root)
  "Return regular root-level .org files under ROOT in deterministic order."
  (mapcar (lambda (path)
            (pcase-let ((`(,raw-id ,slug) (jaunder--read-local-properties path)))
              (jaunder--make-inventory-local
               :path path :id (and raw-id (or (jaunder--canonical-post-id raw-id)
                                              raw-id))
               :slug slug)))
          (cl-remove-if-not #'file-regular-p
                            (directory-files (expand-file-name root) t "\\.org\\'"))))

(defun jaunder--inventory-local-member-evidence-reason (local member)
  "Return LOCAL's first failed identity proof against MEMBER, or nil.
A local filename is evidence only after the Post ID and slug agree."
  (cond
   ((not (equal (jaunder-inventory-local-id local)
                (jaunder-inventory-member-id member)))
    'local-id-mismatch)
   ((not (equal (jaunder-inventory-local-slug local)
                (jaunder-inventory-member-slug member)))
    'local-slug-mismatch)
   ((not (equal (file-name-nondirectory (jaunder-inventory-local-path local))
                (concat (jaunder-inventory-member-slug member) ".org")))
    'local-filename-mismatch)))

(defun jaunder--inventory-node (kind value)
  "Return a tagged inventory graph node of KIND holding VALUE."
  (cons kind value))

(defun jaunder--node-kind (node)
  "Return NODE's inventory graph kind."
  (car node))

(defun jaunder--node-value (node)
  "Return NODE's inventory graph value."
  (cdr node))

(defun jaunder--node-id (node)
  "Return NODE's canonical join ID, or nil when its local ID is invalid."
  (pcase (jaunder--node-kind node)
    ('local (jaunder--canonical-post-id
             (jaunder-inventory-local-id (jaunder--node-value node))))
    ('member (jaunder-inventory-member-id (jaunder--node-value node)))))

(defun jaunder--index-by (items key)
  "Index ITEMS by KEY once, preserving their source order within each bucket."
  (let ((index (make-hash-table :test #'equal)))
    (dolist (item items)
      (let ((value (funcall key item)))
        (when value
          (puthash value (cons item (gethash value index)) index))))
    (maphash (lambda (value bucket) (puthash value (nreverse bucket) index)) index)
    index))

(defun jaunder--indexed-nodes (kind values)
  "Return graph nodes of KIND for VALUES."
  (mapcar (lambda (value) (jaunder--inventory-node kind value)) values))

(defun jaunder--conflict-seeds (locals members local-id-index member-slug-index)
  "Return deterministically ordered graph seeds from indexed duplicate inputs."
  (append
   (delq nil
         (mapcar (lambda (local)
                   (let ((id (jaunder-inventory-local-id local)))
                     (unless (or (null id) (jaunder--canonical-post-id id))
                       (jaunder--inventory-node 'local local))))
                 locals))
   (delq nil
         (mapcar (lambda (local)
                   (let* ((id (jaunder--canonical-post-id
                               (jaunder-inventory-local-id local)))
                          (bucket (and id (gethash id local-id-index))))
                     (when (and bucket (eq local (car bucket)) (cdr bucket))
                       (jaunder--inventory-node 'local local))))
                 locals))
   (delq nil
         (mapcar (lambda (member)
                   (let ((bucket (gethash (jaunder-inventory-member-slug member)
                                          member-slug-index)))
                     (when (and (eq member (car bucket)) (cdr bucket))
                       (jaunder--inventory-node 'member member))))
                 members))))

(defun jaunder--conflict-neighbors
    (node local-id-index member-id-index member-slug-index expanded-ids expanded-slugs)
  "Return NODE's unexpanded indexed conflict graph neighbors."
  (let ((id (jaunder--node-id node)) id-neighbors slug-neighbors)
    (when (and id (not (gethash id expanded-ids)))
      (puthash id t expanded-ids)
      (setq id-neighbors
            (append
             (jaunder--indexed-nodes 'local (gethash id local-id-index))
             (jaunder--indexed-nodes 'member (gethash id member-id-index)))))
    (when (and (eq (jaunder--node-kind node) 'member)
               (let ((slug (jaunder-inventory-member-slug (jaunder--node-value node))))
                 (unless (gethash slug expanded-slugs)
                   (puthash slug t expanded-slugs)
                   (setq slug-neighbors
                         (jaunder--indexed-nodes 'member
                                                 (gethash slug member-slug-index))))))
      slug-neighbors)
    (append id-neighbors slug-neighbors)))

(defun jaunder--conflict-component
    (seed visited local-id-index member-id-index member-slug-index)
  "Traverse the indexed conflict component starting at SEED."
  (let ((pending (list seed))
        (expanded-ids (make-hash-table :test #'equal))
        (expanded-slugs (make-hash-table :test #'equal))
        nodes)
    (while pending
      (let ((node (pop pending)))
        (unless (gethash (jaunder--node-value node) visited)
          (puthash (jaunder--node-value node) t visited)
          (push node nodes)
          (setq pending
                (nconc
                 (jaunder--conflict-neighbors
                  node local-id-index member-id-index member-slug-index
                  expanded-ids expanded-slugs)
                 pending)))))
    nodes))

(defun jaunder--conflict-kinds (nodes local-id-index member-slug-index)
  "Return the conflict kinds present in indexed graph NODES."
  (let (kinds)
    (when (cl-some
           (lambda (node)
             (and (eq (jaunder--node-kind node) 'local)
                  (let ((id (jaunder-inventory-local-id (jaunder--node-value node))))
                    (and id (not (jaunder--canonical-post-id id))))))
           nodes)
      (push 'invalid-local-id kinds))
    (when (cl-some
           (lambda (node)
             (and (eq (jaunder--node-kind node) 'local)
                  (let ((bucket (gethash (jaunder--node-id node) local-id-index)))
                    (cdr bucket))))
           nodes)
      (push 'duplicate-local-id kinds))
    (when (cl-some
           (lambda (node)
             (and (eq (jaunder--node-kind node) 'member)
                  (cdr (gethash
                        (jaunder-inventory-member-slug (jaunder--node-value node))
                        member-slug-index))))
           nodes)
      (push 'duplicate-target-slug kinds))
    (nreverse kinds)))

(defun jaunder--conflict-groups (locals members)
  "Build disjoint connected conflict groups from indexed LOCALS and MEMBERS."
  (let* ((local-id-index
          (jaunder--index-by locals
                             (lambda (local)
                               (jaunder--canonical-post-id
                                (jaunder-inventory-local-id local)))))
         (member-id-index (jaunder--index-by members #'jaunder-inventory-member-id))
         (member-slug-index (jaunder--index-by members #'jaunder-inventory-member-slug))
         (visited (make-hash-table :test #'eq))
         groups)
    (dolist (seed (jaunder--conflict-seeds
                   locals members local-id-index member-slug-index)
                  (nreverse groups))
      (unless (gethash (jaunder--node-value seed) visited)
        (let ((nodes (jaunder--conflict-component
                      seed visited local-id-index member-id-index member-slug-index)))
          (push (list :nodes nodes
                      :kinds (jaunder--conflict-kinds
                              nodes local-id-index member-slug-index))
                groups))))))

(defun jaunder--conflict-owned-table (groups)
  "Return an identity table for every local and Member owned by GROUPS."
  (let ((owned (make-hash-table :test #'eq)))
    (dolist (group groups owned)
      (dolist (node (plist-get group :nodes))
        (puthash (jaunder--node-value node) t owned)))))

(defun jaunder--join-inventory (locals members)
  "Join LOCALS and MEMBERS into a deterministic total `jaunder-inventory'."
  (let* ((groups (jaunder--conflict-groups locals members))
         (owned (jaunder--conflict-owned-table groups))
         (available-locals (cl-remove-if (lambda (local) (gethash local owned)) locals))
         (available-members (cl-remove-if (lambda (member) (gethash member owned)) members))
         (local-id-index
          (jaunder--index-by available-locals
                             (lambda (local)
                               (jaunder--canonical-post-id
                                (jaunder-inventory-local-id local)))))
         (member-id-index
          (jaunder--index-by available-members #'jaunder-inventory-member-id))
         matches orphans server-only)
    (dolist (local available-locals)
      (let ((id (jaunder--canonical-post-id (jaunder-inventory-local-id local))))
        (cond
         ((null (jaunder-inventory-local-id local)))
         ((gethash id member-id-index)
          (push (jaunder--make-inventory-match
                 :local local :member (car (gethash id member-id-index)))
                matches))
         (id (push local orphans)))))
    (dolist (member available-members)
      (unless (gethash (jaunder-inventory-member-id member) local-id-index)
        (push member server-only)))
    (jaunder--make-inventory
     :local-drafts (cl-remove-if (lambda (local) (gethash local owned))
                                 (cl-remove-if-not
                                  (lambda (local) (null (jaunder-inventory-local-id local)))
                                  locals))
     :server-only (nreverse server-only)
     :matched (nreverse matches)
     :orphans (nreverse orphans)
     :conflicts
     (mapcar
      (lambda (group)
        (let ((group-owned (make-hash-table :test #'eq)))
          (dolist (node (plist-get group :nodes))
            (puthash (jaunder--node-value node) t group-owned))
          (jaunder--make-inventory-conflict
           :kinds (plist-get group :kinds)
           :locals (cl-remove-if-not (lambda (local) (gethash local group-owned)) locals)
           :members (cl-remove-if-not (lambda (member) (gethash member group-owned))
                                      members))))
      groups))))

(defun jaunder--inventory-post-link-evidence (inventory)
  "Return unconflicted (MEMBERS LOCALS) from INVENTORY for Post link mapping."
  (list (append (jaunder-inventory-server-only inventory)
                (mapcar #'jaunder-inventory-match-member
                        (jaunder-inventory-matched inventory)))
        (append (jaunder-inventory-local-drafts inventory)
                (jaunder-inventory-orphans inventory)
                (mapcar #'jaunder-inventory-match-local
                        (jaunder-inventory-matched inventory)))))

(defun jaunder--inventory-for-root (root)
  "Return a side-effect-free inventory of configured ROOT and its Collection."
  (jaunder--call-with-blog
   root
   (lambda ()
     (jaunder--join-inventory (jaunder--scan-root-locals root)
                              (jaunder--fetch-collection-members)))))

(provide 'jaunder-inventory)
;;; jaunder-inventory.el ends here
