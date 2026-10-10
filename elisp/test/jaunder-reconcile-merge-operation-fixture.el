;;; jaunder-reconcile-merge-operation-fixture.el --- Public merge lifecycle wire -*- lexical-binding: t; -*-

;; Copyright (C) 2026 Jaunder contributors

;;; Commentary:
;; Real preparation, scratch authorization, conditional finish and refresh with
;; controlled Ediff output and temporary Posts.  Hooks change state at actual
;; HTTP/editing boundaries; no preflight, proof or checkpoint is substituted.

;;; Code:
(require 'jaunder)
(require 'cl-lib)
(load (expand-file-name "jaunder-reconcile-write-fixture.el"
                        (file-name-directory (or load-file-name buffer-file-name))) nil t)

(defun jaunder-test--merge-lifecycle (hook)
  "Run public merge preparation and completion with event HOOK, returning proof.
HOOK receives event and state plists and may override a wire response.  During
editing it may explicitly finish additional attempts before the default finish."
  (let* ((root (file-name-as-directory (make-temp-file "jaunder-merge-operation-" t)))
         (path (expand-file-name "post-001.org" root))
         (target (expand-file-name "post-025.org" root))
         (jaunder-blogs (list (cons root '(:base-url "https://example.test" :username "alice"))))
         (members (make-hash-table :test #'equal))
         (counts (make-hash-table :test #'eq))
         (reads (make-hash-table :test #'equal))
         (parent-active (jaunder--operation-active-p))
         (phase 'opening) writes results scratch session report row state
         (real-refresh (symbol-function 'jaunder--reconcile-refresh-buffer))
         (original "#+TITLE: Local\n#+PROPERTY: JAUNDER_ID 1\n#+PROPERTY: JAUNDER_SLUG post-001\n#+PROPERTY: JAUNDER_SYNCED \"saved\"\n#+PROPERTY: JAUNDER_DATE_TZ UTC\n#+PROPERTY: JAUNDER_STATUS draft\n\nAuthored body.\n"))
    (unwind-protect
        (progn
          (dotimes (n 100)
            (let ((id (number-to-string (1+ n))) (slug (format "post-%03d" (1+ n))))
              (puthash id (list :slug slug :href (concat "https://example.test/~alice/" slug) :etag "\"old\"") members)))
          (with-temp-file path (insert original))
          (with-temp-file target
            (insert "#+PROPERTY: JAUNDER_ID 25\n#+PROPERTY: JAUNDER_SLUG post-025\n\nTarget.\n"))
          (setq row (jaunder--make-reconcile-row
                     :key "post:1" :state 'conflict :remote-etag "\"old\""
                     :local-sha256 (jaunder--reconcile-file-sha256 path)
                     :local (jaunder--make-inventory-local :id "1" :slug "post-001" :path path)
                     :member (jaunder--make-inventory-member :id "1" :slug "post-001"
                                                             :edit-uri "https://example.test/atompub/alice/posts/1"))
                state (list :root root :path path :target target :row row :members members :original original))
          (cl-letf (((symbol-function 'y-or-n-p) (lambda (&rest _) t))
                    ((symbol-function 'ediff-merge-buffers)
                     (lambda (a _b &optional startup _job _file)
                       (should (eq parent-active (jaunder--operation-active-p)))
                       (setq scratch (generate-new-buffer " *Merge lifecycle C*"))
                       (with-current-buffer scratch
                         (insert (with-current-buffer a (buffer-string))
                                 "\nMerged [[file:./post-025.org][Target]].\n"))
                       (with-temp-buffer
                         (setq-local ediff-buffer-C scratch)
                         (dolist (function startup) (funcall function)))))
                    ((symbol-function 'jaunder--reconcile-refresh-buffer)
                     (lambda (buffer)
                       (should-not (jaunder--operation-active-p))
                       (let ((before phase))
                         (setq phase 'refresh)
                         (unwind-protect (funcall real-refresh buffer) (setq phase before)))))
                    ((symbol-function 'jaunder--http-request)
                     (lambda (method url &optional xml _type headers)
                       (cond
                        ((string-suffix-p "/service" url)
                         '(:status 200 :body "<service xmlns=\"http://www.w3.org/2007/app\" xmlns:atom=\"http://www.w3.org/2005/Atom\"><workspace><atom:title>User</atom:title></workspace></service>"))
                        ((not (equal method "GET"))
                         (let ((prior (gethash "1" members)))
                           (push (list :xml xml :headers headers) writes)
                           (puthash "1" (plist-put (copy-sequence prior) :etag "\"written\"") members)
                           (let ((override (funcall hook (list :phase phase :kind 'write :xml xml) state)))
                             (when (plist-get override :no-commit) (puthash "1" prior members))
                             (append override
                                     (list :status 200 :headers '(("etag" . "\"written\"")
                                                                  ("location" . "https://example.test/atompub/alice/posts/1"))
                                           :body (jaunder-test--write-entry "1" "post-001" (plist-get prior :href)))))))
                        ((string-match "/posts/\\([0-9]+\\)\\'" url)
                         (let* ((id (match-string 1 url)) (current (gethash id members))
                                (count (1+ (gethash id reads 0))))
                           (puthash id count reads)
                           (append (funcall hook (list :phase phase :kind 'member :id id :read count) state)
                                   (list :status (if current 200 404)
                                         :headers (list (cons "etag" (plist-get current :etag))
                                                        (cons "x-jaunder-instance" "12345678-1234-1234-1234-123456789abc"))
                                         :body (when current
                                                 (replace-regexp-in-string
                                                  "Remote body." "Remote [[https://example.test/~alice/post-025][Target]]."
                                                  (jaunder-test--write-entry id (plist-get current :slug) (plist-get current :href)) t t))))))
                        (t
                         (puthash phase (1+ (gethash phase counts 0)) counts)
                         (let* ((page (if (string-match "page-\\([0-9]+\\)\\'" url) (string-to-number (match-string 1 url)) 1))
                                (first (1+ (* 25 (1- page)))))
                           (append (funcall hook (list :phase phase :kind 'collection :page page) state)
                                   (list :status 200 :body
                                         (concat "<feed xmlns=\"http://www.w3.org/2005/Atom\" xmlns:j=\"https://jaunder.org/ns/atompub\">"
                                                 (when (< page 4) (format "<link rel=\"next\" href=\"https://example.test/page-%d\"/>" (1+ page)))
                                                 (mapconcat
                                                  (lambda (n)
                                                    (let* ((id (number-to-string n)) (member (gethash id members)))
                                                      (when member
                                                        (replace-regexp-in-string
                                                         "</entry>"
                                                         (concat "<j:etag>" (replace-regexp-in-string "\"" "&quot;" (plist-get member :etag)) "</j:etag></entry>")
                                                         (jaunder-test--write-entry id (plist-get member :slug) (plist-get member :href)) t t))))
                                                  (number-sequence first (+ first 24)) "") "</feed>")))))))))
            (setq report (jaunder--call-with-blog root
						  (lambda ()
						    (jaunder--render-reconcile-report
						     (jaunder--make-reconcile-report :root root :rows (list row)
										     :inventory (jaunder--inventory-for-root root))
						     (generate-new-buffer " *Merge lifecycle report*")))))
            (setq phase 'prepare)
            (with-current-buffer report
              (puthash "post:1" t jaunder-reconcile-marks)
              (setq scratch (jaunder-reconcile-merge-selected)))
            (when (buffer-live-p scratch)
              (setq session (buffer-local-value 'jaunder-reconcile-merge-session scratch))
              (setq state (append state (list :scratch scratch :session session :report report)))
              (setq phase 'finish)
              (funcall hook '(:phase editing) state)
              (when (buffer-live-p scratch)
                (with-current-buffer scratch
                  (push (jaunder-reconcile-merge-finish) results))))
            (list :counts counts :reads reads :writes (nreverse writes) :results (nreverse results)
                  :retained (buffer-live-p scratch)
                  :last (when (buffer-live-p scratch) (buffer-local-value 'jaunder-reconcile-merge-last-result scratch))
                  :terminal (buffer-local-value 'jaunder-reconcile-last-batch-results report)
                  :bytes (with-temp-buffer (insert-file-contents path) (buffer-string))
                  :original original)))
      (when session (jaunder--reconcile-merge-close-views session))
      (when (buffer-live-p scratch)
        (with-current-buffer scratch (setq-local jaunder-reconcile-merge-allow-kill t) (set-buffer-modified-p nil))
        (kill-buffer scratch))
      (when (buffer-live-p report) (kill-buffer report))
      (dolist (buffer (buffer-list))
        (when (and (buffer-file-name buffer) (string-prefix-p root (buffer-file-name buffer)))
          (with-current-buffer buffer (set-buffer-modified-p nil)) (kill-buffer buffer)))
      (delete-directory root t))))

(provide 'jaunder-reconcile-merge-operation-fixture)
;;; jaunder-reconcile-merge-operation-fixture.el ends here
