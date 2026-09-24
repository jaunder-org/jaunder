;;; atompub-local-ahead.el --- proxy-backed reconciliation proof -*- lexical-binding: t; -*-

;;; Commentary:
;; Invoked by server/tests/proxy/atompub_etag.py with a temporary local root,
;; authenticated Jaunder URL, and a temporary app-password in its environment.
;; Runs the real Emacs Protocol Client through Caddy, not a mocked transport.

;;; Code:

(add-to-list 'load-path (expand-file-name "../.." (file-name-directory load-file-name)))
(require 'cl-lib)
(require 'subr-x)
(require 'jaunder)

(let* ((root (file-name-as-directory (getenv "JAUNDER_PROXY_ROOT")))
       (base (getenv "JAUNDER_PROXY_BASE"))
       (token (getenv "JAUNDER_PROXY_TOKEN"))
       (path (expand-file-name "proxy-local-ahead.org" root))
       (jaunder-blogs (list (cons root (list :base-url base :username "alice"))))
       buffer)
  (with-temp-file path
    (insert "#+TITLE: Proxy local ahead\n#+PROPERTY: JAUNDER_STATUS published\n\nOriginal body.\n"))
  (unwind-protect
      (cl-letf (((symbol-function 'jaunder--auth-secret) (lambda () token))
                ((symbol-function 'y-or-n-p) (lambda (_) t)))
               (jaunder--call-with-blog
                root
                (lambda ()
                  (setq buffer (find-file-noselect path))
                  (with-current-buffer buffer
                    (jaunder-publish)
                    (let* ((id (jaunder--buffer-property "JAUNDER_ID"))
                           (old-etag (jaunder--buffer-property "JAUNDER_SYNCED"))
                           (url (jaunder--member-url id)))
                      (unless (and id old-etag (not (string-suffix-p "-zstd\"" old-etag))
                                   (equal old-etag (jaunder--response-header
                                                    (jaunder--http-request "GET" url) "ETag")))
                        (error "Published ETag is not the canonical Member ETag: %S" old-etag))
                      ;; Make the local file server-ahead, then actually fetch it.
                      ;; The pull must install the new validator in JAUNDER_SYNCED.
                      (let ((changed (jaunder--http-request
                                      "PUT" url
                                      (jaunder--atom-entry->xml
                                       (jaunder--make-entry
                                        :title "Proxy local ahead" :content-type "text/org"
                                        :body "Remote change.\n"))
                                      "application/atom+xml"
                                      (list (cons "If-Match" old-etag)))))
                        (unless (eq (plist-get changed :status) 200)
                          (error "Cannot establish server-ahead state: %S" changed)))
                      (jaunder-reconcile root)
                      (with-current-buffer "*Jaunder Reconcile*"
                        (puthash (format "post:%s" id) t jaunder-reconcile-marks)
                        (jaunder-reconcile-pull-selected)
                        (unless (equal (mapcar #'jaunder-reconcile-result-outcome
                                               jaunder-reconcile-last-batch-results)
                                       '(success))
                          (error "Member pull through proxy failed")))
                      (let ((pulled-etag (jaunder--buffer-property "JAUNDER_SYNCED"))
                            (remote-etag (jaunder--response-header
                                          (jaunder--http-request "GET" url) "ETag")))
                        (unless (and pulled-etag (equal pulled-etag remote-etag)
                                     (not (equal pulled-etag old-etag))
                                     (not (string-suffix-p "-zstd\"" pulled-etag)))
                          (error "Pull did not store the canonical read ETag: %S" pulled-etag))
                        (goto-char (point-max))
                        (insert "Local ahead change.\n")
                        (jaunder--set-property "JAUNDER_LOCAL_AHEAD" "true")
                        (save-buffer)
                        (jaunder-reconcile root)
                        (with-current-buffer "*Jaunder Reconcile*"
                          (let ((row (cl-find (format "post:%s" id)
                                              (jaunder-reconcile-report-rows jaunder-reconcile-report)
                                              :key #'jaunder--reconcile-stable-row-key :test #'equal)))
                            (unless (and row (eq (jaunder-reconcile-row-state row) 'local-ahead))
                              (error "Expected local-ahead row before proxy push")))
                          (puthash (format "post:%s" id) t jaunder-reconcile-marks)
                          (let ((real-http (symbol-function 'jaunder--http-request))
                                replayed)
                            (cl-letf (((symbol-function 'jaunder--http-request)
                                       (lambda (method target &rest args)
                                         (when (and (equal method "PUT") (equal target url))
                                           (setq replayed (cdr (assoc "If-Match" (nth 2 args)))))
                                         (apply real-http method target args))))
                                     (jaunder-reconcile-push-selected))
                            (unless (equal replayed pulled-etag)
                              (error "Push did not replay the pulled ETag: %S" replayed)))
                          (unless (equal (mapcar #'jaunder-reconcile-result-outcome
                                                 jaunder-reconcile-last-batch-results)
                                         '(success))
                            (error "Local-ahead proxy push failed: %S"
                                   jaunder-reconcile-last-batch-results)))
                        (let ((remote (jaunder--http-request "GET" url)))
                          (unless (and (eq (plist-get remote :status) 200)
                                       (string-match-p "Local ahead change" (plist-get remote :body))
                                       (not (equal pulled-etag
                                                   (jaunder--response-header remote "ETag"))))
                            (error "Local-ahead proxy push did not change the Post"))))))))
               (message "Emacs local-ahead: identity validator, reconciliation push succeeded"))
    (when (buffer-live-p buffer)
      (with-current-buffer buffer
        (set-buffer-modified-p nil)
        (kill-buffer buffer)))))

;;; atompub-local-ahead.el ends here
