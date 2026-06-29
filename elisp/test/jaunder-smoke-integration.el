;;; jaunder-smoke-integration.el --- live-server smoke tests -*- lexical-binding: t; -*-

;;; Commentary:
;; End-to-end smoke over real HTTP: proves boot + provisioning + auth.  Uses
;; url.el directly (the client HTTP layer is Unit C, #74).  See ADR-0035.

;;; Code:

(require 'ert)
(require 'jaunder)
(require 'jaunder-integration-helper)

(defun jaunder-smoke--get (url &optional extra-headers)
  "GET URL with optional EXTRA-HEADERS; return (STATUS . BODY).
Uses `let*' so `url-request-extra-headers' is bound before the request runs."
  (let* ((url-request-method "GET")
         (url-request-extra-headers extra-headers)
         (buf (url-retrieve-synchronously url t t 10)))
    (unwind-protect
        (with-current-buffer buf
          (goto-char (point-min))
          (let ((status (and (re-search-forward "^HTTP/[0-9.]+ \\([0-9]+\\)" nil t)
                             (string-to-number (match-string 1))))
                (body (progn (goto-char (point-min))
                             (when (re-search-forward "\r?\n\r?\n" nil t)
                               (buffer-substring-no-properties (point) (point-max))))))
            (cons status body)))
      (when (buffer-live-p buf) (kill-buffer buf)))))

(defun jaunder-smoke--auth ()
  "Basic-auth header list for the harness-provisioned user."
  (list (jaunder--basic-auth-header jaunder-username jaunder-test-app-password)))

;; The AtomPub surface (including the service document) requires authentication,
;; so every request carries the app-password Basic header.

(ert-deftest jaunder-smoke-service-document-advertises-capability ()
  "The service document advertises the j:extension capability."
  (jaunder-test--with-live-server
   (let ((resp (jaunder-smoke--get
                (jaunder--build-url jaunder-base-url "atompub" "service")
                (jaunder-smoke--auth))))
     (should (eq (car resp) 200))
     (should (string-match-p "j:extension" (cdr resp)))
     (should (string-match-p "format-media-type" (cdr resp)))
     (should (string-match-p "slug" (cdr resp))))))

(ert-deftest jaunder-smoke-authenticated-collection ()
  "An app-password Basic request returns the user's (empty) posts collection."
  (jaunder-test--with-live-server
   (let ((resp (jaunder-smoke--get
                (jaunder--build-url jaunder-base-url "atompub" jaunder-username "posts")
                (jaunder-smoke--auth))))
     (should (eq (car resp) 200)))))

(provide 'jaunder-smoke-integration)
;;; jaunder-smoke-integration.el ends here
