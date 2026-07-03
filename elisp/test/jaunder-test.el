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

;;; org->atom — field mapping (C2 / issue #160)

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
  ;; JAUNDER_FORMAT header (which would only lie about org body — issue #160).
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

;;; offset parsing / zone resolution (C2 / issue #160)

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

;;; org->atom — publish time / timezone (C2 / issue #160)

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

;;; atom-entry -> xml serializer (C2 / issue #160)

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

;;; media upload (unit C, issue #161)

(ert-deftest jaunder-atom-entry-fields-harvests-content-src-and-type ()
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
    (should (equal (cdr (assq 'content-src (jaunder--atom-entry-fields xml)))
                   "https://h/media/upload/ab/cd/abcd/p.png"))
    (should (equal (cdr (assq 'content-type (jaunder--atom-entry-fields xml)))
                   "image/png"))))

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
             (let ((jaunder-username "alice"))
               (jaunder--http-request "POST" "http://x/media" (list 'file "/tmp/a.png")
                                      "image/png" (list (cons "Slug" "a.png"))))
             (should (equal (cdr (assoc "Slug" captured)) "a.png"))
             (should (equal (cdr (assoc "Content-Type" captured)) "image/png"))
             (should (assoc "Authorization" captured)))))

(ert-deftest jaunder-upload-media-errors-on-non-2xx ()
  (cl-letf (((symbol-function 'jaunder--http-request)
             (lambda (&rest _) '(:status 500 :body "boom"))))
           (let ((jaunder-base-url "http://x") (jaunder-username "alice"))
             (should-error (jaunder--upload-media "/tmp/x.png" "image/png") :type 'error))))

;;; jaunder-test.el ends here
