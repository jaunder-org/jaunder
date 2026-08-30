;;; jaunder-test.el --- ERT suite for jaunder -*- lexical-binding: t; -*-

;;; Commentary:
;; Unit tests for the pure helpers in jaunder.el.

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

;;; org->atom — field mapping

(defun jaunder-test--entry (org)
  "Map ORG text to a `jaunder-entry' via a temp org buffer."
  (with-temp-buffer
    (insert org)
    (org-mode)
    (jaunder--org->atom)))

(ert-deftest jaunder-org->atom-title-present ()
  (should (equal (jaunder-entry-title
                  (jaunder-test--entry "#+TITLE: My Post\n\nBody\n"))
                 "My Post")))

(ert-deftest jaunder-org->atom-title-absent-is-nil ()
  (should (null (jaunder-entry-title
                 (jaunder-test--entry "Just a note\n")))))

(ert-deftest jaunder-org->atom-title-empty-is-nil ()
  (should (null (jaunder-entry-title
                 (jaunder-test--entry "#+TITLE:\n\nBody\n")))))

(ert-deftest jaunder-org->atom-repeated-titles-join-with-newlines ()
  ;; Pulled multiline titles become repeated #+TITLE lines and must publish back
  ;; as the original Atom title, rather than silently retaining only the first.
  (should (equal (jaunder-entry-title
                  (jaunder-test--entry
                   "#+TITLE: First line\n#+TITLE: Second line\n\nBody\n"))
                 "First line\nSecond line")))

