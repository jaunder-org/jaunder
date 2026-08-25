;;; jaunder-pull-test.el --- Deterministic Member pull tests -*- lexical-binding: t; -*-

;; Copyright (C) 2026 Jaunder contributors

;;; Commentary:
;; Pure contracts for Member Entry validation and exact Org synthesis.

;;; Code:

(require 'ert)
(require 'jaunder)

(defconst jaunder-pull-test--captured-at
  (date-to-time "2026-08-25T12:00:00Z")
  "Fixed pull wall clock used by exact-byte tests.")

(defun jaunder-pull-test--entry (&rest parts)
  "Wrap PARTS in a namespaced Atom Member Entry."
  (concat "<entry xmlns=\"http://www.w3.org/2005/Atom\""
          " xmlns:app=\"http://www.w3.org/2007/app\""
          " xmlns:j=\"https://jaunder.org/ns/atompub\">"
          (apply #'concat parts)
          "</entry>"))

(defun jaunder-pull-test--org (xml &optional etag zone)
  "Map XML with fixed pull inputs to exact Org bytes."
  (jaunder--atom->org xml (or etag "\"sha256-test\"")
                      jaunder-pull-test--captured-at (or zone "UTC")))

(ert-deftest jaunder-atom->org-draft-untitled-exact-bytes ()
  ;; Empty Atom title means no local title, while draft native source and sync
  ;; metadata keep their fixed order without date fields.
  (let ((xml (jaunder-pull-test--entry
              "<title></title>"
              "<link rel=\"edit\" href=\"https://h/atompub/alice/posts/42\"/>"
              "<j:slug>untitled-note</j:slug>"
              "<content type=\"text/org\">Body\nhttps://h/media/x.png</content>"
              "<app:control><app:draft>yes</app:draft></app:control>")))
    (should
     (equal
      (jaunder-pull-test--org xml)
      (concat "#+PROPERTY: JAUNDER_STATUS draft\n"
              "#+PROPERTY: JAUNDER_FORMAT org\n"
              "#+PROPERTY: JAUNDER_SLUG untitled-note\n"
              "#+PROPERTY: JAUNDER_ID 42\n"
              "#+PROPERTY: JAUNDER_SYNCED \"sha256-test\"\n"
              "#+PROPERTY: JAUNDER_SYNCED_AT 2026-08-25T12:00:00Z\n"
              "\nBody\nhttps://h/media/x.png")))))

(ert-deftest jaunder-atom->org-scheduled-multiline-metadata-is-reversible ()
  ;; Multiline wire metadata becomes repeated headers in deterministic order and
  ;; the title publishes back as the same LF-delimited value.
  (let* ((xml (jaunder-pull-test--entry
               "<title>Line one\nLine two</title>"
               "<category term=\"alpha\"/><category term=\"beta\"/>"
               "<summary>First\nSecond</summary>"
               "<published>2026-08-26T13:00:00+02:00</published>"
               "<link rel=\"edit\" href=\"https://h/atompub/alice/posts/7\"/>"
               "<j:slug>scheduled-post</j:slug>"
               "<content type=\"text/markdown\"># Body\nhttps://h/media/y.png</content>"))
         (org (jaunder-pull-test--org xml)))
    (should
     (equal org
            (concat "#+TITLE: Line one\n#+TITLE: Line two\n"
                    "#+DATE: [2026-08-26 Wed 11:00]\n"
                    "#+KEYWORDS: alpha, beta\n"
                    "#+DESCRIPTION: First\n#+DESCRIPTION: Second\n"
                    "#+PROPERTY: JAUNDER_STATUS scheduled\n"
                    "#+PROPERTY: JAUNDER_DATE_TZ UTC\n"
                    "#+PROPERTY: JAUNDER_DATE_UTC 2026-08-26T13:00:00+02:00\n"
                    "#+PROPERTY: JAUNDER_FORMAT markdown\n"
                    "#+PROPERTY: JAUNDER_SLUG scheduled-post\n"
                    "#+PROPERTY: JAUNDER_ID 7\n"
                    "#+PROPERTY: JAUNDER_SYNCED \"sha256-test\"\n"
                    "#+PROPERTY: JAUNDER_SYNCED_AT 2026-08-25T12:00:00Z\n"
                    "\n# Body\nhttps://h/media/y.png")))
    (with-temp-buffer
      (insert org)
      (org-mode)
      (should (equal (jaunder-entry-title (jaunder--org->atom))
                     "Line one\nLine two")))))

(ert-deftest jaunder-atom->org-published-html-and-xhtml-bodies ()
  ;; Escaped HTML remains source text; XHTML drops only its required wrapper and
  ;; canonically serializes the ordered child nodes.
  (let ((prefix (jaunder-pull-test--entry
                 "<title>HTML</title>"
                 "<published>2026-08-24T10:00:00Z</published>"
                 "<link rel=\"edit\" href=\"https://h/atompub/alice/posts/9\"/>"
                 "<j:slug>html-post</j:slug>")))
    (should (string-suffix-p "\n<p>A &amp; B</p>"
                             (jaunder-pull-test--org
                              (replace-regexp-in-string
                               "</entry>" "<content type=\"html\">&lt;p&gt;A &amp;amp; B&lt;/p&gt;</content></entry>"
                               prefix t t))))
    (should (string-suffix-p "\n<p>A &amp; B</p>tail"
                             (jaunder-pull-test--org
                              (replace-regexp-in-string
                               "</entry>"
                               (concat "<content type=\"xhtml\"><div xmlns=\"http://www.w3.org/1999/xhtml\">"
                                       "<p>A &amp; B</p>tail</div></content></entry>")
                               prefix t t))))))

(ert-deftest jaunder-atom->org-rejects-malformed-member-or-etag ()
  ;; Required Member cardinality and strong sync identity fail before any caller
  ;; can obtain bytes to install.
  (let ((base '("<title>T</title>"
                "<link rel=\"edit\" href=\"https://h/atompub/alice/posts/1\"/>"
                "<j:slug>safe-slug</j:slug>"
                "<content type=\"text/org\">Body</content>"
                "<app:control><app:draft>yes</app:draft></app:control>")))
    (dolist (xml
             (list (apply #'jaunder-pull-test--entry (cdr base))
                   (apply #'jaunder-pull-test--entry
                          (append (list "<title>A</title><title>B</title>") (cdr base)))
                   (apply #'jaunder-pull-test--entry
                          (append (butlast base 2) (last base 1)))
                   (apply #'jaunder-pull-test--entry
                          (append (butlast base 3)
                                  (list "<link rel=\"edit\" href=\"https://h/posts/1\"/>"
                                        "<link rel=\"edit\" href=\"https://h/posts/2\"/>")
                                  (last base 2)))
                   (apply #'jaunder-pull-test--entry
                          (append (butlast base 2)
                                  (list "<j:slug>../escape</j:slug>")
                                  (last base 1)))))
      (should-error (jaunder-pull-test--org xml)))
    (should-error (jaunder-pull-test--org (apply #'jaunder-pull-test--entry base)
                                          "W/\"weak\""))
    (should-error (jaunder-pull-test--org
                   (jaunder-pull-test--entry
                    "<title>T</title>"
                    "<link rel=\"edit\" href=\"https://h/posts/1\"/>"
                    "<j:slug>safe</j:slug>"
                    "<content type=\"text/org\">Body</content>")))))

(provide 'jaunder-pull-test)
;;; jaunder-pull-test.el ends here
