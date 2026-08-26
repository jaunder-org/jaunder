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
    ;; The complete pull header is consumable by publishing without teaching this
    ;; test the server's Org grammar: assert only the client's outgoing entry.
    (with-temp-buffer
      (insert org)
      (org-mode)
      (let ((entry (jaunder--org->atom)))
        (should (equal (jaunder-entry-title entry) "Line one\nLine two"))
        (should (equal (jaunder-entry-categories entry) '("alpha" "beta")))
        (should (equal (jaunder-entry-summary entry) "First\nSecond"))
        (should-not (jaunder-entry-draft entry))
        (should (equal (jaunder-entry-content-type entry) "text/org"))
        (should (equal (jaunder-entry-published entry) "2026-08-26T11:00:00Z"))
        ;; Republish sends only native content; the locally generated DATE,
        ;; status, and bookkeeping block stays structured client-side.
        (should (equal (jaunder-entry-body entry) "# Body\nhttps://h/media/y.png"))))))

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

(ert-deftest jaunder-atom->org-xhtml-requires-xhtml-wrapper-and-sole-content ()
  ;; Atom XHTML has one XHTML-namespace div wrapper; surrounding whitespace is
  ;; harmless, but a different namespace, text, or element is not native body.
  (let ((prefix (jaunder-pull-test--entry
                 "<title>XHTML</title>"
                 "<published>2026-08-24T10:00:00Z</published>"
                 "<link rel=\"edit\" href=\"https://h/atompub/alice/posts/9\"/>"
                 "<j:slug>xhtml-post</j:slug>")))
    (dolist (content
             '("<content type=\"xhtml\"><div>Body</div></content>"
               "<content type=\"xhtml\">stray<div xmlns=\"http://www.w3.org/1999/xhtml\">Body</div></content>"
               "<content type=\"xhtml\"><div xmlns=\"http://www.w3.org/1999/xhtml\">Body</div><span/></content>"))
      (should-error
       (jaunder-pull-test--org
        (replace-regexp-in-string "</entry>" (concat content "</entry>") prefix t t))))))

(ert-deftest jaunder-atom->org-rejects-unqualified-published-and-unsafe-slugs ()
  ;; Pull accepts offset-qualified RFC-3339 before parsing, and rejects slugs
  ;; that could inject a property header or escape the configured root.
  (let ((parts '("<title>T</title>"
                 "<link rel=\"edit\" href=\"https://h/atompub/alice/posts/1\"/>"
                 "<j:slug>safe-slug</j:slug>"
                 "<content type=\"text/org\">Body</content>")))
    (dolist (published '("2026-08-24T10:00:00"
                         "2026-08-24"
                         "2026-08-24T10:00:00+0000"))
      (should-error
       (jaunder-pull-test--org
        (apply #'jaunder-pull-test--entry
               (append (list "<title>T</title>"
                             (format "<published>%s</published>" published))
                       (cdr parts))))))
    (should (string-match-p
             "#\\+PROPERTY: JAUNDER_DATE_UTC 2026-08-24T10:00:00.123Z"
             (jaunder-pull-test--org
              (apply #'jaunder-pull-test--entry
                     (append (list "<title>T</title>"
                                   "<published>2026-08-24T10:00:00.123Z</published>")
                             (cdr parts))))))
    (dolist (slug '("unsafe\n#+PROPERTY: JAUNDER_STATUS published"
                    "unsafe\rheader"
                    "unsafe/path"
                    "unsafe\\path"
                    "\x1funsafe"))
      (should-not (jaunder--safe-pull-slug-p slug)))
    (should (jaunder--safe-pull-slug-p "日本語-記事"))))

