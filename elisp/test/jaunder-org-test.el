;;; jaunder-org-test.el --- ERT suite for jaunder-org -*- lexical-binding: t; -*-

;;; Commentary:
;; Unit tests for the corresponding jaunder package module.

;;; Code:

(require 'ert)
(require 'jaunder)

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

;;; org->atom — publish time / timezone

(ert-deftest jaunder-org->atom-publication-time-projections ()
  (let ((cases
         `((published-iana-dst-summer
            ,(concat "#+DATE: [2026-07-01 Wed 09:00]\n"
                     "#+PROPERTY: JAUNDER_STATUS published\n"
                     "#+PROPERTY: JAUNDER_DATE_TZ America/New_York\n\nB\n")
            "2026-07-01T13:00:00Z")
           (published-iana-dst-winter
            ,(concat "#+DATE: [2026-01-01 Thu 09:00]\n"
                     "#+PROPERTY: JAUNDER_STATUS published\n"
                     "#+PROPERTY: JAUNDER_DATE_TZ America/New_York\n\nB\n")
            "2026-01-01T14:00:00Z")
           ;; G1 regression: a raw offset string is silently misread by
           ;; `encode-time' as UTC; the mapping must parse it to integer seconds.
           (published-numeric-offset-string
            ,(concat "#+DATE: [2026-07-01 Wed 09:00]\n"
                     "#+PROPERTY: JAUNDER_STATUS published\n"
                     "#+PROPERTY: JAUNDER_DATE_TZ -0500\n\nB\n")
            "2026-07-01T14:00:00Z")
           (published-numeric-offset-colon
            ,(concat "#+DATE: [2026-07-01 Wed 09:00]\n"
                     "#+PROPERTY: JAUNDER_STATUS published\n"
                     "#+PROPERTY: JAUNDER_DATE_TZ -05:00\n\nB\n")
            "2026-07-01T14:00:00Z")
           (published-scheduled
            ,(concat "#+DATE: [2026-07-01 Wed 09:00]\n"
                     "#+PROPERTY: JAUNDER_STATUS scheduled\n"
                     "#+PROPERTY: JAUNDER_DATE_TZ America/New_York\n\nB\n")
            "2026-07-01T13:00:00Z")
           ;; status=published with no #+DATE -> omit (server stamps).
           (published-publish-now-is-nil
            "#+PROPERTY: JAUNDER_STATUS published\n\nB\n"
            nil)
           ;; Drafts carry no publish time even with a #+DATE.
           (published-draft-is-nil
            ,(concat "#+DATE: [2026-07-01 Wed 09:00]\n"
                     "#+PROPERTY: JAUNDER_STATUS draft\n"
                     "#+PROPERTY: JAUNDER_DATE_TZ America/New_York\n\nB\n")
            nil)
           ;; Scheduled entries without a #+DATE omit the publish time.
           (published-missing-date-is-nil
            ,(concat "#+PROPERTY: JAUNDER_STATUS scheduled\n"
                     "#+PROPERTY: JAUNDER_DATE_TZ America/New_York\n\nB\n")
            nil))))
    (dolist (case cases)
      (pcase-let ((`(,label ,source ,expected) case))
        (ert-info ((format "publication-time projection case: %s" label))
                  (should
                   (equal
                    (jaunder-entry-published
                     (jaunder-test--entry source))
                    expected)))))))

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

;;; jaunder-org-test.el ends here
