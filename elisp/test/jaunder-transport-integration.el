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

(ert-deftest jaunder-transport-preserves-multiline-org-request-body ()
  "AtomPub receives source line endings exactly rather than curl-normalized text."
  (jaunder-test--with-live-server
   (let* ((body "first line\nsecond line\n\nnext paragraph")
          (entry (jaunder--make-entry :title "transport-lines"
                                      :content-type "text/org" :body body))
          (created (jaunder--http-request
                    "POST"
                    (jaunder--build-url jaunder-test-base-url "atompub"
                                        jaunder-test-username "posts")
                    (jaunder--atom-entry->xml entry) "application/atom+xml"))
          (location (jaunder--response-header created "Location"))
          (fetched (and location (jaunder--http-request "GET" location)))
          (fields (and fetched
                       (jaunder--harvest-response-fields (plist-get fetched :body))))
          (content (and fields (car (cdr (assq 'content-nodes fields))))))
     (should (eq (plist-get created :status) 201))
     (should (eq (plist-get fetched :status) 200))
     (should (equal (dom-inner-text content) (concat body "\n"))))))

(ert-deftest jaunder-transport-roundtrips-canonical-conditional-etags ()
  "Created and fetched validators support writes, but stale values do not."
  (jaunder-test--with-live-server
   (let* ((url (jaunder--build-url jaunder-test-base-url "atompub"
                                   jaunder-test-username "posts"))
          (original (jaunder--atom-entry->xml
                     (jaunder--make-entry :title "conditional" :content-type "text"
                                          :body "original")))
          (changed (jaunder--atom-entry->xml
                    (jaunder--make-entry :title "conditional" :content-type "text"
                                         :body "changed")))
          (created (jaunder--http-request "POST" url original "application/atom+xml"))
          (location (jaunder--response-header created "Location"))
          (etag (jaunder--response-header created "ETag")))
     (should (eq (plist-get created :status) 201))
     (should (equal (jaunder--response-header created "Cache-Control") "no-transform"))
     (let ((fetched (jaunder--http-request "GET" location)))
       (should (eq (plist-get fetched :status) 200))
       (should (equal (jaunder--response-header fetched "ETag") etag))
       (should (equal (jaunder--response-header fetched "Cache-Control") "no-transform")))
     (let ((updated (jaunder--http-request
                     "PUT" location changed "application/atom+xml"
                     (list (cons "If-Match" etag)))))
       (should (eq (plist-get updated :status) 200))
       (should (equal (jaunder--response-header updated "Cache-Control") "no-transform"))
       (let* ((current-etag (jaunder--response-header updated "ETag"))
              (stale (jaunder--http-request
                      "PUT" location original "application/atom+xml"
                      (list (cons "If-Match" etag)))))
         (should-not (equal etag current-etag))
         (should (eq (plist-get stale :status) 412))
         (should (equal (jaunder--response-header stale "Cache-Control") "no-transform"))
         (let ((still-current (jaunder--http-request "GET" location)))
           (should (equal (jaunder--response-header still-current "ETag") current-etag))
           (should (string-match-p "changed" (plist-get still-current :body))))
         (let ((deleted (jaunder--http-request
                         "DELETE" location nil nil (list (cons "If-Match" current-etag)))))
           (should (eq (plist-get deleted :status) 204))
           (should (equal (jaunder--response-header deleted "Cache-Control")
                          "no-transform"))))))))

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
