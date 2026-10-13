;;; jaunder-reconcile-batch-fixture.el --- Confirmed batch wire fixture -*- lexical-binding: t; -*-

;; Copyright (C) 2026 Jaunder contributors

;;; Commentary:
;; Real selected commands, temporary Posts, pagination, Media installation and
;; report refresh with controlled HTTP/filesystem boundaries for concurrent edits.

;;; Code:

(require 'jaunder)
(require 'cl-lib)
(load (expand-file-name "jaunder-reconcile-performance-fixture.el"
                        (file-name-directory (or load-file-name buffer-file-name))) nil t)

(defun jaunder-test--batch-entry (id &optional body)
  "Return a complete fixture Member for ID with optional authored BODY."
  (format "<entry xmlns=\"http://www.w3.org/2005/Atom\" xmlns:j=\"https://jaunder.org/ns/atompub\" xmlns:app=\"http://www.w3.org/2007/app\"><title>Remote</title><link rel=\"edit\" href=\"https://example.test/atompub/alice/posts/%d\"/><j:slug>post-%03d</j:slug><content type=\"text/org\">%s</content><app:control><app:draft>yes</app:draft></app:control></entry>"
          id id (or body "Remote body.")))

(defun jaunder-test--confirmed-batch (action hook &optional ids rename-ids)
  "Execute confirmed ACTION for IDS, injecting boundary events through HOOK.
RENAME-IDS have old local slugs, requiring canonical renames during pull.
HOOK receives an event plist and fixture state; its optional plist overrides a
Member HTTP response.  Return terminal results, file bytes and request counts
before cleaning up.  All reconciliation and install logic remains real."
  (let* ((ids (or ids '(1 50 100)))
         (root (file-name-as-directory (make-temp-file "jaunder-batch-proof-" t)))
         (jaunder-blogs (list (cons root '(:base-url "https://example.test" :username "alice"))))
         (etag "\"new\"")
         (instance "12345678-1234-1234-1234-123456789abc")
         (hash (secure-hash 'sha256 ""))
         (entries (make-hash-table))
         (member-reads (make-hash-table))
         (originals (make-hash-table))
         (paths (make-hash-table))
         (pages 0) (initial-pages 0) (operation-pages 0) (refreshing nil) (active nil)
         (real-rename (symbol-function 'rename-file))
         (real-refresh (symbol-function 'jaunder--reconcile-refresh-buffer))
         buffer current-id state rows status)
    (unwind-protect
        (progn
          (setq rows
                (mapcar
                 (lambda (id)
                   (let* ((slug (format "post-%03d" id))
                          (local-slug (if (memq id rename-ids) (concat "old-" slug) slug))
                          (path (expand-file-name (concat local-slug ".org") root))
                          (bytes (format "#+TITLE: Local\n#+PROPERTY: JAUNDER_ID %d\n#+PROPERTY: JAUNDER_SLUG %s\n#+PROPERTY: JAUNDER_SYNCED \"old\"\n\nLocal body.\n" id local-slug)))
                     (puthash id bytes originals)
                     (puthash id path paths)
                     (with-temp-file path (insert bytes))
                     (puthash id (jaunder-test--batch-entry
                                  id (format "Remote body. [[/media/upload/%s/%s/%s/a-%d.bin][Media]]."
                                             (substring hash 0 2) (substring hash 2 4) hash id)) entries)
                     (jaunder--make-reconcile-row
                      :key (format "post:%d" id) :state (if (eq action 'pull) 'server-ahead 'conflict)
                      :remote-etag etag :local-sha256 (jaunder--reconcile-file-sha256 path)
                      :local (jaunder--make-inventory-local :path path :id (number-to-string id) :slug local-slug)
                      :member (jaunder--make-inventory-member
                               :id (number-to-string id) :slug slug
                               :edit-uri (format "https://example.test/atompub/alice/posts/%d" id))))) ids))
          (setq state (list :root root :rows rows :entries entries :originals originals :paths paths))
          (funcall hook '(:phase setup :id 0) state)
          (cl-letf (((symbol-function 'y-or-n-p) (lambda (&rest _) t))
                    ((symbol-function 'plz)
                     (lambda (_method _url &rest _)
                       (make-plz-response :status 200 :body ""
                                          :headers (list (cons 'etag (concat "\"sha256-" hash "\""))
                                                         (cons 'x-jaunder-instance instance)))))
                    ((symbol-function 'rename-file)
                     (lambda (old new &rest args)
                       (prog1 (apply real-rename old new args)
                         (when (and active (string-prefix-p (expand-file-name "local-media/" root) new))
                           (funcall hook (list :phase 'media-installed :id current-id :path new) state)))))
                    ((symbol-function 'jaunder--reconcile-refresh-buffer)
                     (lambda (target)
                       (setq refreshing t operation-pages pages)
                       (funcall real-refresh target)))
                    ((symbol-function 'jaunder--http-request)
                     (lambda (_method url &rest _)
                       (cond
                        ((string-suffix-p "/service" url)
                         (list :status 200 :body "<service xmlns=\"http://www.w3.org/2007/app\" xmlns:atom=\"http://www.w3.org/2005/Atom\"><workspace><atom:title>Blog</atom:title></workspace></service>"))
                        ((string-match "/posts/\\([0-9]+\\)\\'" url)
                         (let* ((id (string-to-number (match-string 1 url)))
                                (count (1+ (gethash id member-reads 0))))
                           (setq current-id id)
                           (puthash id count member-reads)
                           (append (when active
                                     (funcall hook (list :phase 'member :id id :read count) state))
                                   (list :status 200 :headers (list (cons "etag" etag)
                                                                    (cons "x-jaunder-instance" instance))
                                         :body (gethash id entries (jaunder-test--batch-entry id))))))
                        (t
                         (setq pages (1+ pages))
                         (append (when active
                                   (funcall hook (list :phase 'collection :id 0 :url url
                                                       :page pages :refresh refreshing) state))
                                 (list :status 200 :body (jaunder-test--collection-page url "&quot;new&quot;"))))))))
            (setq buffer (jaunder--render-reconcile-report
                          (jaunder--make-reconcile-report
                           :root root :rows rows :inventory (jaunder--inventory-for-root root))
                          (generate-new-buffer " *Jaunder batch proof*")))
            (setq initial-pages pages pages 0 active t)
            (clrhash member-reads)
            (with-current-buffer buffer
              (dolist (row rows)
                (puthash (jaunder--reconcile-stable-row-key row) t jaunder-reconcile-marks))
              (setq status (funcall (if (eq action 'pull) #'jaunder-reconcile-pull-selected
                                      #'jaunder-reconcile-keep-remote-selected)))
              (funcall hook (list :phase 'completed :id 0 :status status) state)
              (list :status status :results jaunder-reconcile-last-batch-results :pages pages
                    :initial-pages initial-pages :operation-pages operation-pages :member-reads member-reads
                    :originals originals
                    :bytes (mapcar (lambda (id)
                                     (let ((path (gethash id paths)))
                                       (when (file-exists-p path)
                                         (with-temp-buffer (insert-file-contents-literally path) (buffer-string))))) ids)))))
      (dolist (visiting (buffer-list))
        (when (and (buffer-file-name visiting)
                   (string-prefix-p root (buffer-file-name visiting)))
          (with-current-buffer visiting (set-buffer-modified-p nil))
          (kill-buffer visiting)))
      (when (buffer-live-p buffer) (kill-buffer buffer))
      (delete-directory root t))))

(provide 'jaunder-reconcile-batch-fixture)
;;; jaunder-reconcile-batch-fixture.el ends here