(ert-deftest jaunder-org->atom-keywords-split-multiline-flatten ()
  (should (equal (jaunder-entry-categories
                  (jaunder-test--entry
                   "#+KEYWORDS: rust, programming\n#+KEYWORDS: emacs\n\nBody\n"))
                 '("rust" "programming" "emacs"))))

(ert-deftest jaunder-org->atom-keywords-absent-is-nil ()
  (should (null (jaunder-entry-categories
                 (jaunder-test--entry "#+TITLE: T\n\nBody\n")))))

(ert-deftest jaunder-org->atom-description-joined-with-newline ()
  (should (equal (jaunder-entry-summary
                  (jaunder-test--entry
                   "#+DESCRIPTION: line one\n#+DESCRIPTION: line two\n\nBody\n"))
                 "line one\nline two")))

(ert-deftest jaunder-org->atom-description-absent-is-nil ()
  (should (null (jaunder-entry-summary
                 (jaunder-test--entry "#+TITLE: T\n\nBody\n")))))

(ert-deftest jaunder-org->atom-content-type-is-always-org ()
  ;; org->atom converts an org buffer, so the content is org regardless of any
  ;; JAUNDER_FORMAT header (which would only lie about org body).
  (should (equal (jaunder-entry-content-type
                  (jaunder-test--entry "#+TITLE: T\n\nB\n"))
                 "text/org"))
  (should (equal (jaunder-entry-content-type
                  (jaunder-test--entry "#+PROPERTY: JAUNDER_FORMAT markdown\n\nB\n"))
                 "text/org")))

(ert-deftest jaunder-org->atom-status-draft ()
  (should (eq t (jaunder-entry-draft
                 (jaunder-test--entry "#+PROPERTY: JAUNDER_STATUS draft\n\nB\n")))))

(ert-deftest jaunder-org->atom-status-scheduled-not-draft ()
  (should (null (jaunder-entry-draft
                 (jaunder-test--entry "#+PROPERTY: JAUNDER_STATUS scheduled\n\nB\n")))))

(ert-deftest jaunder-org->atom-status-published-not-draft ()
  (should (null (jaunder-entry-draft
                 (jaunder-test--entry "#+PROPERTY: JAUNDER_STATUS published\n\nB\n")))))

(ert-deftest jaunder-org->atom-body-strips-header-block ()
  (let ((e (jaunder-test--entry
            (concat "#+TITLE: My Post\n"
                    "#+KEYWORDS: rust\n"
                    "#+DESCRIPTION: d\n"
                    "#+PROPERTY: JAUNDER_STATUS draft\n"
                    "#+PROPERTY: JAUNDER_FORMAT org\n"
                    "\n"
                    "Body line 1\n"
                    "Body line 2\n"))))
    (should (equal (jaunder-entry-body e) "Body line 1\nBody line 2"))
    (should-not (string-match-p "JAUNDER_" (jaunder-entry-body e)))
    (should-not (string-match-p "#\\+TITLE" (jaunder-entry-body e)))))

(ert-deftest jaunder-org->atom-body-keeps-leading-indentation ()
  ;; Header-block stripping locates the start of content and trims only the
  ;; trailing newline; leading whitespace on the first content line is body, not
  ;; header, so it is preserved rather than reflowed.
  (let ((e (jaunder-test--entry
            (concat "#+TITLE: T\n"
                    "\n"
                    "    indented first line\n"
                    "second line\n"))))
    (should (equal (jaunder-entry-body e) "    indented first line\nsecond line"))))

(ert-deftest jaunder-org->atom-untitled-all-emoji-body ()
  (let ((e (jaunder-test--entry "🎉✨\n")))
    (should (null (jaunder-entry-title e)))
    (should (equal (jaunder-entry-body e) "🎉✨"))))

(ert-deftest jaunder-org->atom-body-strips-interleaved-unmapped-keywords ()
  ;; An unmapped keyword between header lines must not halt stripping and leak
  ;; a later JAUNDER_* into the body (the header block is any leading run of
  ;; #+KEY: lines, not just the mapped ones).
  (let ((e (jaunder-test--entry
            (concat "#+TITLE: My Post\n"
                    "#+AUTHOR: Alice\n"
                    "#+OPTIONS: toc:nil\n"
                    "#+PROPERTY: JAUNDER_STATUS draft\n"
                    "\n"
                    "Body line 1\n"))))
    (should (equal (jaunder-entry-body e) "Body line 1"))
    (should-not (string-match-p "JAUNDER_" (jaunder-entry-body e)))
    (should-not (string-match-p "#\\+AUTHOR" (jaunder-entry-body e)))))

;;; offset parsing / zone resolution

(ert-deftest jaunder-offset->seconds-negative ()
  (should (= (jaunder--offset->seconds "-0500") (* -5 3600))))

(ert-deftest jaunder-offset->seconds-positive-with-minutes ()
  (should (= (jaunder--offset->seconds "+0530") (+ (* 5 3600) (* 30 60)))))

(ert-deftest jaunder-offset->seconds-colon-form ()
  (should (= (jaunder--offset->seconds "-05:00") (* -5 3600))))

(ert-deftest jaunder-offset->seconds-zero ()
  (should (= (jaunder--offset->seconds "+0000") 0)))

(ert-deftest jaunder-offset->seconds-iana-name-is-nil ()
  (should (null (jaunder--offset->seconds "America/New_York"))))

(ert-deftest jaunder-offset->seconds-garbage-is-nil ()
  (should (null (jaunder--offset->seconds "not-an-offset")))
  (should (null (jaunder--offset->seconds nil))))

(ert-deftest jaunder-resolve-zone-iana-passthrough ()
  (should (equal (jaunder--resolve-zone "America/New_York") "America/New_York")))

(ert-deftest jaunder-resolve-zone-numeric-to-seconds ()
  (should (= (jaunder--resolve-zone "-0500") (* -5 3600))))

(ert-deftest jaunder-resolve-zone-empty-is-local-nil ()
  (should (null (jaunder--resolve-zone nil)))
  (should (null (jaunder--resolve-zone "   "))))

;;; org->atom — publish time / timezone

(ert-deftest jaunder-org->atom-published-iana-dst-summer ()
  (should (equal (jaunder-entry-published
                  (jaunder-test--entry
                   (concat "#+DATE: [2026-07-01 Wed 09:00]\n"
                           "#+PROPERTY: JAUNDER_STATUS published\n"
                           "#+PROPERTY: JAUNDER_DATE_TZ America/New_York\n\nB\n")))
                 "2026-07-01T13:00:00Z")))

(ert-deftest jaunder-org->atom-published-iana-dst-winter ()
  (should (equal (jaunder-entry-published
                  (jaunder-test--entry
                   (concat "#+DATE: [2026-01-01 Thu 09:00]\n"
                           "#+PROPERTY: JAUNDER_STATUS published\n"
                           "#+PROPERTY: JAUNDER_DATE_TZ America/New_York\n\nB\n")))
                 "2026-01-01T14:00:00Z")))

(ert-deftest jaunder-org->atom-published-numeric-offset-string ()
  ;; G1 regression: a raw offset *string* is silently misread by `encode-time'
  ;; as UTC; the mapping must parse it to integer seconds.
  (should (equal (jaunder-entry-published
                  (jaunder-test--entry
                   (concat "#+DATE: [2026-07-01 Wed 09:00]\n"
                           "#+PROPERTY: JAUNDER_STATUS published\n"
                           "#+PROPERTY: JAUNDER_DATE_TZ -0500\n\nB\n")))
                 "2026-07-01T14:00:00Z")))

(ert-deftest jaunder-org->atom-published-numeric-offset-colon ()
  (should (equal (jaunder-entry-published
                  (jaunder-test--entry
                   (concat "#+DATE: [2026-07-01 Wed 09:00]\n"
                           "#+PROPERTY: JAUNDER_STATUS published\n"
                           "#+PROPERTY: JAUNDER_DATE_TZ -05:00\n\nB\n")))
                 "2026-07-01T14:00:00Z")))

(ert-deftest jaunder-org->atom-published-scheduled ()
  (should (equal (jaunder-entry-published
                  (jaunder-test--entry
                   (concat "#+DATE: [2026-07-01 Wed 09:00]\n"
                           "#+PROPERTY: JAUNDER_STATUS scheduled\n"
                           "#+PROPERTY: JAUNDER_DATE_TZ America/New_York\n\nB\n")))
                 "2026-07-01T13:00:00Z")))

(ert-deftest jaunder-org->atom-published-publish-now-is-nil ()
  ;; status=published with no #+DATE -> omit (server stamps).
  (should (null (jaunder-entry-published
                 (jaunder-test--entry
                  "#+PROPERTY: JAUNDER_STATUS published\n\nB\n")))))

(ert-deftest jaunder-org->atom-published-draft-is-nil ()
  ;; drafts carry no publish time even with a #+DATE.
  (should (null (jaunder-entry-published
                 (jaunder-test--entry
                  (concat "#+DATE: [2026-07-01 Wed 09:00]\n"
                          "#+PROPERTY: JAUNDER_STATUS draft\n"
                          "#+PROPERTY: JAUNDER_DATE_TZ America/New_York\n\nB\n"))))))

(ert-deftest jaunder-org->atom-published-missing-date-is-nil ()
  (should (null (jaunder-entry-published
                 (jaunder-test--entry
                  (concat "#+PROPERTY: JAUNDER_STATUS scheduled\n"
                          "#+PROPERTY: JAUNDER_DATE_TZ America/New_York\n\nB\n"))))))

;;; utc->org-date + machine-zone capture

(ert-deftest jaunder-utc->org-date-renders-in-zone ()
  ;; 13:00Z in America/New_York (EDT, -04:00) is 09:00 local.
  (should (equal (jaunder--utc->org-date "2026-07-01T13:00:00Z" "America/New_York")
                 "[2026-07-01 Wed 09:00]"))
  ;; Round-trips through the existing forward mapping.
  (should (equal (jaunder--org-date->utc
                  (jaunder--utc->org-date "2026-07-01T13:00:00Z" "America/New_York")
                  "America/New_York")
                 "2026-07-01T13:00:00Z")))

(ert-deftest jaunder-current-zone-name-is-nonempty ()
  (let ((z (jaunder--current-zone-name)))
    (should (stringp z))
    (should (> (length z) 0))))

(ert-deftest jaunder-current-zone-name-prefers-explicit-tz ()
  "A configured IANA zone outranks the host localtime link."
  (cl-letf (((symbol-function 'getenv)
             (lambda (name) (and (equal name "TZ") "America/Chicago"))))
           (should (equal (jaunder--current-zone-name) "America/Chicago"))))

(ert-deftest jaunder-current-zone-name-reads-localtime-zoneinfo-link ()
  "Without TZ, retain the named zone exposed by /etc/localtime."
  (cl-letf (((symbol-function 'getenv) (lambda (_name) nil))
            ((symbol-function 'file-symlink-p)
             (lambda (_path) "/usr/share/zoneinfo/Europe/Paris")))
           (should (equal (jaunder--current-zone-name) "Europe/Paris"))))

(ert-deftest jaunder-ensure-date-tz-captures-when-unset-and-preserves ()
  (with-temp-buffer
    (org-mode)
    (insert "#+TITLE: T\n\nBody.\n")
    ;; Unset → captured to something non-empty.
    (jaunder--ensure-date-tz)
    (let ((captured (jaunder--buffer-property "JAUNDER_DATE_TZ")))
      (should (stringp captured))
      (should (> (length captured) 0))
      ;; Already set → preserved verbatim (idempotent, no re-capture).
      (jaunder--set-property "JAUNDER_DATE_TZ" "Europe/Paris")
      (jaunder--ensure-date-tz)
      (should (equal (jaunder--buffer-property "JAUNDER_DATE_TZ") "Europe/Paris")))))

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

(ert-deftest jaunder-with-blog-binds-active-blog ()
  (let ((jaunder-blogs '(("/home/me/blog/" :base-url "https://a" :username "a"))))
    (jaunder--with-blog "/home/me/blog/post.org"
                        (should (equal (jaunder--active-base-url) "https://a"))
                        (should (equal (jaunder--active-username) "a")))))

(ert-deftest jaunder-active-accessors-error-without-active-blog ()
  ;; Outside `jaunder--with-blog' the accessors must signal, so a transport call
  ;; that forgot to establish request context fails loudly instead of using nil.
  (let ((jaunder--active-blog nil))
    (should-error (jaunder--active-base-url))
    (should-error (jaunder--active-username))))

;;; atom-entry -> xml serializer

(ert-deftest jaunder-atom-entry->xml-full-entry ()
  (let ((xml (jaunder--atom-entry->xml
              (jaunder--make-entry
               :title "My Post"
               :categories '("rust" "prog")
               :summary "An excerpt"
               :draft nil
               :content-type "text/org"
               :body "Body text"
               :published "2026-07-01T13:00:00Z"))))
    (should (string-match-p "<entry\\b" xml))
    (should (string-match-p "xmlns=\"http://www.w3.org/2005/Atom\"" xml))
    (should (string-match-p "<title>My Post</title>" xml))
    (should (string-match-p "<summary>An excerpt</summary>" xml))
    (should (string-match-p "<category term=\"rust\"" xml))
    (should (string-match-p "<category term=\"prog\"" xml))
    (should (string-match-p "<content type=\"text/org\">Body text</content>" xml))
    (should (string-match-p "<published>2026-07-01T13:00:00Z</published>" xml))))

(ert-deftest jaunder-atom-entry->xml-draft-marker ()
  (let ((xml (jaunder--atom-entry->xml
              (jaunder--make-entry :draft t :content-type "text/org" :body "b"))))
    (should (string-match-p "xmlns:app=\"http://www.w3.org/2007/app\"" xml))
    (should (string-match-p
             "<app:control><app:draft>yes</app:draft></app:control>" xml))))

(ert-deftest jaunder-atom-entry->xml-non-draft-omits-control ()
  (let ((xml (jaunder--atom-entry->xml
              (jaunder--make-entry :draft nil :content-type "text/org" :body "b"))))
    (should-not (string-match-p "app:draft" xml))
    (should-not (string-match-p "xmlns:app" xml))))

(ert-deftest jaunder-atom-entry->xml-omits-absent-optionals ()
  (let ((xml (jaunder--atom-entry->xml
              (jaunder--make-entry :content-type "text/org" :body "b"))))
    (should-not (string-match-p "<title>" xml))
    (should-not (string-match-p "<summary>" xml))
    (should-not (string-match-p "<published>" xml))
    (should-not (string-match-p "<category" xml))))

(ert-deftest jaunder-atom-entry->xml-escapes-text-and-attrs ()
  (let ((xml (jaunder--atom-entry->xml
              (jaunder--make-entry
               :title "Tom & Jerry <3 \"x\""
               :categories '("a&b")
               :content-type "text/org"
               :body "1 < 2 & 3 > 0"))))
    (should (string-match-p "<title>Tom &amp; Jerry &lt;3 &quot;x&quot;</title>" xml))
    (should (string-match-p "term=\"a&amp;b\"" xml))
    (should (string-match-p "1 &lt; 2 &amp; 3 &gt; 0" xml))
    ;; No raw unescaped ampersand leaked into text.
    (should-not (string-match-p "Tom & Jerry" xml))))

(ert-deftest jaunder-atom-entry->xml-empty-body-is-explicit-element ()
  (let ((xml (jaunder--atom-entry->xml
              (jaunder--make-entry :content-type "text/org" :body ""))))
    (should (string-match-p "<content type=\"text/org\"></content>" xml))))

(ert-deftest jaunder-atom-entry->xml-well-formed ()
  ;; Parse it back to prove well-formedness (libxml when available).
  (skip-unless (fboundp 'libxml-parse-xml-region))
  (let ((xml (jaunder--atom-entry->xml
              (jaunder--make-entry
               :title "T" :categories '("x") :summary "s" :draft t
               :content-type "text/org" :body "b <y> & z"
               :published "2026-07-01T13:00:00Z"))))
    (with-temp-buffer
      (insert xml)
      (should (consp (libxml-parse-xml-region (point-min) (point-max)))))))

;;; media upload

(ert-deftest jaunder-harvest-response-fields-content-src-and-type ()
  (skip-unless (fboundp 'libxml-parse-xml-region))
  (let ((xml (concat "<?xml version=\"1.0\" encoding=\"utf-8\"?>"
                     "<entry xmlns=\"http://www.w3.org/2005/Atom\">"
                     "<id>x</id><title>p.png</title>"
                     "<updated>2026-07-02T00:00:00Z</updated>"
                     "<published>2026-07-02T00:00:00Z</published>"
                     "<content type=\"image/png\""
                     " src=\"https://h/media/upload/ab/cd/abcd/p.png\"/>"
                     "<link rel=\"edit-media\""
                     " href=\"https://h/media/upload/ab/cd/abcd/p.png\"/>"
                     "</entry>")))
    (should (equal (cdr (assq 'content-src (jaunder--harvest-response-fields xml)))
                   "https://h/media/upload/ab/cd/abcd/p.png"))
    (should (equal (cdr (assq 'content-type (jaunder--harvest-response-fields xml)))
                   "image/png"))))

(ert-deftest jaunder-harvest-response-fields-slug-and-published ()
  (let ((xml (concat
              "<entry xmlns=\"http://www.w3.org/2005/Atom\""
              " xmlns:j=\"https://jaunder.org/ns/atompub\">"
              "<content type=\"text/org\">Body</content>"
              "<published>2026-07-01T13:00:00+00:00</published>"
              "<j:slug>my-post</j:slug></entry>")))
    (let ((fields (jaunder--harvest-response-fields xml)))
      (should (equal (cdr (assq 'slug fields)) "my-post"))
      (should (equal (cdr (assq 'published fields)) "2026-07-01T13:00:00+00:00"))
      (should (equal (cdr (assq 'content-type fields)) "text/org")))))

(ert-deftest jaunder-harvest-response-fields-absent-slug-published-are-nil ()
  ;; A content-only entry (no <j:slug>, no <published> — e.g. a draft, which
  ;; the server stamps <published> onto only when live) yields nil for both,
  ;; exercising the `(and NODE (dom-text NODE))' nil-guard branches.
  (let ((xml (concat
              "<entry xmlns=\"http://www.w3.org/2005/Atom\""
              " xmlns:j=\"https://jaunder.org/ns/atompub\">"
              "<content type=\"text/org\">Body</content></entry>")))
    (let ((fields (jaunder--harvest-response-fields xml)))
      (should (null (cdr (assq 'slug fields))))
      (should (null (cdr (assq 'published fields))))
      (should (equal (cdr (assq 'content-type fields)) "text/org")))))

(ert-deftest jaunder-media-content-type-is-deterministic ()
  (dolist (case '(("a.jpg" . "image/jpeg")
                  ("a.jpeg" . "image/jpeg")
                  ("a.png" . "image/png")
                  ("a.gif" . "image/gif")
                  ("a.webp" . "image/webp")
                  ("a.svg" . "image/svg+xml")
                  ("a.mp3" . "audio/mpeg")
                  ("a.ogg" . "audio/ogg")
                  ("a.oga" . "audio/ogg")
                  ("a.flac" . "audio/flac")
                  ("a.wav" . "audio/wav")
                  ("a.mp4" . "video/mp4")
                  ("a.webm" . "video/webm")
                  ("a.pdf" . "application/pdf")
                  ("A.PDF" . "application/pdf")
                  ("a.unknown" . "application/octet-stream")
                  ("extensionless" . "application/octet-stream")))
    (should (equal (jaunder--media-content-type (car case)) (cdr case)))))
(ert-deftest jaunder-harvest-response-fields-content-only-keeps-compatible-shape ()
  ;; Media upload responses are Entries too: adding D2's plural parse must not
  ;; make their absent Member metadata an error or alter their singular fields.
  (let* ((xml (concat "<entry xmlns=\"http://www.w3.org/2005/Atom\">"
                      "<content type=\"image/png\" src=\"https://h/image.png\"/>"
                      "</entry>"))
         (fields (jaunder--harvest-response-fields xml)))
    (should (equal (mapcar #'car fields)
                   '(content-src content-type slug published
                                 titles categories summaries content-nodes drafts
                                 published-values edit-uris slugs)))
    (should (equal (cdr (assq 'content-src fields)) "https://h/image.png"))
    (should (equal (cdr (assq 'content-type fields)) "image/png"))
    (should (null (cdr (assq 'slug fields))))
    (should (null (cdr (assq 'published fields))))
    (dolist (key '(titles categories summaries drafts published-values edit-uris slugs))
      (should (equal (cdr (assq key fields)) nil)))
    (should (= (length (cdr (assq 'content-nodes fields))) 1))))

(ert-deftest jaunder-harvest-response-fields-uses-only-direct-entry-metadata ()
  ;; XHTML body markup is content, not Atom metadata: a nested title/category
  ;; or link must never change the Member fields which D2 and D3 consume.
  (let* ((xml (concat
               "<entry xmlns=\"http://www.w3.org/2005/Atom\""
               " xmlns:app=\"http://www.w3.org/2007/app\""
               " xmlns:j=\"https://jaunder.org/ns/atompub\">"
               "<title>Entry title</title><category term=\"entry-category\"/>"
               "<summary>Entry summary</summary><published>2026-08-25T10:00:00Z</published>"
               "<link rel=\"edit\" href=\"https://h/posts/7\"/>"
               "<j:slug>entry-slug</j:slug><app:control><app:draft>yes</app:draft></app:control>"
               "<content type=\"xhtml\"><div xmlns=\"http://www.w3.org/1999/xhtml\">"
               "<title>Body title</title><category term=\"body-category\"/>"
               "<summary>Body summary</summary><published>body-time</published>"
               "<link rel=\"edit\" href=\"https://h/posts/8\"/><j:slug>body-slug</j:slug>"
               "<app:control><app:draft>no</app:draft></app:control></div></content>"
               "</entry>"))
         (fields (jaunder--harvest-response-fields xml)))
    (should (equal (cdr (assq 'titles fields)) '("Entry title")))
    (should (equal (cdr (assq 'categories fields)) '("entry-category")))
    (should (equal (cdr (assq 'summaries fields)) '("Entry summary")))
    (should (equal (cdr (assq 'drafts fields)) '("yes")))
    (should (equal (cdr (assq 'published-values fields)) '("2026-08-25T10:00:00Z")))
    (should (equal (cdr (assq 'edit-uris fields)) '("https://h/posts/7")))
    (should (equal (cdr (assq 'slugs fields)) '("entry-slug")))
    (should (= (length (cdr (assq 'content-nodes fields))) 1))))

(ert-deftest jaunder-harvest-response-fields-ignores-foreign-direct-elements ()
  ;; Namespace identity, not a familiar local name, makes Member metadata:
  ;; extension markup must not impersonate Atom, APP, or Jaunder fields.
  (let* ((xml (concat
               "<entry xmlns=\"http://www.w3.org/2005/Atom\""
               " xmlns:app=\"http://www.w3.org/2007/app\""
               " xmlns:j=\"https://jaunder.org/ns/atompub\""
               " xmlns:f=\"https://example.invalid/foreign\">"
               "<f:title>foreign title</f:title><title>Atom title</title>"
               "<f:category term=\"foreign\"/><category term=\"Atom category\"/>"
               "<f:summary>foreign summary</f:summary><summary>Atom summary</summary>"
               "<f:content type=\"text/markdown\">foreign body</f:content>"
               "<content type=\"text/org\">Atom body</content>"
               "<f:published>foreign time</f:published><published>Atom time</published>"
               "<f:link rel=\"edit\" href=\"https://h/foreign\"/>"
               "<link rel=\"edit\" href=\"https://h/atom\"/>"
               "<f:control><f:draft>foreign draft</f:draft></f:control>"
               "<app:control><f:draft>foreign child</f:draft><app:draft>yes</app:draft></app:control>"
               "<f:slug>foreign slug</f:slug><j:slug>atom-slug</j:slug></entry>"))
         (fields (jaunder--harvest-response-fields xml)))
    (should (equal (cdr (assq 'titles fields)) '("Atom title")))
    (should (equal (cdr (assq 'categories fields)) '("Atom category")))
    (should (equal (cdr (assq 'summaries fields)) '("Atom summary")))
    (should (equal (mapcar #'dom-text (cdr (assq 'content-nodes fields)))
                   '("Atom body")))
    (should (equal (cdr (assq 'published-values fields)) '("Atom time")))
    (should (equal (cdr (assq 'edit-uris fields)) '("https://h/atom")))
    (should (equal (cdr (assq 'drafts fields)) '("yes")))
    (should (equal (cdr (assq 'slugs fields)) '("atom-slug")))))

(ert-deftest jaunder-harvest-response-fields-resolves-arbitrary-prefixes ()
  ;; Atom, APP, and Jaunder namespace URIs remain valid when a Member chooses
  ;; noncanonical prefixes, including declarations on direct children.
  (let* ((xml (concat
               "<a:entry xmlns:a=\"http://www.w3.org/2005/Atom\""
               " xmlns:x=\"https://jaunder.org/ns/atompub\">"
               "<a:title>title</a:title><a:category term=\"category\"/>"
               "<a:summary>summary</a:summary><a:content type=\"text/org\">body</a:content>"
               "<a:published>2026-08-25T10:00:00Z</a:published>"
               "<a:link rel=\"edit\" href=\"https://h/post/1\"/>"
               "<p:control xmlns:p=\"http://www.w3.org/2007/app\"><p:draft>yes</p:draft></p:control><x:slug>slug</x:slug>"
               "</a:entry>"))
         (fields (jaunder--harvest-response-fields xml)))
    (should (equal (cdr (assq 'titles fields)) '("title")))
    (should (equal (cdr (assq 'categories fields)) '("category")))
    (should (equal (cdr (assq 'summaries fields)) '("summary")))
    (should (equal (cdr (assq 'published-values fields))
                   '("2026-08-25T10:00:00Z")))
    (should (equal (cdr (assq 'edit-uris fields)) '("https://h/post/1")))
    (should (equal (cdr (assq 'drafts fields)) '("yes")))
    (should (equal (cdr (assq 'slugs fields)) '("slug")))))

(ert-deftest jaunder-harvest-response-fields-preserves-direct-child-cardinality ()
  ;; D2 validates Member requirements later; the shared harvester must instead
  ;; retain every direct wire value so D3 can diagnose duplicates precisely.
  (let* ((xml (concat
               "<entry xmlns=\"http://www.w3.org/2005/Atom\""
               " xmlns:app=\"http://www.w3.org/2007/app\""
               " xmlns:j=\"https://jaunder.org/ns/atompub\">"
               "<title>first</title><title>second</title>"
               "<category term=\"alpha\"/><category term=\"beta\"/>"
               "<summary>one</summary><summary>two</summary>"
               "<content type=\"text/org\" src=\"https://h/first\"/>"
               "<content type=\"text/markdown\" src=\"https://h/second\"/>"
               "<app:control><app:draft>yes</app:draft><app:draft>no</app:draft></app:control>"
               "<app:control><app:draft>maybe</app:draft></app:control>"
               "<published>first-time</published><published>second-time</published>"
               "<link rel=\"edit\" href=\"https://h/posts/1\"/>"
               "<link rel=\"alternate\" href=\"https://h/posts/1/view\"/>"
               "<link rel=\"edit\" href=\"https://h/posts/2\"/>"
               "<j:slug>first-slug</j:slug><j:slug>second-slug</j:slug></entry>"))
         (fields (jaunder--harvest-response-fields xml)))
    (should (equal (cdr (assq 'titles fields)) '("first" "second")))
    (should (equal (cdr (assq 'categories fields)) '("alpha" "beta")))
    (should (equal (cdr (assq 'summaries fields)) '("one" "two")))
    (should (= (length (cdr (assq 'content-nodes fields))) 2))
    (should (equal (cdr (assq 'drafts fields)) '("yes" "no" "maybe")))
    (should (equal (cdr (assq 'published-values fields)) '("first-time" "second-time")))
    (should (equal (cdr (assq 'edit-uris fields))
                   '("https://h/posts/1" "https://h/posts/2")))
    (should (equal (cdr (assq 'slugs fields)) '("first-slug" "second-slug")))
    (should (equal (cdr (assq 'content-src fields)) "https://h/first"))
    (should (equal (cdr (assq 'content-type fields)) "text/org"))
    (should (equal (cdr (assq 'slug fields)) "first-slug"))
    (should (equal (cdr (assq 'published fields)) "first-time"))))


(defun jaunder-test--collect (org dir)
  "Collect media links from ORG with `default-directory' DIR."
  (with-temp-buffer
    (insert org)
    (org-mode)
    (setq default-directory dir)
    (jaunder--collect-media-links)))

(ert-deftest jaunder-media-collect-file-links-qualify-and-resolve ()
  ;; relative, ./-relative-with-desc, and absolute file: links all qualify.
  (let ((rs (jaunder-test--collect
             (concat "#+TITLE: T\n\nSee [[file:img/a.png]] and [[./b.JPG][alt]]"
                     " and [[file:/abs/c.gif]].\n")
             "/home/u/post/")))
    (should (equal (mapcar (lambda (r) (plist-get r :raw-link)) rs)
                   '("file:img/a.png" "./b.JPG" "file:/abs/c.gif")))
    (should (equal (mapcar (lambda (r) (plist-get r :path)) rs)
                   '("/home/u/post/img/a.png" "/home/u/post/b.JPG" "/abs/c.gif")))
    (should (equal (mapcar (lambda (r) (plist-get r :content-type)) rs)
                   '("image/png" "image/jpeg" "image/gif")))))

(ert-deftest jaunder-media-collects-local-files-and-excludes-non-local-links ()
  ;; Header-region, absolute HTTP, fuzzy, and block-contained links are excluded;
  ;; an arbitrary body-local file remains a candidate.
  (let ((rs (jaunder-test--collect
             (concat "#+DESCRIPTION: [[file:cover.png]]\n"
                     "\n"
                     "abs [[https://x/y.png]] "
                     "fuzzy [[a.png]] "
                     "doc [[file:notes.txt]]\n"
                     "#+begin_src org\n[[file:code.png]]\n#+end_src\n"
                     "#+begin_example\n[[file:ex.png]]\n#+end_example\n")
             "/d/")))
    (should (equal rs
                   '((:raw-link "file:notes.txt"
                                :content-type "application/octet-stream"
                                :path "/d/notes.txt"))))))

(ert-deftest jaunder-media-collect-uses-resolved-path-for-content-type ()
  (cl-letf (((symbol-function 'jaunder--org-body-links)
             (lambda ()
               '((:type "file" :path "opaque" :raw-link "file:opaque"
                        :file "/resolved/document.pdf")))))
           (should
            (equal (jaunder--collect-media-links)
                   '((:raw-link "file:opaque"
                                :content-type "application/pdf"
                                :path "/resolved/document.pdf"))))))

(ert-deftest jaunder-localize-media-aggregates-preflight-failures-before-upload ()
  (let* ((dir (make-temp-file "jt-preflight-" t))
         (missing (expand-file-name "missing.pdf" dir))
         (directory (expand-file-name "directory.pdf" dir))
         (unreadable (expand-file-name "unreadable.pdf" dir))
         (real-file-readable-p (symbol-function 'file-readable-p))
         (upload-calls 0))
    (unwind-protect
        (progn
          (make-directory directory)
          (with-temp-file unreadable (insert "PDF"))
          (cl-letf (((symbol-function 'file-readable-p)
                     (lambda (path)
                       (and (not (equal path unreadable))
                            (funcall real-file-readable-p path))))
                    ((symbol-function 'jaunder--upload-media)
                     (lambda (&rest _)
                       (setq upload-calls (1+ upload-calls)))))
                   (with-temp-buffer
                     (insert (format "#+TITLE: T\n\n[[file:%s]] [[file:%s]] [[file:%s]]\n"
                                     missing directory unreadable))
                     (org-mode)
                     (let* ((body (jaunder-entry-body (jaunder--org->atom)))
                            (err (should-error (jaunder--localize-media body)
                                               :type 'error))
                            (message (error-message-string err)))
                       (should
                        (string-prefix-p
                         "jaunder: media file(s) missing, unreadable, or not regular: "
                         message))
                       (dolist (path (list missing directory unreadable))
                         (should (string-match-p (regexp-quote path) message)))
                       (should (= upload-calls 0))))))
      (delete-directory dir t))))

(ert-deftest jaunder-media-substitute-single-and-desc ()
  (should (equal (jaunder--substitute-media
                  "a [[file:x.png]] b [[./y.png][alt]] c"
                  '("https://h/m/x.png" "https://h/m/y.png"))
                 "a [[https://h/m/x.png]] b [[https://h/m/y.png][alt]] c")))

(ert-deftest jaunder-media-substitute-collision-is-positional ()
  ;; same raw target, different resolved URLs -> each rewritten independently
  (should (equal (jaunder--substitute-media
                  "[[attachment:p.png]] and [[attachment:p.png]]"
                  '("https://h/m/aaa/p.png" "https://h/m/bbb/p.png"))
                 "[[https://h/m/aaa/p.png]] and [[https://h/m/bbb/p.png]]")))

(ert-deftest jaunder-media-substitute-same-file-same-url ()
  ;; one file behind two links -> caller passes the same URL twice; both rewrite
  (should (equal (jaunder--substitute-media
                  "[[file:x.png]] then [[file:x.png]]"
                  '("https://h/m/x.png" "https://h/m/x.png"))
                 "[[https://h/m/x.png]] then [[https://h/m/x.png]]")))

(ert-deftest jaunder-media-substitute-no-links-is-noop ()
  (should (equal (jaunder--substitute-media
                  "plain [[https://x/y.png]] and [[fuzzy-target]] only" nil)
                 "plain [[https://x/y.png]] and [[fuzzy-target]] only")))

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

(ert-deftest jaunder-upload-media-errors-on-non-2xx ()
  (cl-letf (((symbol-function 'jaunder--http-request)
             (lambda (&rest _) '(:status 500 :body "boom"))))
           (let ((jaunder--active-blog '(:base-url "http://x" :username "alice")))
             (should-error (jaunder--upload-media "/tmp/x.png" "image/png") :type 'error))))

(ert-deftest jaunder-media-link-p-qualifies-file-and-attachment-types ()
  ;; Local-path link type is the eligibility boundary; media type and filesystem
  ;; state are handled after resolution.
  (should (equal
           (mapcar #'jaunder--media-link-p
                   '((:type "file" :path "document.pdf")
                     (:type "attachment" :path "recording.flac")
                     (:type "https" :path "//x/c.png")
                     (:type "file" :path "extensionless")
                     (:type "fuzzy" :path "e.png")))
           '(t t nil t nil))))

;;; org link primitives (jaunder-org)

(ert-deftest jaunder-org-link->record-neutral-fields ()
  ;; An org-element link becomes a neutral plist: :type/:path/:raw-link, and
  ;; :file resolved (absolute) for a local file:, nil for a non-local link.
  (with-temp-buffer
    (insert "[[file:pic.png][a pic]] [[https://x/y.png]]")
    (org-mode)
    (let* ((links (org-element-map (org-element-parse-buffer) 'link #'identity))
           (file-rec (jaunder--org-link->record (nth 0 links)))
           (http-rec (jaunder--org-link->record (nth 1 links))))
      (should (equal (plist-get file-rec :type) "file"))
      (should (equal (plist-get file-rec :path) "pic.png"))
      (should (equal (plist-get file-rec :raw-link) "file:pic.png"))
      (should (equal (plist-get file-rec :file) (expand-file-name "pic.png")))
      (should (equal (plist-get http-rec :type) "https"))
      (should (null (plist-get http-rec :file))))))

(ert-deftest jaunder-org-link-file-unescapes-local-target-once ()
  ;; Local Media Copies retain decoded leaves, while their native Org links use
  ;; percent encoding and may preserve a URL fragment. Keep raw spelling for
  ;; rewriting; strip the fragment and decode exactly once for filesystem lookup
  ;; (not `%2525' twice).
  (let* ((directory (make-temp-file "jt-org-link-" t))
         (names '("source image.png" "literal%25.png" "画像.png"))
         (targets '("source%20image.png#view" "literal%2525.png"
                    "%E7%94%BB%E5%83%8F.png")))
    (unwind-protect
        (progn
          (dolist (name names)
            (with-temp-file (expand-file-name name directory) (insert name)))
          (with-temp-buffer
            (setq default-directory directory)
            (insert (mapconcat (lambda (target) (format "[[file:%s]]" target))
                               targets " "))
            (org-mode)
            (let ((records (mapcar #'jaunder--org-link->record
                                   (org-element-map (org-element-parse-buffer)
                                                    'link #'identity))))
              (should (equal (mapcar (lambda (record) (plist-get record :path))
                                     records)
                             targets))
              (should (equal (mapcar (lambda (record) (plist-get record :file))
                                     records)
                             (mapcar (lambda (name) (expand-file-name name directory))
                                     names))))))
      (delete-directory directory t))))

(ert-deftest jaunder-org-body-links-returns-body-records-in-order ()
  ;; Links after the header block come back as neutral records, in document
  ;; order; header-block keyword lines contribute none.
  (with-temp-buffer
    (insert "#+TITLE: T\n#+KEYWORDS: x\n\n[[file:a.png]] and [[file:b.gif]]\n")
    (org-mode)
    (should (equal (mapcar (lambda (r) (plist-get r :path)) (jaunder--org-body-links))
                   '("a.png" "b.gif")))))

(ert-deftest jaunder-org-substitute-links-rewrites-selected-by-predicate ()
  ;; The PREDICATE (on neutral records) selects which links are rewritten to the
  ;; paired URLs, in order; a description is preserved and non-selected links are
  ;; left untouched.
  (should (equal
           (jaunder--org-substitute-links
            "see [[file:a.png][pic]] and [[https://x/keep]] and [[file:b.png]]"
            (lambda (rec) (equal (plist-get rec :type) "file"))
            '("http://s/a" "http://s/b"))
           "see [[http://s/a][pic]] and [[https://x/keep]] and [[http://s/b]]")))

(ert-deftest jaunder-localize-media-handles-files-attachments-and-deduplication ()
  ;; Exercise the complete pure localization boundary: resolved paths determine
  ;; MIME, repeated files upload once, descriptions survive, and source stays local.
  (let* ((dir (make-temp-file "jt-localize-" t))
         (attach-dir (expand-file-name "attachments" dir))
         (pdf (expand-file-name "document.pdf" dir))
         (flac (expand-file-name "recording.flac" attach-dir))
         (jaunder-warn-untracked-media nil)
         calls)
    (unwind-protect
        (progn
          (make-directory attach-dir)
          (with-temp-file pdf (insert "PDF"))
          (with-temp-file flac (insert "FLAC"))
          (cl-letf (((symbol-function 'jaunder--upload-media)
                     (lambda (path content-type)
                       (push (list path content-type) calls)
                       (if (equal path pdf)
                           "https://h/media/document.pdf"
                         "https://h/media/recording.flac"))))
                   (with-temp-buffer
                     (setq default-directory dir)
                     (org-mode)
                     (insert
                      (format
                       (concat "#+TITLE: T\n\n"
                               "[[file:document.pdf][download]] and "
                               "[[file:document.pdf][again]]\n"
                               "* Audio\n:PROPERTIES:\n:DIR: %s\n:END:\n\n"
                               "[[attachment:recording.flac][listen]]\n")
                       attach-dir))
                     (let* ((body (jaunder-entry-body (jaunder--org->atom)))
                            (before (buffer-string))
                            (out (jaunder--localize-media body)))
                       (should (= (length calls) 2))
                       (should (member (list pdf "application/pdf") calls))
                       (should (member (list flac "audio/flac") calls))
                       (should
                        (string-match-p
                         (regexp-quote "[[https://h/media/document.pdf][download]]")
                         out))
                       (should
                        (string-match-p
                         (regexp-quote "[[https://h/media/document.pdf][again]]")
                         out))
                       (should
                        (string-match-p
                         (regexp-quote "[[https://h/media/recording.flac][listen]]")
                         out))
                       (should-not (string-match-p "file:document\\.pdf" out))
                       (should-not (string-match-p "attachment:recording\\.flac" out))
                       (should (equal (buffer-string) before))))))
      (delete-directory dir t))))

(ert-deftest jaunder-localize-media-no-candidates-is-noop ()
  (let (called)
    (cl-letf (((symbol-function 'jaunder--upload-media)
               (lambda (&rest _) (setq called t) "u")))
             (with-temp-buffer
               (insert "#+TITLE: T\n\nJust prose, [[https://x/y.png]] absolute.\n")
               (org-mode)
               (let ((body (jaunder-entry-body (jaunder--org->atom))))
                 (should (equal (jaunder--localize-media body) body))
                 (should-not called))))))

;;; buffer read/write helpers

(ert-deftest jaunder-set-property-replaces-existing ()
  (with-temp-buffer
    (org-mode)
    (insert "#+TITLE: T\n#+PROPERTY: JAUNDER_ID 7\n\nBody.\n")
    (jaunder--set-property "JAUNDER_ID" "42")
    (should (equal (jaunder--buffer-property "JAUNDER_ID") "42"))
    (should (string-match-p "Body\\." (buffer-string)))
    (should-not (string-match-p "JAUNDER_ID 7" (buffer-string)))))

(ert-deftest jaunder-set-property-inserts-into-header-block ()
  (with-temp-buffer
    (org-mode)
    (insert "#+TITLE: T\n\nBody.\n")
    (jaunder--set-property "JAUNDER_SLUG" "my-post")
    (should (equal (jaunder--buffer-property "JAUNDER_SLUG") "my-post"))
    ;; Inserted in the header block, body untouched.
    (should (string-match-p "\\`#\\+TITLE: T\n#\\+PROPERTY: JAUNDER_SLUG my-post\n\nBody\\."
                            (buffer-string)))))

(ert-deftest jaunder-set-keyword-replaces-and-inserts ()
  (with-temp-buffer
    (org-mode)
    (insert "#+TITLE: T\n\nBody.\n")
    (jaunder--set-keyword "DATE" "[2026-07-01 Wed 09:00]")
    (should (equal (jaunder--buffer-keyword "DATE") "[2026-07-01 Wed 09:00]"))
    (jaunder--set-keyword "DATE" "[2027-01-01 Fri 00:00]")
    (should (equal (jaunder--buffer-keyword "DATE") "[2027-01-01 Fri 00:00]"))))

;;; publish validation + Location->id + force-draft

(ert-deftest jaunder-validate-publish-rejects-empty-body ()
  (let ((e (jaunder--make-entry :body "   \n")))
    (should-error (jaunder--validate-publish e "published" nil nil))))

(ert-deftest jaunder-validate-publish-scheduled-needs-future ()
  (let ((e (jaunder--make-entry :body "x")))
    (should-error (jaunder--validate-publish e "scheduled" "[2000-01-01 Sat 00:00]" nil))
    ;; A far-future date passes.
    (should-not (jaunder--validate-publish e "scheduled" "[2999-01-01 Tue 00:00]" nil))))

(ert-deftest jaunder-location->id-extracts-numeric-tail ()
  (should (equal (jaunder--location->id "https://x/atompub/alice/posts/42") "42"))
  (should (equal (jaunder--location->id "https://x/atompub/alice/posts/42/") "42"))
  (should (null (jaunder--location->id nil))))

(ert-deftest jaunder-force-draft-sets-draft-and-clears-published ()
  ;; A dated, non-draft entry forced to draft must not carry <published>:
  ;; the serializer emits <published> whenever the slot is set, independent of
  ;; the draft flag, so force-draft has to nil it (spec invariant).
  (let ((e (jaunder--make-entry :body "x" :draft nil :content-type "text/org"
                                :published "2026-07-01T13:00:00Z")))
    (jaunder--force-draft e)
    (should (jaunder-entry-draft e))
    (should (null (jaunder-entry-published e)))
    ;; And the wire entry indeed omits <published>.
    (should-not (string-match-p "<published>" (jaunder--atom-entry->xml e)))))

;;; rename temp draft to <slug>.org

(ert-deftest jaunder-rename-to-slug-renames-and-handles-collision ()
  (let ((dir (make-temp-file "jaunder-rn-" t)))
    (unwind-protect
        (let ((tmp (expand-file-name "draft-20260101T000000.org" dir)))
          (with-temp-file tmp (insert "x"))
          (let ((buf (find-file-noselect tmp)))
            (unwind-protect
                (with-current-buffer buf
                  (let ((p (jaunder--rename-to-slug "my-post")))
                    (should (equal (file-name-nondirectory p) "my-post.org"))
                    (should (equal (buffer-file-name) p))
                    (should (file-exists-p p))
                    (should-not (file-exists-p tmp))
                    ;; Idempotent: renaming to the same slug is a no-op.
                    (should (equal (jaunder--rename-to-slug "my-post") p))))
              (kill-buffer buf)))
          ;; Collision: a second post with the same slug gets -1.
          (let ((tmp2 (expand-file-name "draft-20260101T000001.org" dir)))
            (with-temp-file tmp2 (insert "y"))
            (let ((buf2 (find-file-noselect tmp2)))
              (unwind-protect
                  (with-current-buffer buf2
                    (should (equal (file-name-nondirectory
                                    (jaunder--rename-to-slug "my-post"))
                                   "my-post-1.org")))
                (kill-buffer buf2)))))
      (delete-directory dir t))))

(defun jaunder-test--response (status headers body)
  "Build a `jaunder--http-request'-shaped plist for tests."
  (list :status status
        :headers (mapcar (lambda (h) (cons (downcase (car h)) (cdr h))) headers)
        :body body))

(ert-deftest jaunder-write-back-create-writes-id-first ()
  (with-temp-buffer
    (org-mode)
    (insert "#+TITLE: T\n#+PROPERTY: JAUNDER_STATUS published\n\nBody.\n")
    (set-visited-file-name (make-temp-file "jaunder-wb-" nil ".org") nil t)
    (unwind-protect
        (let ((resp (jaunder-test--response
                     201
                     '(("Location" . "https://x/atompub/alice/posts/42")
                       ("ETag" . "\"abc\""))
                     (concat "<entry xmlns=\"http://www.w3.org/2005/Atom\""
                             " xmlns:j=\"https://jaunder.org/ns/atompub\">"
                             "<content type=\"text/org\">Body</content>"
                             "<published>2026-07-01T13:00:00+00:00</published>"
                             "<j:slug>my-post</j:slug></entry>"))))
          (should (equal (jaunder--write-back resp t) "my-post"))
          (should (equal (jaunder--buffer-property "JAUNDER_ID") "42"))
          (should (equal (jaunder--buffer-property "JAUNDER_SLUG") "my-post"))
          (should (equal (jaunder--buffer-property "JAUNDER_SYNCED") "\"abc\""))
          ;; The server's <published> offset is dropped to the canonical UTC
          ;; instant (tz-independent, so deterministic across machines).
          (should (equal (jaunder--buffer-property "JAUNDER_DATE_UTC")
                         "2026-07-01T13:00:00Z"))
          ;; publish-now (no author #+DATE:) → #+DATE: rendered from server time.
          (should (jaunder--buffer-keyword "DATE")))
      (when (buffer-file-name) (delete-file (buffer-file-name))))))

(ert-deftest jaunder-write-back-update-keeps-id ()
  (with-temp-buffer
    (org-mode)
    (insert "#+TITLE: T\n#+PROPERTY: JAUNDER_ID 7\n#+DATE: [2026-07-01 Wed 09:00]\n\nBody.\n")
    (set-visited-file-name (make-temp-file "jaunder-wb-" nil ".org") nil t)
    (unwind-protect
        (let ((resp (jaunder-test--response
                     200 '(("ETag" . "\"z\""))
                     (concat "<entry xmlns=\"http://www.w3.org/2005/Atom\""
                             " xmlns:j=\"https://jaunder.org/ns/atompub\">"
                             "<content type=\"text/org\">Body</content>"
                             "<j:slug>my-post</j:slug></entry>"))))
          (jaunder--write-back resp nil)     ; created = nil (update)
          (should (equal (jaunder--buffer-property "JAUNDER_ID") "7"))  ; unchanged
          (should (equal (jaunder--buffer-property "JAUNDER_SYNCED") "\"z\"")))
      (when (buffer-file-name) (delete-file (buffer-file-name))))))

(ert-deftest jaunder-new-post-writes-timestamped-draft ()
  (let ((dir (make-temp-file "jaunder-np-" t)))
    (unwind-protect
        (let ((path (jaunder--new-post-in dir "20260703T101500")))
          (should (equal (file-name-nondirectory path) "draft-20260703T101500.org"))
          (should (file-exists-p path))
          (let ((buf (find-file-noselect path)))
            (unwind-protect
                (with-current-buffer buf
                  (should (equal (jaunder--buffer-property "JAUNDER_STATUS") "draft"))
                  (should (jaunder--buffer-keyword "TITLE"))   ; present (may be empty)
                  (should (jaunder--buffer-keyword "DATE")))
              (kill-buffer buf))))
      (delete-directory dir t))))

;;; Publish-time warnings (shared idiom) ----------------------------------

(defmacro jaunder-test--capturing-warnings (&rest body)
  "Run BODY with `display-warning' captured; return the list of (TYPE MSG LEVEL).
Lets the warning tests assert on emitted warnings without touching the real
`*Warnings*' buffer."
  (declare (indent 0))
  `(let (jaunder-test--warnings)
     (cl-letf (((symbol-function 'display-warning)
                (lambda (type message &optional level &rest _)
                  (push (list type message level) jaunder-test--warnings))))
              ,@body)
     (nreverse jaunder-test--warnings)))

;;; #217 — zone-mismatch warning

(ert-deftest jaunder-zone-offset-p-recognizes-offsets ()
  (should (jaunder--zone-offset-p "-0400"))
  (should (jaunder--zone-offset-p "+0000"))
  (should-not (jaunder--zone-offset-p "America/New_York"))
  (should-not (jaunder--zone-offset-p nil)))

(ert-deftest jaunder-warn-zone-mismatch-fires-on-difference ()
  ;; AC-217a: recorded IANA zone differs from the machine's current zone.
  (cl-letf (((symbol-function 'jaunder--current-zone-name)
             (lambda () "Europe/London")))
           (let ((warnings (jaunder-test--capturing-warnings
                            (jaunder--warn-zone-mismatch "America/New_York"))))
             (should (= (length warnings) 1))
             (pcase-let ((`(,type ,message ,level) (car warnings)))
               (should (eq type 'jaunder))
               (should (eq level :warning))
               (should (string-prefix-p "jaunder: " message))
               (should (string-match-p "America/New_York" message))
               (should (string-match-p "Europe/London" message))))))

(ert-deftest jaunder-warn-zone-mismatch-silent-when-unset ()
  ;; AC-217b: no recorded zone yet (captured this publish) → nothing to warn about.
  (cl-letf (((symbol-function 'jaunder--current-zone-name)
             (lambda () "Europe/London")))
           (should-not (jaunder-test--capturing-warnings
                        (jaunder--warn-zone-mismatch nil)))))

(ert-deftest jaunder-warn-zone-mismatch-silent-when-equal ()
  ;; AC-217c (IANA): recorded == current.
  (cl-letf (((symbol-function 'jaunder--current-zone-name)
             (lambda () "America/New_York")))
           (should-not (jaunder-test--capturing-warnings
                        (jaunder--warn-zone-mismatch "America/New_York")))))

(ert-deftest jaunder-warn-zone-mismatch-silent-both-offsets ()
  ;; AC-217c (offset): two numeric offsets differ only across DST on one machine.
  (cl-letf (((symbol-function 'jaunder--current-zone-name)
             (lambda () "-0400")))
           (should-not (jaunder-test--capturing-warnings
                        (jaunder--warn-zone-mismatch "-0500")))))

(ert-deftest jaunder-warn-zone-mismatch-suppressed ()
  ;; AC-217d: the defcustom silences it even on a real difference.
  (cl-letf (((symbol-function 'jaunder--current-zone-name)
             (lambda () "Europe/London")))
           (let ((jaunder-warn-zone-mismatch nil))
             (should-not (jaunder-test--capturing-warnings
                          (jaunder--warn-zone-mismatch "America/New_York"))))))

;;; #206 — untracked-media warning

(ert-deftest jaunder-warn-untracked-media-one-per-untracked ()
  ;; AC-206a: one committed + one untracked → exactly one warning, the untracked.
  (cl-letf (((symbol-function 'jaunder--git-toplevel) (lambda (_dir) "/repo"))
            ((symbol-function 'jaunder--git-tracked-p)
             (lambda (_top path) (equal path "/repo/a.png"))))
           (let ((warnings (jaunder-test--capturing-warnings
                            (jaunder--warn-untracked-media
                             (list (list :path "/repo/a.png")
                                   (list :path "/repo/b.png"))))))
             (should (= (length warnings) 1))
             (should (eq (nth 0 (car warnings)) 'jaunder))
             (should (string-prefix-p "jaunder: " (nth 1 (car warnings))))
             (should (string-match-p "/repo/b.png" (nth 1 (car warnings)))))))

(ert-deftest jaunder-warn-untracked-media-all-tracked ()
  ;; AC-206d
  (cl-letf (((symbol-function 'jaunder--git-toplevel) (lambda (_dir) "/repo"))
            ((symbol-function 'jaunder--git-tracked-p) (lambda (_top _path) t)))
           (should-not (jaunder-test--capturing-warnings
                        (jaunder--warn-untracked-media (list (list :path "/repo/a.png")))))))

(ert-deftest jaunder-warn-untracked-media-skips-non-repo ()
  ;; AC-206e: no repo (or no git) → skip entirely.
  (cl-letf (((symbol-function 'jaunder--git-toplevel) (lambda (_dir) nil)))
           (should-not (jaunder-test--capturing-warnings
                        (jaunder--warn-untracked-media (list (list :path "/x/a.png")))))))

(ert-deftest jaunder-warn-untracked-media-suppressed ()
  ;; AC-206f
  (cl-letf (((symbol-function 'jaunder--git-toplevel) (lambda (_dir) "/repo"))
            ((symbol-function 'jaunder--git-tracked-p) (lambda (_top _path) nil)))
           (let ((jaunder-warn-untracked-media nil))
             (should-not (jaunder-test--capturing-warnings
                          (jaunder--warn-untracked-media (list (list :path "/repo/a.png"))))))))

(ert-deftest jaunder-warn-untracked-media-dedups ()
  ;; AC-206g: the same untracked path referenced twice warns once.
  (cl-letf (((symbol-function 'jaunder--git-toplevel) (lambda (_dir) "/repo"))
            ((symbol-function 'jaunder--git-tracked-p) (lambda (_top _path) nil)))
           (let ((warnings (jaunder-test--capturing-warnings
                            (jaunder--warn-untracked-media
                             (list (list :path "/repo/a.png")
                                   (list :path "/repo/a.png"))))))
             (should (= (length warnings) 1)))))

(ert-deftest jaunder-git-tracked-p-real-repo ()
  ;; AC-206b/c deterministic: pin git's actual exit code for gitignored and
  ;; outside-tree paths (and toplevel resolution from a subdirectory), rather
  ;; than assuming it.  Staging (git add) suffices for `ls-files --error-unmatch',
  ;; so no commit/identity setup is needed.
  (skip-unless (executable-find "git"))
  ;; Strip inherited GIT_* vars (a pre-commit/CI hook exports GIT_DIR /
  ;; GIT_WORK_TREE) so the temp repo's git subprocesses stay hermetic and do not
  ;; resolve against the ambient work tree.
  (let* ((process-environment
          (seq-remove (lambda (v) (string-prefix-p "GIT_" v))
                      process-environment))
         (root (file-truename (make-temp-file "jaunder-git-" t)))
         (default-directory root))
    (unwind-protect
        (progn
          (should (zerop (call-process "git" nil nil nil "init")))
          (with-temp-file (expand-file-name "tracked.png" root) (insert "x"))
          (should (zerop (call-process "git" nil nil nil "add" "tracked.png")))
          (with-temp-file (expand-file-name ".gitignore" root) (insert "ignored.png\n"))
          (with-temp-file (expand-file-name "ignored.png" root) (insert "y"))
          (let ((outside (make-temp-file "jaunder-outside-" nil ".png")))
            (unwind-protect
                (progn
                  (should (jaunder--git-tracked-p
                           root (expand-file-name "tracked.png" root)))
                  (should-not (jaunder--git-tracked-p
                               root (expand-file-name "ignored.png" root)))
                  (should-not (jaunder--git-tracked-p root outside)))
              (delete-file outside)))
          (let ((sub (expand-file-name "sub/deep" root)))
            (make-directory sub t)
            (should (equal (file-truename (jaunder--git-toplevel sub))
                           (file-truename root)))))
      (delete-directory root t))))

;;; #216 — missing-format-media-type warning

(defconst jaunder-test--service-doc-with-feature
  (concat "<?xml version=\"1.0\"?>"
          "<service xmlns=\"http://www.w3.org/2007/app\""
          " xmlns:j=\"https://jaunder.org/ns/atompub\">"
          "<workspace><j:extension version=\"1\""
          " features=\"format-media-type slug\"/></workspace></service>")
  "A service document advertising the format-media-type feature.")

(ert-deftest jaunder-parse-service-features-reads-features-attr ()
  (should (equal (jaunder--parse-service-features
                  jaunder-test--service-doc-with-feature)
                 '("format-media-type" "slug"))))

(ert-deftest jaunder-parse-service-features-absent-is-empty ()
  ;; Parses fine but advertises nothing → empty list, not `unknown'.
  (should (equal (jaunder--parse-service-features
                  (concat "<service xmlns=\"http://www.w3.org/2007/app\">"
                          "<workspace/></service>"))
                 '())))

(ert-deftest jaunder-parse-service-features-ignores-incidental-text ()
  ;; AC-216e: the token in a title text node is not the `features' attribute.
  (should-not
   (member "format-media-type"
           (jaunder--parse-service-features
            (concat "<service xmlns=\"http://www.w3.org/2007/app\">"
                    "<workspace><atom:title"
                    " xmlns:atom=\"http://www.w3.org/2005/Atom\">"
                    "format-media-type</atom:title></workspace></service>")))))

(ert-deftest jaunder-parse-service-features-unparseable-is-unknown ()
  ;; AC-216d: a 2xx body that is not parseable XML → unknown, not "absent".
  (should (eq (jaunder--parse-service-features "garbage, not xml") 'unknown)))

(ert-deftest jaunder-warn-missing-fmt-fires-when-absent ()
  ;; AC-216a
  (let ((jaunder--service-doc-cache nil))
    (cl-letf (((symbol-function 'jaunder--fetch-service-features)
               (lambda (_base) '("slug"))))
             (let ((warnings (jaunder-test--capturing-warnings
                              (jaunder--warn-missing-format-media-type "https://blog"))))
               (should (= (length warnings) 1))
               (should (eq (nth 0 (car warnings)) 'jaunder))
               (should (string-prefix-p "jaunder: " (nth 1 (car warnings))))
               (should (string-match-p "format-media-type" (nth 1 (car warnings))))
               (should (string-match-p "https://blog" (nth 1 (car warnings))))))))

(ert-deftest jaunder-warn-missing-fmt-caches-once-per-blog ()
  ;; AC-216b: a second publish neither re-warns nor re-fetches.
  (let ((jaunder--service-doc-cache nil) (fetches 0))
    (cl-letf (((symbol-function 'jaunder--fetch-service-features)
               (lambda (_base) (cl-incf fetches) '("slug"))))
             (let ((first (jaunder-test--capturing-warnings
                           (jaunder--warn-missing-format-media-type "https://blog")))
                   (second (jaunder-test--capturing-warnings
                            (jaunder--warn-missing-format-media-type "https://blog"))))
               (should (= (length first) 1))
               (should (null second))
               (should (= fetches 1))))))

(ert-deftest jaunder-warn-missing-fmt-silent-when-present ()
  ;; AC-216c
  (let ((jaunder--service-doc-cache nil))
    (cl-letf (((symbol-function 'jaunder--fetch-service-features)
               (lambda (_base) '("format-media-type" "slug"))))
             (should-not (jaunder-test--capturing-warnings
                          (jaunder--warn-missing-format-media-type "https://blog"))))))

(ert-deftest jaunder-warn-missing-fmt-unknown-not-cached ()
  ;; AC-216d: unknown → no warning and no cache entry (a later publish retries).
  (let ((jaunder--service-doc-cache nil))
    (cl-letf (((symbol-function 'jaunder--fetch-service-features)
               (lambda (_base) 'unknown)))
             (should-not (jaunder-test--capturing-warnings
                          (jaunder--warn-missing-format-media-type "https://blog")))
             (should-not (assoc "https://blog" jaunder--service-doc-cache)))))

(ert-deftest jaunder-fetch-service-features-catches-signal ()
  ;; AC-216d seam: a transport signal becomes `unknown', never propagates.
  (cl-letf (((symbol-function 'jaunder--http-request)
             (lambda (&rest _) (error "boom"))))
           (should (eq (jaunder--fetch-service-features "https://blog") 'unknown))))

(ert-deftest jaunder-warn-missing-fmt-suppressed ()
  ;; AC-216f: disabled → no fetch, no warning.
  (let ((jaunder--service-doc-cache nil) (fetches 0))
    (cl-letf (((symbol-function 'jaunder--fetch-service-features)
               (lambda (_base) (cl-incf fetches) '("slug"))))
             (let ((jaunder-warn-missing-format-media-type nil))
               (should-not (jaunder-test--capturing-warnings
                            (jaunder--warn-missing-format-media-type "https://blog")))
               (should (= fetches 0))))))

;;; Cross-cutting shared-idiom tests

(ert-deftest jaunder-publish-warnings-are-independent ()
  ;; AC-S2: suppressing one warning leaves the other two firing.
  (let ((jaunder--service-doc-cache nil))
    (cl-letf (((symbol-function 'jaunder--current-zone-name)
               (lambda () "Europe/London"))
              ((symbol-function 'jaunder--git-toplevel) (lambda (_dir) "/repo"))
              ((symbol-function 'jaunder--git-tracked-p) (lambda (_top _path) nil))
              ((symbol-function 'jaunder--fetch-service-features)
               (lambda (_base) '("slug"))))
             (let* ((jaunder-warn-zone-mismatch nil)
                    (msgs (mapcar
                           (lambda (w) (nth 1 w))
                           (jaunder-test--capturing-warnings
                            (jaunder--warn-zone-mismatch "America/New_York")
                            (jaunder--warn-untracked-media
                             (list (list :path "/repo/a.png")))
                            (jaunder--warn-missing-format-media-type "https://blog")))))
               (should (= (length msgs) 2))
               (should-not (seq-find (lambda (m) (string-match-p "timezone" m)) msgs))
               (should (seq-find (lambda (m) (string-match-p "not tracked" m)) msgs))
               (should (seq-find (lambda (m) (string-match-p "format-media-type" m)) msgs))))))

(ert-deftest jaunder-publish-request-identical-with-warnings ()
  ;; AC-S1: the publish request/return is byte-identical whether the warnings
  ;; fire or are all suppressed — they are side-effect-free on the publish path.
  (let* ((dir (file-truename (make-temp-file "jaunder-s1-" t)))
         (file (expand-file-name "post.org" dir))
         (jaunder-blogs (list (cons dir (list :base-url "https://blog.example"
                                              :username "alice"))))
         (captured nil))
    (unwind-protect
        (cl-letf (((symbol-function 'jaunder--current-zone-name)
                   (lambda () "Europe/London"))
                  ((symbol-function 'jaunder--fetch-service-features)
                   (lambda (_base) '("slug")))
                  ((symbol-function 'jaunder--write-back) (lambda (&rest _) nil))
                  ((symbol-function 'jaunder--http-request)
                   (lambda (method _url &optional body &rest _)
                     (when (member method '("POST" "PUT"))
                       (setq captured body))
                     (jaunder-test--response 201 nil ""))))
                 (cl-flet ((do-publish ()
                             (with-temp-buffer
                               (org-mode)
                               (insert (concat "#+TITLE: T\n"
                                               "#+PROPERTY: JAUNDER_DATE_TZ America/New_York\n"
                                               "\nBody.\n"))
                               (set-visited-file-name file nil t)
                               (setq captured nil)
                               (jaunder-publish)
                               (set-buffer-modified-p nil)
                               captured)))
                          ;; All three warnings WANT to fire here (recorded zone differs,
                          ;; service doc lacks the feature).
                          (let ((body-enabled (let ((jaunder--service-doc-cache nil))
                                                (do-publish)))
                                (body-suppressed
                                 (let ((jaunder-warn-zone-mismatch nil)
                                       (jaunder-warn-untracked-media nil)
                                       (jaunder-warn-missing-format-media-type nil)
                                       (jaunder--service-doc-cache nil))
                                   (do-publish))))
                            (should (stringp body-enabled))
                            (should (equal body-enabled body-suppressed)))))
      (delete-directory dir t))))

;;; Review follow-ups

(ert-deftest jaunder-parse-service-features-non-service-root-is-unknown ()
  ;; A 2xx HTML/error page from a proxy parses to a non-service root → unknown
  ;; (skip + no cache), not a false "feature absent".
  (should (eq (jaunder--parse-service-features
               "<html><body>Error 500</body></html>")
              'unknown)))

(ert-deftest jaunder-fetch-service-features-non-2xx-is-unknown ()
  ;; AC-216d: a 4xx/5xx status → unknown (never a "feature absent").
  (cl-letf (((symbol-function 'jaunder--http-request)
             (lambda (&rest _) (jaunder-test--response 404 nil "nope"))))
           (should (eq (jaunder--fetch-service-features "https://blog") 'unknown))))

(ert-deftest jaunder-git-toplevel-skips-on-unenterable-dir ()
  ;; Best-effort: an unenterable `default-directory' must not signal on the
  ;; publish path — the helper returns nil (skip), never errors.
  (should-not (jaunder--git-toplevel "/jaunder-no-such-dir-xyz/")))

;;; #79 — create idempotency key + auto-retry

(ert-deftest jaunder-idempotency-key-is-fresh-and-nonempty ()
  ;; AC-C1/C6: each call yields a non-empty token, and two calls differ.
  (let ((k1 (jaunder--idempotency-key))
        (k2 (jaunder--idempotency-key)))
    (should (stringp k1))
    (should (> (length k1) 0))
    (should-not (equal k1 k2))))

(ert-deftest jaunder-create-retry-sends-key-and-retries-5xx ()
  ;; AC-C2/C3/C4: a 5xx retries with the SAME key, then succeeds.
  (let ((calls 0)
        (keys nil))
    (cl-letf (((symbol-function 'sleep-for) (lambda (&rest _) nil))
              ((symbol-function 'jaunder--http-request)
               (lambda (_method _url _body _ctype extra-headers)
                 (setq calls (1+ calls))
                 (push (cdr (assoc "Idempotency-Key" extra-headers)) keys)
                 (if (= calls 1)
                     '(:status 503 :body "")
                   '(:status 201 :body "ok")))))
             (let ((resp (jaunder--create-with-retry "http://x/posts" "<xml/>")))
               (should (= (plist-get resp :status) 201))
               (should (= calls 2))
               (should (equal (nth 0 keys) (nth 1 keys)))
               (should (> (length (nth 0 keys)) 0))))))

(ert-deftest jaunder-create-retry-does-not-retry-4xx ()
  ;; AC-C3: a 4xx returns immediately, no retry.
  (let ((calls 0))
    (cl-letf (((symbol-function 'sleep-for) (lambda (&rest _) nil))
              ((symbol-function 'jaunder--http-request)
               (lambda (&rest _)
                 (setq calls (1+ calls))
                 '(:status 400 :body ""))))
             (let ((resp (jaunder--create-with-retry "http://x/posts" "<xml/>")))
               (should (= (plist-get resp :status) 400))
               (should (= calls 1))))))

(ert-deftest jaunder-create-retry-exhausts-on-transport-error ()
  ;; AC-C5: after 3 transport failures the publish errors. The stub signals a
  ;; `plz-error' subtype because that is what `jaunder--http-request' re-signals
  ;; when a transport failure carries no response.
  (let ((calls 0))
    (cl-letf (((symbol-function 'sleep-for) (lambda (&rest _) nil))
              ((symbol-function 'jaunder--http-request)
               (lambda (&rest _)
                 (setq calls (1+ calls))
                 (signal 'plz-curl-error (list "Curl error" (make-plz-error))))))
             (should-error (jaunder--create-with-retry "http://x/posts" "<xml/>"))
             (should (= calls 3)))))

(ert-deftest jaunder-create-retry-does-not-retry-a-config-error ()
  ;; #945: a non-transport error (e.g. no auth-source entry) cannot succeed on
  ;; retry, so it surfaces on the first attempt with no backoff.
  (let ((calls 0)
        (sleeps 0))
    (cl-letf (((symbol-function 'sleep-for) (lambda (&rest _) (setq sleeps (1+ sleeps))))
              ((symbol-function 'jaunder--http-request)
               (lambda (&rest _)
                 (setq calls (1+ calls))
                 (error "jaunder: no auth-source entry for a@b"))))
             (should-error (jaunder--create-with-retry "http://x/posts" "<xml/>"))
             (should (= calls 1))
             (should (= sleeps 0)))))
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
(ert-deftest jaunder-git-media-helpers-handle-worktree-and-untracked-results ()
  "Git helper results drive the warning layer without exposing process details."
  (let (arguments)
    (cl-letf (((symbol-function 'executable-find) (lambda (_) "/git"))
              ((symbol-function 'call-process)
               (lambda (&rest args)
                 (push args arguments)
                 (if (equal (nth 4 args) "rev-parse") 0 1)))
              ((symbol-function 'buffer-string) (lambda () "/repo\n")))
             (should (equal (jaunder--git-toplevel "/repo/subdir") "/repo"))
             (should-not (jaunder--git-tracked-p "/repo" "/repo/missing.png"))
             (should (= (length arguments) 2)))))


(ert-deftest jaunder-publish-commands-require-visiting-file ()
  "Interactive publish must not silently manufacture request context."
  (with-temp-buffer
    (should-error (jaunder-publish) :type 'error)
    (should-error (jaunder--rename-to-slug "post") :type 'error)))

(ert-deftest jaunder-new-post-prompts-for-a-blog-when-directory-is-unmapped ()
  "New-post selects an explicit configured blog rather than using an unrelated cwd."
  (let* ((root (make-temp-file "jaunder-new-post-" t))
         (other (make-temp-file "jaunder-other-" t))
         (jaunder-blogs (list (cons (file-name-as-directory root)
                                    '(:base-url "https://blog" :username "alice"))))
         (default-directory other)
         selected)
    (unwind-protect
        (cl-letf (((symbol-function 'completing-read)
                   (lambda (&rest _) (setq selected (file-name-as-directory root))))
                  ((symbol-function 'format-time-string) (lambda (&rest _) "20260829T000000")))
                 (jaunder-new-post)
                 (should (equal selected (file-name-as-directory root)))
                 (should (equal (buffer-file-name)
                                (expand-file-name "draft-20260829T000000.org" root))))
      (when (buffer-file-name) (kill-buffer (current-buffer)))
      (delete-directory root t)
      (delete-directory other t))))

(ert-deftest jaunder-new-post-uses-default-directory-without-configured-blogs ()
  "Without configured blogs, a draft is deliberately created in the current directory."
  (let* ((root (file-name-as-directory (make-temp-file "jaunder-new-post-" t)))
         (default-directory root)
         (jaunder-blogs nil)
         created)
    (unwind-protect
        (cl-letf (((symbol-function 'format-time-string)
                   (lambda (&rest _) "20260829T000001")))
                 (jaunder-new-post)
                 (setq created (current-buffer))
                 (should (equal (buffer-file-name)
                                (expand-file-name "draft-20260829T000001.org" root))))
      (when (buffer-live-p created) (kill-buffer created))
      (delete-directory root t))))


;;; jaunder-test.el ends here
