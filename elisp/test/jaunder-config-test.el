;;; jaunder-config-test.el --- ERT suite for jaunder-config -*- lexical-binding: t; -*-

;;; Commentary:
;; Unit tests for the corresponding jaunder package module.

;;; Code:

(require 'ert)
(require 'jaunder)

;;; multi-blog config + resolution

(ert-deftest jaunder-resolve-blog-longest-prefix ()
  (let ((jaunder-blogs '(("/home/me/blog/" :base-url "https://a" :username "a")
                         ("/home/me/blog/work/" :base-url "https://b" :username "b"))))
    (should (equal (plist-get (jaunder--resolve-blog "/home/me/blog/post.org") :username) "a"))
    (should (equal (plist-get (jaunder--resolve-blog "/home/me/blog/work/x.org") :username) "b"))))

(ert-deftest jaunder-resolve-blog-errors-when-unconfigured ()
  (let ((jaunder-blogs nil))
    (should-error (jaunder--resolve-blog "/tmp/x.org"))))

(ert-deftest jaunder-resolve-blog-errors-on-incomplete-entry ()
  ;; A matched entry missing :username must fail loudly rather than issue a
  ;; half-configured request: a nil username silently yields a wrong URL (the
  ;; segment is dropped) and garbage Basic credentials.
  (let ((jaunder-blogs '(("/home/me/blog/" :base-url "https://a"))))
    (should-error (jaunder--resolve-blog "/home/me/blog/post.org"))))

(ert-deftest jaunder-resolve-blog-errors-on-malformed-base-url ()
  ;; The real requirement on :base-url is that it is a URL, not merely non-empty;
  ;; a value with no scheme/host is rejected at the config boundary.
  (let ((jaunder-blogs '(("/home/me/blog/" :base-url "not-a-url" :username "a"))))
    (should-error (jaunder--resolve-blog "/home/me/blog/post.org"))))

(ert-deftest jaunder-resolve-blog-normalizes-base-url-trailing-slash ()
  ;; A trailing slash on :base-url is stripped here so downstream URL joining can
  ;; treat the base as a clean prefix.
  (let ((jaunder-blogs '(("/home/me/blog/" :base-url "https://a/" :username "a"))))
    (should (equal (plist-get (jaunder--resolve-blog "/home/me/blog/post.org") :base-url)
                   "https://a"))))

(ert-deftest jaunder-call-with-blog-binds-active-blog-dynamically ()
  (let ((jaunder-blogs '(("/home/me/blog/" :base-url "https://a" :username "a")))
        (jaunder--active-blog '(:base-url "https://outer" :username "outer")))
    (jaunder--call-with-blog
     "/home/me/blog/post.org"
     (lambda ()
       (should (equal (jaunder--active-base-url) "https://a"))
       (should (equal (jaunder--active-username) "a"))))
    (should (equal (jaunder--active-base-url) "https://outer"))))

(ert-deftest jaunder-call-with-blog-returns-thunk-result ()
  (let ((jaunder-blogs '(("/home/me/blog/" :base-url "https://a" :username "a"))))
    (should (eq (jaunder--call-with-blog "/home/me/blog/post.org"
                                         (lambda () 'result))
                'result))))

(ert-deftest jaunder-call-with-blog-propagates-thunk-errors ()
  (let ((jaunder-blogs '(("/home/me/blog/" :base-url "https://a" :username "a")))
        (jaunder--active-blog '(:base-url "https://outer" :username "outer")))
    (should-error
     (jaunder--call-with-blog "/home/me/blog/post.org"
                              (lambda () (error "injected thunk failure"))))
    (should (equal (jaunder--active-base-url) "https://outer"))))

(ert-deftest jaunder-call-with-blog-resolves-file-once ()
  (let ((calls 0))
    (cl-letf (((symbol-function 'jaunder--resolve-blog)
               (lambda (_file)
                 (setq calls (1+ calls))
                 '(:base-url "https://a" :username "a"))))
             (should (eq (jaunder--call-with-blog "/home/me/blog/post.org"
                                                  (lambda () 'result))
                         'result))
             (should (= calls 1)))))

(ert-deftest jaunder-active-accessors-error-without-active-blog ()
  ;; Outside `jaunder--call-with-blog' the accessors must signal, so a transport
  ;; call that forgot to establish request context fails loudly instead of using nil.
  (let ((jaunder--active-blog nil))
    (should-error (jaunder--active-base-url))
    (should-error (jaunder--active-username))))

;;; jaunder-config-test.el ends here