(ert-deftest jaunder-atom->org-rejects-semantic-rfc-3339-invalidity ()
  ;; `date-to-time' normalizes several malformed calendar values, so the pull
  ;; boundary validates each component before parsing while retaining valid wire
  ;; text with leap days, fractions, and numeric offsets verbatim.
  (let ((parts '("<title>T</title>"
                 "<link rel=\"edit\" href=\"https://h/atompub/alice/posts/1\"/>"
                 "<j:slug>safe-slug</j:slug>"
                 "<content type=\"text/org\">Body</content>")))
    (dolist (published '("2026-13-01T00:00:00Z"
                         "2025-02-29T00:00:00Z"
                         "2024-02-30T00:00:00Z"
                         "2026-01-01T24:00:00Z"
                         "2026-01-01T00:60:00Z"
                         "2026-01-01T00:00:60Z"
                         "2026-01-01T00:00:00+24:00"
                         "2026-01-01T00:00:00+00:60"))
      (should-error
       (jaunder-pull-test--org
        (apply #'jaunder-pull-test--entry
               (cons (format "<published>%s</published>" published) parts)))))
    (dolist (published '("2024-02-29T23:59:59.123+14:30"
                         "2024-02-29T00:00:00Z"))
      (should
       (string-match-p
        (regexp-quote (concat "#+PROPERTY: JAUNDER_DATE_UTC " published))
        (jaunder-pull-test--org
         (apply #'jaunder-pull-test--entry
                (cons (format "<published>%s</published>" published) parts))))))))

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


(defun jaunder-pull-test--member (&optional id slug)
  "Return a D1 Member fixture with optional ID and SLUG."
  (jaunder--make-inventory-member
   :id (or id "42")
   :slug (or slug "untitled-note")
   :edit-uri (format "https://h/atompub/alice/posts/%s" (or id "42"))))

(defun jaunder-pull-test--response-entry (&optional id slug)
  "Return a valid draft response Entry for optional ID and SLUG."
  (jaunder-pull-test--entry
   "<title></title>"
   (format "<link rel=\"edit\" href=\"https://h/atompub/alice/posts/%s\"/>"
           (or id "42"))
   (format "<j:slug>%s</j:slug>" (or slug "untitled-note"))
   "<content type=\"text/org\">Body</content>"
   "<app:control><app:draft>yes</app:draft></app:control>"))

(defun jaunder-pull-test--temp-artifacts (root)
  "Return pull temporary artifacts directly under ROOT."
  (directory-files root t "\\.jaunder-pull-" t))

(ert-deftest jaunder-pull-member-preflight-blocks-before-network ()
  ;; A previewed destination that already exists is reported with its exact path;
  ;; no GET runs and its bytes remain untouched.
  (let* ((root (make-temp-file "jaunder-pull-" t))
         (path (expand-file-name "untitled-note.org" root))
         (calls 0))
    (unwind-protect
        (progn
          (write-region "winner" nil path nil 'silent)
          (cl-letf (((symbol-function 'jaunder--http-request)
                     (lambda (&rest _) (setq calls (1+ calls)))))
                   (let ((result (jaunder--pull-member root
                                                       (jaunder-pull-test--member))))
                     (should (eq (jaunder-pull-result-status result) 'blocked))
                     (should (equal (jaunder-pull-result-path result) path))
                     (should (= calls 0))
                     (should (equal (with-temp-buffer
                                      (insert-file-contents path)
                                      (buffer-string))
                                    "winner")))))
      (delete-directory root t))))

(ert-deftest jaunder-pull-member-gets-d1-uri-and-installs-exact-file ()
  ;; The D3-facing seam resolves the configured blog, GETs the D1 edit URI, and
  ;; returns one exact pulled path without leaking its same-directory temp file.
  (let* ((root (make-temp-file "jaunder-pull-" t))
         (path (expand-file-name "untitled-note.org" root))
         (jaunder-blogs
          (list (cons (file-name-as-directory root)
                      '(:base-url "https://h" :username "alice"))))
         requested)
    (unwind-protect
        (cl-letf (((symbol-function 'jaunder--http-request)
                   (lambda (method url &rest _)
                     (setq requested
                           (list method url (jaunder--active-base-url)
                                 (jaunder--active-username)))
                     (list :status 200
                           :headers '(("etag" . "\"sha256-test\""))
                           :body (jaunder-pull-test--response-entry))))
                  ((symbol-function 'current-time)
                   (lambda () jaunder-pull-test--captured-at))
                  ((symbol-function 'jaunder--current-zone-name)
                   (lambda () "UTC")))
                 (let ((result (jaunder--pull-member root
                                                     (jaunder-pull-test--member))))
                   (should (eq (jaunder-pull-result-status result) 'pulled))
                   (should (equal (jaunder-pull-result-path result) path))
                   (should (equal requested
                                  '("GET" "https://h/atompub/alice/posts/42"
                                    "https://h" "alice")))
                   (should (file-exists-p path))
                   (should (string-suffix-p "\n\nBody"
                                            (with-temp-buffer
                                              (insert-file-contents path)
                                              (buffer-string))))
                   (should-not (jaunder-pull-test--temp-artifacts root))))
      (delete-directory root t))))

(ert-deftest jaunder-pull-install-writes-utf-8-unix-despite-ambient-coding ()
  ;; Pull artifacts are portable deterministic bytes, not a product of a user's
  ;; coding or line-ending defaults; the temporary install remains no-replace.
  (let* ((root (make-temp-file "jaunder-pull-" t))
         (path (expand-file-name "日本語.org" root))
         (bytes "café\n日本語\n"))
    (unwind-protect
        (let ((coding-system-for-write 'utf-16le-dos)
              (default-buffer-file-coding-system 'utf-16le-dos))
          (let ((result (jaunder--install-pulled-bytes path bytes)))
            (should (eq (jaunder-pull-result-status result) 'pulled))
            (should
             (equal
              (with-temp-buffer
                (set-buffer-multibyte nil)
                (insert-file-contents-literally path)
                (buffer-string))
              (encode-coding-string bytes 'utf-8-unix)))))
      (delete-directory root t))))

(ert-deftest jaunder-pull-member-rejects-stale-response-identity ()
  ;; Collection preview identity is stable for one apply: a changed response ID
  ;; or slug aborts without claiming either old or new destination.
  (dolist (response (list (jaunder-pull-test--response-entry "99" nil)
                          (jaunder-pull-test--response-entry nil "new-slug")))
    (let* ((root (make-temp-file "jaunder-pull-" t))
           (jaunder-blogs
            (list (cons (file-name-as-directory root)
                        '(:base-url "https://h" :username "alice")))))
      (unwind-protect
          (cl-letf (((symbol-function 'jaunder--http-request)
                     (lambda (&rest _)
                       (list :status 200
                             :headers '(("etag" . "\"sha256-test\""))
                             :body response))))
                   (should-error (jaunder--pull-member root
                                                       (jaunder-pull-test--member)))
                   (should (null (directory-files root nil "\\.org\\'" t)))
                   (should-not (jaunder-pull-test--temp-artifacts root)))
        (delete-directory root t)))))

(ert-deftest jaunder-pull-member-failures-leave-root-unchanged ()
  ;; HTTP, transport, mapping, temp-write, and install failures never expose a
  ;; destination or retain a temporary artifact.
  (dolist (failure '(http transport mapping write install))
    (let* ((root (make-temp-file "jaunder-pull-" t))
           (jaunder-blogs
            (list (cons (file-name-as-directory root)
                        '(:base-url "https://h" :username "alice"))))
           (real-write (symbol-function 'write-region))
           (real-link (symbol-function 'add-name-to-file)))
      (unwind-protect
          (cl-letf (((symbol-function 'jaunder--http-request)
                     (lambda (&rest _)
                       (pcase failure
                         ('http '(:status 503 :headers nil :body "no"))
                         ('transport (error "transport"))
                         ('mapping
                          (list :status 200 :headers '(("etag" . "W/\"weak\""))
                                :body (jaunder-pull-test--response-entry)))
                         (_
                          (list :status 200
                                :headers '(("etag" . "\"sha256-test\""))
                                :body (jaunder-pull-test--response-entry))))))
                    ((symbol-function 'write-region)
                     (lambda (&rest args)
                       (if (eq failure 'write)
                           (error "write")
                         (apply real-write args))))
                    ((symbol-function 'add-name-to-file)
                     (lambda (&rest args)
                       (if (eq failure 'install)
                           (error "install")
                         (apply real-link args)))))
                   (should-error (jaunder--pull-member root
                                                       (jaunder-pull-test--member)))
                   (should (null (directory-files root nil "\\.org\\'" t)))
                   (should-not (jaunder-pull-test--temp-artifacts root)))
        (delete-directory root t)))))

(ert-deftest jaunder-pull-member-race-preserves-winner-and-blocks ()
  ;; Atomic no-replace installation reports a racing winner as blocked and never
  ;; replaces the winner's bytes.
  (let* ((root (make-temp-file "jaunder-pull-" t))
         (path (expand-file-name "untitled-note.org" root))
         (jaunder-blogs
          (list (cons (file-name-as-directory root)
                      '(:base-url "https://h" :username "alice"))))
         (real-write (symbol-function 'write-region)))
    (unwind-protect
        (cl-letf (((symbol-function 'jaunder--http-request)
                   (lambda (&rest _)
                     (list :status 200
                           :headers '(("etag" . "\"sha256-test\""))
                           :body (jaunder-pull-test--response-entry))))
                  ((symbol-function 'add-name-to-file)
                   (lambda (_temp destination &optional _ok)
                     (funcall real-write "winner" nil destination nil 'silent)
                     (signal 'file-already-exists (list destination)))))
                 (let ((result (jaunder--pull-member root
                                                     (jaunder-pull-test--member))))
                   (should (eq (jaunder-pull-result-status result) 'blocked))
                   (should (equal (jaunder-pull-result-path result) path))
                   (should (equal (with-temp-buffer
                                    (insert-file-contents path)
                                    (buffer-string))
                                  "winner"))
                   (should-not (jaunder-pull-test--temp-artifacts root))))
      (delete-directory root t))))
(provide 'jaunder-pull-test)
;;; jaunder-pull-test.el ends here
