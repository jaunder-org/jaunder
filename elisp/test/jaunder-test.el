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

(ert-deftest jaunder-auth-source-spec-derives-host ()
  (should (equal (jaunder--auth-source-spec "https://blog.example.com/path" "alice")
                 '(:host "blog.example.com" :user "alice" :max 1))))

(ert-deftest jaunder-auth-source-spec-ignores-port ()
  (should (equal (plist-get (jaunder--auth-source-spec "https://blog.example.com:8443" "bob")
                            :host)
                 "blog.example.com")))

;;; jaunder-test.el ends here
