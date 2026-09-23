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
                      (unless (and id old-etag (not (string-suffix-p "-zstd\"" old-etag)))
                        (error "Published ETag is not canonical: %S" old-etag))
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
                        (jaunder-reconcile-push-selected)
                        (unless (equal (mapcar #'jaunder-reconcile-result-outcome
                                               jaunder-reconcile-last-batch-results)
                                       '(success))
                          (error "Local-ahead proxy push failed: %S"
                                 jaunder-reconcile-last-batch-results)))
                      (let ((remote (jaunder--http-request "GET" url)))
                        (unless (and (eq (plist-get remote :status) 200)
                                     (string-match-p "Local ahead change" (plist-get remote :body))
                                     (not (equal old-etag
                                                 (jaunder--response-header remote "ETag"))))
                          (error "Local-ahead proxy push did not change the Post")))))))
               (message "Emacs local-ahead: identity validator, reconciliation push succeeded"))
    (when (buffer-live-p buffer)
      (with-current-buffer buffer
        (set-buffer-modified-p nil)
        (kill-buffer buffer)))))

;;; atompub-local-ahead.el ends here
