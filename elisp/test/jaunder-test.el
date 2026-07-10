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

(ert-deftest jaunder-media-content-type-maps-extensions ()
  (should (equal (jaunder--media-content-type "a.png") "image/png"))
  (should (equal (jaunder--media-content-type "a.jpg") "image/jpeg"))
  (should (equal (jaunder--media-content-type "a.jpeg") "image/jpeg"))
  (should (equal (jaunder--media-content-type "a.gif") "image/gif"))
  (should (equal (jaunder--media-content-type "a.webp") "image/webp"))
  (should (equal (jaunder--media-content-type "a.svg") "image/svg+xml")))

(ert-deftest jaunder-media-content-type-is-case-insensitive ()
  (should (equal (jaunder--media-content-type "IMG.PNG") "image/png"))
  (should (equal (jaunder--media-content-type "p.JPEG") "image/jpeg")))

(ert-deftest jaunder-media-content-type-non-image-is-nil ()
  (should (null (jaunder--media-content-type "notes.txt")))
  (should (null (jaunder--media-content-type "noext"))))

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

(ert-deftest jaunder-media-collect-excludes-nonqualifying ()
  ;; header-region link (in the stripped #+DESCRIPTION block), absolute http,
  ;; bare fuzzy link, non-image file link, and links inside src/example blocks.
  (let ((rs (jaunder-test--collect
             (concat "#+DESCRIPTION: [[file:cover.png]]\n"
                     "\n"
                     "abs [[https://x/y.png]] "
                     "fuzzy [[a.png]] "
                     "doc [[file:notes.txt]]\n"
                     "#+begin_src org\n[[file:code.png]]\n#+end_src\n"
                     "#+begin_example\n[[file:ex.png]]\n#+end_example\n")
             "/d/")))
    (should (null rs))))

(ert-deftest jaunder-media-preflight-errors-on-missing-listing-all ()
  (let* ((d (make-temp-file "jt-preflight-" t))
         (present (expand-file-name "a.png" d)))
    (unwind-protect
        (progn
          (with-temp-file present (insert "x"))
          (should-not (jaunder--media-preflight (list (list :path present))))
          (let ((err (should-error
                      (jaunder--media-preflight
                       (list (list :path (expand-file-name "m1.png" d))
                             (list :path present)
                             (list :path (expand-file-name "m2.png" d))))
                      :type 'error)))
            (should (string-match-p "m1.png" (error-message-string err)))
            (should (string-match-p "m2.png" (error-message-string err)))
            (should-not (string-match-p "a.png" (error-message-string err)))))
      (delete-directory d t))))

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
                  "plain [[https://x/y.png]] and [[file:notes.txt]] only" nil)
                 "plain [[https://x/y.png]] and [[file:notes.txt]] only")))

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

(ert-deftest jaunder-media-link-p-qualifies-file-and-attachment ()
  ;; file:/attachment: with an image extension qualify; http, a non-image file:,
  ;; and a bare fuzzy link do not.  Operates on neutral link records — no org.
  (should (equal
           (mapcar (lambda (r) (and (jaunder--media-link-p r) t))
                   '((:type "file" :path "a.png")
                     (:type "attachment" :path "b.gif")
                     (:type "https" :path "//x/c.png")
                     (:type "file" :path "d.txt")
                     (:type "fuzzy" :path "e.png")))
           '(t t nil nil nil))))

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

(ert-deftest jaunder-localize-media-uploads-each-file-once ()
  ;; Two links to the same file upload once (dedup cache); both rewrite to the
  ;; harvested URL; the authoring buffer is never modified.
  (let* ((d (make-temp-file "jt-localize-" t))
         (img (expand-file-name "x.png" d))
         (calls nil))
    (unwind-protect
        (progn
          (with-temp-file img (insert "x"))
          (cl-letf (((symbol-function 'jaunder--upload-media)
                     (lambda (path _ct) (push path calls) "https://h/m/x.png")))
                   (with-temp-buffer
                     (insert (format "#+TITLE: T\n\n[[file:%s]] and [[file:%s]]\n" img img))
                     (org-mode)
                     (let* ((body (jaunder-entry-body (jaunder--org->atom)))
                            (before (buffer-string))
                            (out (jaunder--localize-media body)))
                       (should (equal calls (list img)))
                       (should (equal out
                                      "[[https://h/m/x.png]] and [[https://h/m/x.png]]"))
                       (should (equal (buffer-string) before))))))
      (delete-directory d t))))

(ert-deftest jaunder-localize-media-no-images-is-noop ()
  ;; A body with no qualifying local images returns unchanged, uploading nothing.
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

;;; jaunder-test.el ends here
