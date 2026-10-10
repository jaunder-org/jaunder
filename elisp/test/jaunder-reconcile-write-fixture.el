;;; jaunder-reconcile-write-fixture.el --- Confirmed write wire fixture -*- lexical-binding: t; -*-

;; Copyright (C) 2026 Jaunder contributors

;;; Commentary:
;; Public confirmed commands use real publishing, local files, conditional sends,
;; write-back, renames and report refresh against a deterministic paginated wire.

;;; Code:

(require 'jaunder)
(require 'cl-lib)

(defun jaunder-test--write-entry (id slug href &optional duplicate-alternate)
  "Return native Member XML for ID, SLUG and HREF."
  (format "<entry xmlns=\"http://www.w3.org/2005/Atom\" xmlns:j=\"https://jaunder.org/ns/atompub\" xmlns:app=\"http://www.w3.org/2007/app\"><title>Remote</title><link rel=\"edit\" href=\"https://example.test/atompub/alice/posts/%s\"/><link rel=\"alternate\" href=\"%s\"/>%s<j:slug>%s</j:slug><content type=\"text/org\">Remote body.</content><app:control><app:draft>yes</app:draft></app:control></entry>"
          id href (if duplicate-alternate (format "<link rel=\"alternate\" href=\"%s\"/>" href) "") slug))

(defun jaunder-test--confirmed-write-batch (action hook &optional ids links)
  "Run public ACTION for IDS with LINKS and event HOOK, returning observed proof.
HOOK receives an event plist and state and may override an HTTP response."
  (let* ((ids (or ids '(1 50 100)))
         (root (file-name-as-directory (make-temp-file "jaunder-write-proof-" t)))
         (jaunder-blogs (list (cons root '(:base-url "https://example.test" :username "alice"))))
         (members (make-hash-table :test #'equal))
         (paths (make-hash-table :test #'equal))
         (originals (make-hash-table :test #'equal))
         (reads (make-hash-table :test #'equal))
         (create-ids (make-hash-table :test #'equal))
         (writes nil) (pages 0) (initial-pages 0) operation-pages refreshing active
         (real-refresh (symbol-function 'jaunder--reconcile-refresh-buffer))
         rows buffer state status)
    (unwind-protect
        (progn
          (dotimes (n 100)
            (let* ((id (number-to-string (1+ n))) (slug (format "post-%03d" (1+ n))))
              (puthash id (list :slug slug :href (format "https://example.test/~alice/%s" slug)
                                :etag "\"old\"") members)))
          (dolist (n (delete-dups (cons 25 (copy-sequence ids))))
            (let* ((id (number-to-string n)) (slug (format "post-%03d" n))
                   (path (expand-file-name (concat slug ".org") root))
                   (bytes (format "#+TITLE: Local\n#+PROPERTY: JAUNDER_ID %s\n#+PROPERTY: JAUNDER_SLUG %s\n#+PROPERTY: JAUNDER_SYNCED \"old\"\n#+PROPERTY: JAUNDER_DATE_TZ UTC\n#+PROPERTY: JAUNDER_STATUS draft\n\nAuthored body.%s\n"
                                  id slug (if links " [[file:./post-025.org][Target]]" ""))))
              (with-temp-file path (insert bytes))
              (puthash id path paths) (puthash id bytes originals)
              (when (memq n ids)
                (push (jaunder--make-reconcile-row
                       :key (concat "post:" id)
                       :state (pcase action ('push 'local-ahead) ('delete 'unchanged) (_ 'conflict))
                       :remote-etag "\"old\""
                       :local (jaunder--make-inventory-local :id id :slug slug :path path)
                       :member (jaunder--make-inventory-member
                                :id id :slug slug :edit-uri (concat "https://example.test/atompub/alice/posts/" id))) rows))))
          (setq rows (nreverse rows) state (list :root root :rows rows :members members :paths paths :originals originals))
          (funcall hook '(:phase setup) state)
          (dolist (row rows)
            (setf (jaunder-reconcile-row-local-sha256 row)
                  (jaunder--reconcile-file-sha256 (jaunder-inventory-local-path (jaunder-reconcile-row-local row)))))
          (cl-letf (((symbol-function 'y-or-n-p) (lambda (&rest _) t))
                    ((symbol-function 'sleep-for) (lambda (&rest _) nil))
                    ((symbol-function 'jaunder--reconcile-refresh-buffer)
                     (lambda (target)
                       (setq operation-pages pages refreshing t)
                       (funcall real-refresh target)))
                    ((symbol-function 'jaunder--http-request)
                     (lambda (method url &optional xml _content-type headers)
                       (cond
                        ((string-suffix-p "/service" url)
                         '(:status 200 :body "<service xmlns=\"http://www.w3.org/2007/app\" xmlns:atom=\"http://www.w3.org/2005/Atom\"><workspace><atom:title>User</atom:title></workspace></service>"))
                        ((not (equal method "GET"))
                         (let* ((key (cdr (assoc "Idempotency-Key" headers)))
                                (id (if (equal method "POST")
                                        (or (gethash key create-ids)
                                            (puthash key (number-to-string (+ 101 (hash-table-count create-ids))) create-ids))
                                      (car (last (split-string url "/" t)))))
                                (current (gethash id members))
                                (slug (or (plist-get current :slug)
                                          (if (equal id "101") "created" (concat "created-" id))))
                                (href (or (plist-get current :href) (concat "https://example.test/~alice/" slug)))
                                (event (list :phase 'write :method method :id id :url url :xml xml :headers headers)))
                           (push event writes)
                           (if (equal method "DELETE") (remhash id members)
                             (puthash id (list :slug slug :href href :etag "\"written\"") members))
                           (let ((override (funcall hook event state)))
                             (when (plist-get override :no-commit)
                               (if current (puthash id current members) (remhash id members)))
                             (append override
                                     (list :status (if (equal method "DELETE") 204 (if (equal method "POST") 201 200))
                                           :headers (list (cons "etag" "\"written\"")
                                                          (cons "location" (concat "https://example.test/atompub/alice/posts/" id)))
                                           :body (jaunder-test--write-entry id slug href))))))
                        ((string-match "/posts/\\([0-9]+\\)\\'" url)
                         (let* ((id (match-string 1 url)) (current (gethash id members))
                                (count (1+ (gethash id reads 0))))
                           (puthash id count reads)
                           (append (when active (funcall hook (list :phase 'member :id id :read count) state))
                                   (list :status (if current 200 404)
                                         :headers (list (cons "etag" (plist-get current :etag)))
                                         :body (when current (jaunder-test--write-entry id (plist-get current :slug)
                                                                                        (plist-get current :href)))))))
                        (t
                         (let* ((page (if (string-match "page-\\([0-9]+\\)\\'" url) (string-to-number (match-string 1 url)) 1))
                                (first (1+ (* 25 (1- page))))
                                (last-page (ceiling (apply #'max (mapcar #'string-to-number (hash-table-keys members))) 25)))
                           (if active (setq pages (1+ pages)) (setq initial-pages (1+ initial-pages)))
                           (append (when active (funcall hook (list :phase 'collection :page page :refresh refreshing) state))
                                   (list :status 200 :body
                                         (concat "<feed xmlns=\"http://www.w3.org/2005/Atom\" xmlns:j=\"https://jaunder.org/ns/atompub\">"
                                                 (when (< page last-page) (format "<link rel=\"next\" href=\"https://example.test/page-%d\"/>" (1+ page)))
                                                 (mapconcat
                                                  (lambda (n)
                                                    (let* ((id (number-to-string n)) (current (gethash id members)))
                                                      (if (not current) ""
                                                        (replace-regexp-in-string
                                                         "</entry>"
                                                         (concat "<j:etag>" (replace-regexp-in-string "\"" "&quot;" (plist-get current :etag)) "</j:etag></entry>")
                                                         (jaunder-test--write-entry id (plist-get current :slug) (plist-get current :href)) t t))))
                                                  (number-sequence first (+ first 24)) "") "</feed>")))))))))
            (setq buffer (jaunder--render-reconcile-report
                          (jaunder--make-reconcile-report :root root :rows rows :inventory (jaunder--inventory-for-root root))
                          (generate-new-buffer " *Jaunder write proof*")) active t)
            (with-current-buffer buffer
              (dolist (row rows) (puthash (jaunder--reconcile-stable-row-key row) t jaunder-reconcile-marks))
              (setq status (funcall (pcase action ('push #'jaunder-reconcile-push-selected)
                                           ('delete #'jaunder-reconcile-delete-selected)
                                           (_ #'jaunder-reconcile-keep-local-selected))))
              (funcall hook (list :phase 'completed :status status) state)
              (list :status status :results jaunder-reconcile-last-batch-results
                    :initial-pages initial-pages :operation-pages operation-pages :pages pages
                    :writes (nreverse writes) :reads reads :originals originals
                    :files (mapcar (lambda (path) (cons (file-name-nondirectory path)
                                                        (with-temp-buffer (insert-file-contents path) (buffer-string))))
                                   (directory-files root t "\\.org\\'"))))))
      (dolist (visiting (buffer-list))
        (when (and (buffer-file-name visiting) (string-prefix-p root (buffer-file-name visiting)))
          (with-current-buffer visiting (set-buffer-modified-p nil)) (kill-buffer visiting)))
      (when (buffer-live-p buffer) (kill-buffer buffer))
      (delete-directory root t))))

(provide 'jaunder-reconcile-write-fixture)
;;; jaunder-reconcile-write-fixture.el ends here
