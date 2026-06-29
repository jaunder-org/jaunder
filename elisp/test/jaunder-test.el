;;; jaunder-test.el --- ERT suite for jaunder -*- lexical-binding: t; -*-

;;; Commentary:
;; Unit tests for the pure helpers in jaunder.el.

;;; Code:

(require 'ert)
(require 'jaunder)

(ert-deftest jaunder-build-url-bare ()
  (should (equal (jaunder--build-url "https://x.example") "https://x.example")))

(ert-deftest jaunder-build-url-strips-trailing-slash ()
  (should (equal (jaunder--build-url "https://x.example/") "https://x.example")))

(ert-deftest jaunder-build-url-joins-segments ()
  (should (equal (jaunder--build-url "https://x.example" "atom" "feed")
                 "https://x.example/atom/feed")))

(ert-deftest jaunder-build-url-collapses-inner-slashes ()
  (should (equal (jaunder--build-url "https://x.example/" "/atom/" "feed")
                 "https://x.example/atom/feed")))

(ert-deftest jaunder-build-url-drops-empty-segments ()
  (should (equal (jaunder--build-url "https://x.example" nil "" "feed")
                 "https://x.example/feed")))

(ert-deftest jaunder-build-url-errors-on-empty-base ()
  (should-error (jaunder--build-url nil))
  (should-error (jaunder--build-url "")))

(ert-deftest jaunder-basic-auth-header ()
  (should (equal (jaunder--basic-auth-header "alice" "secret")
                 (cons "Authorization" "Basic YWxpY2U6c2VjcmV0"))))

(ert-deftest jaunder-basic-auth-header-utf8-roundtrips ()
  ;; Non-ASCII credentials must not raise; the base64 payload must decode
  ;; back to the original UTF-8 "user:password" (RFC 7617).
  (let* ((header (jaunder--basic-auth-header "tëst" "pä"))
         (b64 (substring (cdr header) (length "Basic "))))
    (should (equal (decode-coding-string (base64-decode-string b64) 'utf-8)
                   "tëst:pä"))))

(ert-deftest jaunder-auth-source-spec-derives-host ()
  (should (equal (jaunder--auth-source-spec "https://blog.example.com/path" "alice")
                 '(:host "blog.example.com" :user "alice" :max 1))))

(ert-deftest jaunder-auth-source-spec-ignores-port ()
  (should (equal (plist-get (jaunder--auth-source-spec "https://blog.example.com:8443" "bob")
                            :host)
                 "blog.example.com")))

(ert-deftest jaunder-plz-response->plist-maps-status-headers-body ()
  (let ((r (jaunder--plz-response->plist
            (make-plz-response
             :status 200
             :headers '((content-type . "application/atom+xml")
                        (etag . "\"v1\"")
                        (location . "/atompub/alice/posts/42"))
             :body "<feed/>"))))
    (should (eq (plist-get r :status) 200))
    (should (equal (jaunder--response-header r "ETag") "\"v1\""))
    (should (equal (jaunder--response-header r "content-type") "application/atom+xml"))
    (should (equal (jaunder--response-header r "location") "/atompub/alice/posts/42"))
    (should (equal (plist-get r :body) "<feed/>"))))

(ert-deftest jaunder-plz-response->plist-nil-body-is-empty-string ()
  (let ((r (jaunder--plz-response->plist
            (make-plz-response :status 201 :headers nil :body nil))))
    (should (eq (plist-get r :status) 201))
    (should (equal (plist-get r :body) ""))))

(ert-deftest jaunder-response-header-is-case-insensitive-and-missing-nil ()
  (let ((r (jaunder--plz-response->plist
            (make-plz-response :status 200 :headers '((x-a . "1")) :body ""))))
    (should (equal (jaunder--response-header r "x-a") "1"))
    (should (equal (jaunder--response-header r "X-A") "1"))
    (should (null (jaunder--response-header r "x-missing")))))

;;; jaunder-test.el ends here
