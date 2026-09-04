;;; jaunder-atom-test.el --- ERT suite for jaunder-atom -*- lexical-binding: t; -*-

;;; Commentary:
;; Unit tests for the corresponding jaunder package module.

;;; Code:

(require 'ert)
(require 'jaunder)

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

;;; jaunder-atom-test.el ends here
