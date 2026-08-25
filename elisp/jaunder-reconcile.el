;;; jaunder-reconcile.el --- Side-effect-free Post inventory -*- lexical-binding: t; -*-

;; Copyright (C) 2026 Jaunder contributors

;;; Commentary:
;; Enumerate an AtomPub Collection and the directly contained Org files of one
;; configured root.  The result is a complete, conflict-safe inventory for the
;; reconcile UI; this module never mutates either the filesystem or the server.

;;; Code:

(require 'cl-lib)
(require 'dom)
(require 'url-parse)
(require 'jaunder-config)
(require 'jaunder-org)
(require 'jaunder-transport)

(cl-defstruct (jaunder-inventory-member
               (:constructor jaunder--make-inventory-member))
              "One Post advertised by an AtomPub Collection."
              id slug edit-uri)

(cl-defstruct (jaunder-inventory-local
               (:constructor jaunder--make-inventory-local))
              "One root-level local Org file."
              path id)

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

(defun jaunder--canonical-id (value)
  "Return VALUE as a canonical decimal ID string, or nil when malformed."
  (when (and (stringp value) (string-match-p "\\`[0-9]+\\'" value))
    (replace-regexp-in-string "\\`0+" "" value)))

(defun jaunder--canonical-id-or-zero (value)
  "Return canonical decimal VALUE, retaining zero as the string `0'."
  (let ((id (jaunder--canonical-id value)))
    (and id (if (string= id "") "0" id))))

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
          (and (equal id (jaunder--canonical-id-or-zero id)) id))))))

(defun jaunder--parse-collection-member (entry collection-url)
  "Parse one Collection ENTRY beneath COLLECTION-URL into an inventory Member."
  (let* ((edit (jaunder--single-element
                (cl-remove-if-not (lambda (link) (equal (dom-attr link 'rel) "edit"))
                                  (jaunder--direct-elements entry 'link))
                "Member must have exactly one rel=edit link"))
         (href (dom-attr edit 'href))
         (id (jaunder--collection-edit-id href collection-url))
         (slug-node (jaunder--single-element (jaunder--direct-elements entry 'slug)
                                             "Member must have exactly one j:slug"))
         (slug (dom-text slug-node)))
    (unless id
      (jaunder--inventory-error "Member edit URI must name a decimal Post ID"))
    (unless (and (stringp slug) (not (string= slug "")))
      (jaunder--inventory-error "Member j:slug must be non-empty"))
    (jaunder--make-inventory-member :id id :slug slug :edit-uri href)))

(defun jaunder--parse-collection-page (xml collection-url)
  "Parse Collection XML beneath COLLECTION-URL into (:members MEMBERS :next URI).
Signals on malformed page-level or Member invariants; no partial page is
returned."
  (let ((feed (jaunder--parse-collection-xml xml)))
    (unless (eq (car feed) 'feed)
      (jaunder--inventory-error "Collection document must have a feed root"))
    (let ((next-links (cl-remove-if-not
                       (lambda (link) (equal (dom-attr link 'rel) "next"))
                       (jaunder--direct-elements feed 'link))))
      (when (> (length next-links) 1)
        (jaunder--inventory-error "Collection page has multiple rel=next links"))
      (let ((next (when next-links (dom-attr (car next-links) 'href))))
        (when (and next (or (not (stringp next)) (string= next "")))
          (jaunder--inventory-error "Collection rel=next URI must be non-empty"))
        (list :members (mapcar (lambda (entry)
                                 (jaunder--parse-collection-member entry collection-url))
                               (jaunder--direct-elements feed 'entry))
              :next next)))))


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
              (jaunder--inventory-error "Collection contains duplicate Post ID"))
            (puthash (jaunder-inventory-member-id member) t ids))
          (setq members (nconc members (plist-get page :members))
                url (plist-get page :next)))))
    members))

(defun jaunder--read-local-id (path)
  "Read PATH's `JAUNDER_ID' through the shared Org property reader."
  (with-temp-buffer
    (insert-file-contents path)
    ;; Delay mode-specific hooks and suppress the generic hooks which run
    ;; immediately; this temporary buffer must not execute user configuration.
    (let ((change-major-mode-hook nil)
          (after-change-major-mode-hook nil))
      (delay-mode-hooks (org-mode)))
    (jaunder--buffer-property "JAUNDER_ID")))

(defun jaunder--scan-root-locals (root)
  "Return regular root-level .org files under ROOT in deterministic order."
  (mapcar (lambda (path)
            (let ((raw-id (jaunder--read-local-id path)))
              (jaunder--make-inventory-local
               :path path :id (and raw-id (or (jaunder--canonical-id-or-zero raw-id)
                                              raw-id)))))
          (cl-remove-if-not #'file-regular-p
                            (directory-files (expand-file-name root) t "\\.org\\'"))))

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
    ('local (jaunder--canonical-id-or-zero
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
                     (unless (or (null id) (jaunder--canonical-id-or-zero id))
                       (jaunder--inventory-node 'local local))))
                 locals))
   (delq nil
         (mapcar (lambda (local)
                   (let* ((id (jaunder--canonical-id-or-zero
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
                    (and id (not (jaunder--canonical-id-or-zero id))))))
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
                               (jaunder--canonical-id-or-zero
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
                               (jaunder--canonical-id-or-zero
                                (jaunder-inventory-local-id local)))))
         (member-id-index
          (jaunder--index-by available-members #'jaunder-inventory-member-id))
         matches orphans server-only)
    (dolist (local available-locals)
      (let ((id (jaunder--canonical-id-or-zero (jaunder-inventory-local-id local))))
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

(defun jaunder--inventory-for-root (root)
  "Return a side-effect-free inventory of configured ROOT and its Collection."
  (jaunder--with-blog root
                      (jaunder--join-inventory (jaunder--scan-root-locals root)
                                               (jaunder--fetch-collection-members))))

(provide 'jaunder-reconcile)
;;; jaunder-reconcile.el ends here
