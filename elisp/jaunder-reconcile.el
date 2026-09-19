;;; jaunder-reconcile.el --- Post inventory and reconciliation -*- lexical-binding: t; -*-

;; Copyright (C) 2026 Jaunder contributors

;;; Commentary:
;; Enumerate an AtomPub Collection and the directly contained Org files of one
;; configured root.  Inventory is side-effect-free; reconciliation classifies
;; it and renders a persistent selection report without transferring Posts.

;;; Code:

(require 'cl-lib)
(require 'dom)
(require 'url-parse)
(require 'jaunder-config)
(require 'jaunder-org)
(require 'jaunder-transport)
(require 'jaunder-datetime)
(require 'jaunder-publish)

(declare-function jaunder--pull-destination "jaunder-pull")
(declare-function jaunder--pull-destination-exists-p "jaunder-pull")
(declare-function jaunder--pull-member "jaunder-pull")
(declare-function jaunder--pull-stage-member "jaunder-pull")
(declare-function jaunder-pull-result-status "jaunder-pull")
(declare-function jaunder-pull-result-id "jaunder-pull")
(declare-function jaunder-pull-result-slug "jaunder-pull")
(declare-function jaunder-pull-result-etag "jaunder-pull")
(declare-function jaunder-pull-result-synced-at "jaunder-pull")
(declare-function jaunder-pull-result-http-status "jaunder-pull")
(declare-function jaunder-pull-result-local-effect "jaunder-pull")

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
         (slug (dom-inner-text slug-node)))
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
              (signal 'jaunder-inventory-duplicate-remote-id
                      (list (jaunder-inventory-member-id member))))
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
               :path path :id (and raw-id (or (jaunder--canonical-post-id raw-id)
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

(defun jaunder--inventory-for-root (root)
  "Return a side-effect-free inventory of configured ROOT and its Collection."
  (jaunder--call-with-blog
   root
   (lambda ()
     (jaunder--join-inventory (jaunder--scan-root-locals root)
                              (jaunder--fetch-collection-members)))))
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

(define-derived-mode jaunder-reconcile-report-mode special-mode "Jaunder-Reconcile"
  "Major mode for selecting rows in a Jaunder reconciliation report."
  (setq-local truncate-lines t)
  (define-key jaunder-reconcile-report-mode-map "m" #'jaunder-reconcile-toggle-mark)
  (define-key jaunder-reconcile-report-mode-map "p" #'jaunder-reconcile-push-selected)
  (define-key jaunder-reconcile-report-mode-map "f" #'jaunder-reconcile-pull-selected)
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
               (let ((change-major-mode-hook nil) (after-change-major-mode-hook nil))
                 (delay-mode-hooks (org-mode)))
               (list (jaunder--buffer-property "JAUNDER_SYNCED")
                     (jaunder--buffer-property "JAUNDER_SYNCED_AT")
                     (jaunder--buffer-property "JAUNDER_LOCAL_AHEAD")))
           (error (list nil nil nil)))))
    (append markers
            (list
             (condition-case nil
                 (file-attribute-modification-time
                  (file-attributes (jaunder-inventory-local-path local)))
               (error nil))))))

(defun jaunder--reconcile-member-outcome (member)
  "Fetch MEMBER once, retaining a transport failure as row-local data."
  (condition-case err
      (list :response (jaunder--http-request
                       "GET" (jaunder-inventory-member-edit-uri member)))
    (error (list :error err))))

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
  (when (jaunder-reconcile-result-reason result)
    (insert (format "; %s" (jaunder-reconcile-result-reason result)))
    (when (jaunder-reconcile-result-detail result)
      (insert (format " (%s)" (jaunder-reconcile-result-detail result)))))
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
                  (goto-char
                   (min (save-excursion
                          (goto-char row-position)
                          (line-end-position))
                        (+ row-position column)))
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

(defun jaunder--reconcile-refresh-batch-buffer (buffer)
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
           (marks (jaunder--reconcile-pruned-marks report jaunder-reconcile-marks)))
      (setq-local jaunder-reconcile-marks marks)
      (jaunder--render-reconcile-report report buffer))))

(defun jaunder-reconcile-refresh ()
  "Refresh the current reconciliation report from local and remote state."
  (interactive)
  (jaunder--reconcile-refresh-batch-buffer (current-buffer)))

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
    (jaunder--reconcile-refresh-batch-buffer buffer)
    (if cancelled 'cancelled 'completed)))

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

(defun jaunder--reconcile-local-mutation-safety-reason (row)
  "Return a stale or modified local safety reason immediately before mutation."
  (let* ((local (jaunder-reconcile-row-local row))
         (path (and local (jaunder-inventory-local-path local)))
         (expected (if (eq (jaunder-reconcile-row-state row) 'local-draft)
                       nil (jaunder--reconcile-row-post-id row))))
    (when local
      (with-current-buffer (find-file-noselect path)
        (cond
         ((buffer-modified-p) 'local-buffer-modified)
         ((not (equal (jaunder--canonical-post-id
                       (jaunder--buffer-property "JAUNDER_ID")) expected))
          (if expected 'matched-identity-changed 'draft-identity-changed))
         ((not (equal (jaunder--reconcile-current-local-id row) expected))
          (if expected 'matched-identity-changed 'draft-identity-changed)))))))

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
         (with-current-buffer (find-file-noselect path)
           (let ((published (jaunder-publish)))
             (list :outcome 'success
                   :post-id (jaunder--buffer-property "JAUNDER_ID")
                   :slug (jaunder--buffer-property "JAUNDER_SLUG")
                   :etag (jaunder--buffer-property "JAUNDER_SYNCED")
                   :synced-at (jaunder--buffer-property "JAUNDER_SYNCED_AT")
                   :http-status (plist-get published :http-status)
                   :local-effect (if (eq (jaunder-reconcile-row-state row) 'local-draft)
                                     'created 'updated))))))))
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

(defun jaunder--reconcile-pull-install-staged (row staged remote path)
  "Install STAGED ROW bytes after successful REMOTE revalidation at PATH."
  (let ((preflight (jaunder--reconcile-pull-preflight row staged)))
    (if preflight
        (jaunder--reconcile-blocked row preflight)
      (let* ((destination (jaunder--reconcile-pull-destination row (plist-get staged :slug)))
             (installed (jaunder--reconcile-replace-pulled-file
                         path destination (plist-get staged :bytes)))
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
          (let* ((staged (jaunder--pull-stage-member
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
       report))))



(provide 'jaunder-reconcile)
;;; jaunder-reconcile.el ends here
