;;; jaunder-smoke-integration.el --- live-server smoke tests -*- lexical-binding: t; -*-

;;; Commentary:
;; End-to-end smoke over real HTTP: proves boot + provisioning + auth, driving the
;; real client transport (`jaunder--http-request', Unit C #74, plz/ADR-0038).  The
;; AtomPub surface (including the service document) requires authentication;
;; `jaunder--http-request' supplies the app-password Basic header via auth-source.
;; See ADR-0035.

;;; Code:

(require 'ert)
(require 'jaunder)
(require 'jaunder-integration-helper)

(ert-deftest jaunder-smoke-service-document-advertises-capability ()
  "The service document advertises the j:extension capability."
  (jaunder-test--with-live-server
   (let ((r (jaunder--http-request
             "GET" (jaunder--build-url jaunder-base-url "atompub" "service"))))
     (should (eq (plist-get r :status) 200))
     (should (string-match-p "j:extension" (plist-get r :body)))
     (should (string-match-p "format-media-type" (plist-get r :body)))
     (should (string-match-p "slug" (plist-get r :body))))))

(ert-deftest jaunder-smoke-authenticated-collection ()
  "An app-password Basic request returns the user's (empty) posts collection."
  (jaunder-test--with-live-server
   (let ((r (jaunder--http-request
             "GET" (jaunder--build-url jaunder-base-url "atompub" jaunder-username "posts"))))
     (should (eq (plist-get r :status) 200)))))

(provide 'jaunder-smoke-integration)
;;; jaunder-smoke-integration.el ends here
