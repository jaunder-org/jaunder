;;; jaunder-transport-integration.el --- live transport test -*- lexical-binding: t; -*-

;;; Commentary:
;; Exercises `jaunder--http-request' end-to-end against a real server (harness).

;;; Code:

(require 'ert)
(require 'jaunder)
(require 'jaunder-integration-helper)

(ert-deftest jaunder-transport-authed-get-collection ()
  "An authed GET of the posts collection through `jaunder--http-request' is 200."
  (jaunder-test--with-live-server
   (let ((r (jaunder--http-request
             "GET"
             (jaunder--build-url jaunder-test-base-url "atompub" jaunder-test-username "posts"))))
     (should (eq (plist-get r :status) 200))
     (should (string-match-p "<feed" (plist-get r :body))))))

(ert-deftest jaunder-transport-error-status-returned-not-signalled ()
  "A 4xx from the server is returned in :status, not signalled."
  (jaunder-test--with-live-server
   (let ((r (jaunder--http-request
             "GET"
             (jaunder--build-url jaunder-test-base-url "atompub" jaunder-test-username
                                 "posts" "does-not-exist-999999"))))
     (should (>= (plist-get r :status) 400))
     (should (< (plist-get r :status) 500)))))

(provide 'jaunder-transport-integration)
;;; jaunder-transport-integration.el ends here
