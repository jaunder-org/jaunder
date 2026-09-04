;;; jaunder-transport-test.el --- ERT suite for jaunder-transport -*- lexical-binding: t; -*-

;;; Commentary:
;; Unit tests for the corresponding jaunder package module.

;;; Code:

(require 'ert)
(require 'jaunder)

(ert-deftest jaunder-build-url-bare ()
  (should (equal (jaunder--build-url "https://x.example") "https://x.example")))

(ert-deftest jaunder-build-url-joins-segments ()
  (should (equal (jaunder--build-url "https://x.example" "atom" "feed")
                 "https://x.example/atom/feed")))

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

(ert-deftest jaunder-http-request-passes-extra-headers ()
  (let (captured)
    (cl-letf (((symbol-function 'jaunder--auth-secret) (lambda () "tok"))
              ((symbol-function 'jaunder--plz-response->plist) (lambda (r) r))
              ((symbol-function 'plz)
               (lambda (_verb _url &rest args)
                 (setq captured (plist-get args :headers))
                 '(:status 201 :body ""))))
             (let ((jaunder--active-blog '(:base-url "http://x" :username "alice")))
               (jaunder--http-request "POST" "http://x/media" (list 'file "/tmp/a.png")
                                      "image/png" (list (cons "Slug" "a.png"))))
             (should (equal (cdr (assoc "Slug" captured)) "a.png"))
             (should (equal (cdr (assoc "Content-Type" captured)) "image/png"))
             (should (assoc "Authorization" captured)))))

(ert-deftest jaunder-curl-header-value-escapes-quotes-and-backslashes ()
  ;; plz 0.9.1 wraps each header value in double quotes inside a curl --config
  ;; file without escaping it, so a raw quote (a strong ETag echoed as If-Match)
  ;; truncates the header and curl drops it.  Escaping \ and " lets curl rebuild
  ;; the literal value; a value without either is unchanged.
  (should (equal (jaunder--curl-header-value "\"abc123\"") "\\\"abc123\\\""))
  (should (equal (jaunder--curl-header-value "a\\b") "a\\\\b"))
  (should (equal (jaunder--curl-header-value "application/atom+xml;type=entry")
                 "application/atom+xml;type=entry")))

(ert-deftest jaunder-auth-secret-rejects-missing-credential ()
  "An auth-source match without a usable secret cannot make an anonymous request."
  (let ((jaunder--active-blog '(:base-url "https://blog" :username "alice")))
    (cl-letf (((symbol-function 'auth-source-search) (lambda (&rest _) nil)))
             (should-error (jaunder--auth-secret) :type 'error))))

(ert-deftest jaunder-auth-secret-returns-a-literal-auth-source-secret ()
  "A literal auth-source secret is returned unchanged for request authentication."
  (let ((jaunder--active-blog '(:base-url "https://blog" :username "alice")))
    (cl-letf (((symbol-function 'auth-source-search)
               (lambda (&rest _) (list '(:secret "literal-token")))))
             (should (equal (jaunder--auth-secret) "literal-token")))))

(ert-deftest jaunder-http-request-resignals-transport-failure-without-response ()
  "A transport failure without an HTTP response remains distinguishable to retry."
  (let ((jaunder--active-blog '(:base-url "https://blog" :username "alice")))
    (cl-letf (((symbol-function 'jaunder--auth-secret) (lambda () "secret"))
              ((symbol-function 'plz)
               (lambda (&rest _)
                 (signal 'plz-curl-error
                         (list "offline" (make-plz-error :message "offline"))))))
             (should-error (jaunder--http-request "GET" "https://blog/posts")
                           :type 'plz-error))))

;;; jaunder-transport-test.el ends here
