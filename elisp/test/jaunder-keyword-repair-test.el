;;; jaunder-keyword-repair-test.el --- Repair preview proofs -*- lexical-binding: t; -*-

(require 'ert)
(require 'cl-lib)
(require 'jaunder-entry)

(ert-deftest jaunder-normalize-keyword-matches-rust-door ()
  (dolist (pair '(("100 bullets" . "100-bullets")
                  ("James S. A. Corey" . "james-s-a-corey")
                  ("old man's war" . "old-man-s-war")
                  (" Rust-- " . "Rust--")
                  ("\u000bRust\u000b" . "Rust")
                  ("\u2003Rust\u2003" . "Rust")
                  ("\u00a0Rust---Lang-\u00a0" . "Rust---Lang-")
                  (" --A / B!! " . "a-b")
                  ("a\t\nB" . "a-b")))
    (should (equal (jaunder-normalize-keyword (car pair)) (cdr pair)))))

(ert-deftest jaunder-normalize-keyword-rejects-unusable-and-non-ascii ()
  (dolist (term (list "" " \t" "!!!" "---" "café" "K" "\u00a0K\u00a0" "\u2003two words\u2003"
                      "\u00a0100 bullets" "100 bullets\u00a0"
                      (concat "a" (string 0) "b") (concat "a" (string 11) "b") 42))
    (should-error (jaunder-normalize-keyword term))))

(defun jaunder-keyword-repair-test--member (&optional categories)
  "Return one synthetic complete current Member."
  (concat "<entry xmlns=\"http://www.w3.org/2005/Atom\""
          " xmlns:app=\"http://www.w3.org/2007/app\""
          " xmlns:j=\"https://jaunder.org/ns/atompub\">"
          "<title>Newer server title</title><summary>Server summary</summary>"
          "<link rel=\"edit\" href=\"https://h/atompub/alice/posts/42\"/>"
          "<j:slug>post-2</j:slug><j:audience>public</j:audience>"
          "<published>2026-01-01T00:00:00Z</published>"
          "<app:control><app:draft>no</app:draft></app:control>"
          "<content type=\"text/org\">Newer body</content>"
          (or categories "<category term=\"RUST\"/>") "</entry>"))

(defmacro jaunder-keyword-repair-test--with-preview (source xml &rest body)
  "Evaluate BODY with SOURCE and XML behind the real preview's HTTP boundary."
  (declare (indent 2))
  `(let* ((root (make-temp-file "jaunder-repair-test" t))
          (path (expand-file-name "post.org" root))
          (jaunder-blogs (list (cons root '(:base-url "https://h" :username "alice"))))
          (member-xml ,xml)
          (collection-xml nil)
          (member-status 200)
          (request-error nil)
          (member-headers '(("etag" . "\"current\"")))
          (requests nil))
     (unwind-protect
         (progn
           (with-temp-file path (insert ,source))
           (cl-letf (((symbol-function 'jaunder--http-request)
                      (lambda (method url &rest _)
                        (push (list method url) requests)
                        (unless (equal method "GET") (ert-fail "Preview attempted a write"))
                        (when (and request-error
                                   (or (eq request-error 'collection)
                                       (not (equal url "https://h/atompub/alice/posts"))))
                          (signal 'file-error '("sensitive transport cause")))
                        (list :status (if (equal url "https://h/atompub/alice/posts") 200 member-status)
                              :headers member-headers
                              :body (if (equal url "https://h/atompub/alice/posts")
                                        (or collection-xml
                                            (concat "<feed xmlns=\"http://www.w3.org/2005/Atom\">"
                                                    member-xml "</feed>"))
                                      member-xml)))))
             ,@body))
       (dolist (buffer (buffer-list))
         (when (equal (buffer-file-name buffer) path)
           (with-current-buffer buffer (set-buffer-modified-p nil))
           (kill-buffer buffer)))
       (delete-directory root t))))

(ert-deftest jaunder-keyword-repair-preview-binds-read-only-additions ()
  (require 'jaunder-keyword-repair)
  (jaunder-keyword-repair-test--with-preview
   "#+PROPERTY: JAUNDER_ID 42\n#+KEYWORDS: Rust, 100 bullets\n#+TAGS: James S. A. Corey\n#+KEYWORDS: 100-bullets\n\nBody\n#+KEYWORDS: ignored words\n"
   (jaunder-keyword-repair-test--member)
   (let* ((before (with-temp-buffer (insert-file-contents-literally path) (buffer-string)))
          (report (jaunder-keyword-repair-preview root))
          (data (jaunder-keyword-repair-report-data report))
          (row (car (plist-get data :rows))))
     (should (equal (plist-get data :root) (file-name-as-directory root)))
     (should (equal (plist-get data :base-url) "https://h"))
     (should (equal (plist-get data :username) "alice"))
     (should (eq (plist-get row :status) 'eligible))
     (should (equal (plist-get row :id) "42"))
     (should (equal (plist-get row :path) path))
     (should (equal (plist-get row :etag) "\"current\""))
     (should (equal (plist-get row :member-xml) member-xml))
     (should (equal (plist-get row :existing-labels) '("RUST")))
     (should (equal (plist-get row :additions) '("100-bullets" "james-s-a-corey")))
     (should (equal (plist-get row :union) '("RUST" "100-bullets" "james-s-a-corey")))
     (should (equal (plist-get row :sha256) (secure-hash 'sha256 before)))
     (should (equal before (with-temp-buffer (insert-file-contents-literally path) (buffer-string))))
     (should (= (length requests) 2))
     ;; Inspection must not mutate the reviewed snapshot, even through strings.
     (aset (plist-get row :id) 0 ?9)
     (should (equal (plist-get (car (plist-get (jaunder-keyword-repair-report-data report) :rows)) :id) "42"))
     (let ((captured member-xml))
       (aset member-xml 0 ?X)
       (should (equal (plist-get (car (plist-get (jaunder-keyword-repair-report-data report) :rows)) :member-xml)
                      (concat "<" (substring captured 1))))))))

(ert-deftest jaunder-keyword-repair-preview-blocks-preservation-defects ()
  (require 'jaunder-keyword-repair)
  (dolist (change '(("<title>Newer server title</title>" . "<title type=\"html\">Newer server title</title>")
                    ("<summary>Server summary</summary>" . "<summary>One</summary><summary>Two</summary>")
                    ("<j:audience>public</j:audience>" . "<j:audience>foreign</j:audience>")
                    ("<j:audience>public</j:audience>" . "")
                    ("<j:slug>post-2</j:slug>" . "<j:slug>../bad</j:slug>")
                    ("<published>2026-01-01T00:00:00Z</published>" . "<published>2026-02-30T00:00:00Z</published>")
                    ("<content type=\"text/org\">Newer body</content>" . "<content type=\"text/org\" src=\"https://h/media/1\"/>")
                    ("<app:control><app:draft>no</app:draft></app:control>" . "<app:control><app:draft xmlns:app=\"urn:foreign\">no</app:draft></app:control>")
                    ("<app:control><app:draft>no</app:draft></app:control>" . "<app:control><app:draft>no</app:draft></app:control><app:control/>")
                    ("<category term=\"RUST\"/>" . "<category term=\"rust words\"/>")
                    ("<category term=\"RUST\"/>" . "<category term=\"Rust\" label=\"Other\"/>")
                    ("<category term=\"RUST\"/>" . "<category xmlns=\"urn:foreign\" term=\"Rust\"/>")
                    ("<entry " . "<feed ")))
    (jaunder-keyword-repair-test--with-preview
     "#+PROPERTY: JAUNDER_ID 42\n#+KEYWORDS: two words\n\nBody\n"
     (jaunder-keyword-repair-test--member)
     (setq collection-xml (concat "<feed>" member-xml "</feed>"))
     (setq member-xml (replace-regexp-in-string (regexp-quote (car change)) (cdr change) member-xml t t))
     (when (equal (car change) "<entry ")
       (setq member-xml (replace-regexp-in-string "</entry>" "</feed>" member-xml t t)))
     (should (eq (plist-get (car (plist-get (jaunder-keyword-repair-report-data
                                             (jaunder-keyword-repair-preview root)) :rows)) :status) 'blocked)))))

(ert-deftest jaunder-keyword-repair-preview-blocks-local-remote-and-cap ()
  (require 'jaunder-keyword-repair)
  (dolist (case '(invalid-term missing-id duplicate-id duplicate-local duplicate-remote missing-remote
                               foreign modified draft over-cap weak-etag duplicate-etag deleted))
    (jaunder-keyword-repair-test--with-preview
     "#+PROPERTY: JAUNDER_ID 42\n#+KEYWORDS: 100 bullets\n\nBody\n"
     (jaunder-keyword-repair-test--member)
     (setq collection-xml (concat "<feed>" member-xml "</feed>"))
     (pcase case
       ('invalid-term (with-temp-file path (insert "#+PROPERTY: JAUNDER_ID 42\n#+KEYWORDS: !!!\n\nBody\n")))
       ('missing-id (with-temp-file path (insert "#+KEYWORDS: two words\n\nBody\n")))
       ('duplicate-id (with-temp-file path (insert "#+PROPERTY: JAUNDER_ID 42\n#+PROPERTY: JAUNDER_ID 42\n#+KEYWORDS: two words\n\nBody\n")))
       ('duplicate-local (copy-file path (expand-file-name "other.org" root)))
       ('duplicate-remote (setq collection-xml (concat "<feed>" member-xml member-xml "</feed>")))
       ('missing-remote (setq collection-xml "<feed/>"))
       ('foreign (setq member-xml (replace-regexp-in-string "https://h/atompub/alice" "https://foreign/atompub/bob" member-xml)))
       ('modified (with-current-buffer (find-file-noselect path) (goto-char (point-max)) (insert "Unsaved")))
       ('draft (setq member-xml (replace-regexp-in-string "<app:draft>no" "<app:draft>yes" member-xml)))
       ('over-cap (setq member-xml (jaunder-keyword-repair-test--member
                                    (mapconcat (lambda (n) (format "<category term=\"tag-%d\"/>" n))
                                               (number-sequence 1 256) ""))))
       ('weak-etag (setq member-headers '(("etag" . "W/\"weak\""))))
       ('duplicate-etag (setq member-headers '(("etag" . "\"first\"") ("etag" . "\"second\""))))
       ('deleted (setq member-status 404)))
     (let ((rows (plist-get (jaunder-keyword-repair-report-data (jaunder-keyword-repair-preview root)) :rows)))
       (should rows)
       (dolist (row rows) (should (eq (plist-get row :status) 'blocked)))))))

(ert-deftest jaunder-keyword-repair-preview-blocks-unicode-folding-label ()
  (require 'jaunder-keyword-repair)
  (jaunder-keyword-repair-test--with-preview
   "#+PROPERTY: JAUNDER_ID 42\n#+KEYWORDS: K\n\nBody\n"
   (jaunder-keyword-repair-test--member)
   (let ((row (car (plist-get (jaunder-keyword-repair-report-data
                               (jaunder-keyword-repair-preview root)) :rows))))
     (should (eq (plist-get row :status) 'blocked))
     (should (eq (plist-get row :reason) 'invalid-local-keyword))
     (should (plist-get row :sha256))
     (should (= (length requests) 1)))))

(ert-deftest jaunder-keyword-repair-preview-org-boundaries-and-noop ()
  (require 'jaunder-keyword-repair)
  (dolist (body '("* Headline\n#+KEYWORDS: ignored words\n"
                  "Paragraph\n#+KEYWORDS: ignored words\n"
                  "#+begin_src org\n#+KEYWORDS: ignored words\n#+end_src\n"
                  "# Comment\n#+KEYWORDS: ignored words\n"))
    (jaunder-keyword-repair-test--with-preview
     (concat "#+PROPERTY: JAUNDER_ID 42\n#+CATEGORY: Rust\n#+AUTHOR: User\n#+TAGS: RUST\n\n" body)
     (jaunder-keyword-repair-test--member)
     (let ((row (car (plist-get (jaunder-keyword-repair-report-data (jaunder-keyword-repair-preview root)) :rows))))
       (should (eq (plist-get row :status) 'unchanged))
       (should-not (plist-get row :additions))))))

(ert-deftest jaunder-keyword-repair-preview-scheduled-clean-buffer-and-root-scope ()
  (require 'jaunder-keyword-repair)
  (jaunder-keyword-repair-test--with-preview
   "#+PROPERTY: JAUNDER_ID 42\n#+PROPERTY: JAUNDER_SYNCED stale\n#+PROPERTY: JAUNDER_SYNCED_AT nonsense\n#+KEYWORDS: one / two, ONE-TWO\n\nBody\n"
   (replace-regexp-in-string "2026-01-01" "2099-01-01" (jaunder-keyword-repair-test--member))
   (let ((buffer (find-file-noselect path)))
     (make-directory (expand-file-name "nested" root))
     (copy-file path (expand-file-name "nested/duplicate.org" root))
     (let* ((before (with-current-buffer buffer (buffer-string)))
            (rows (plist-get (jaunder-keyword-repair-report-data
                              (jaunder-keyword-repair-preview (expand-file-name "nested" root))) :rows)))
       (should (= (length rows) 1))
       (should (eq (plist-get (car rows) :status) 'eligible))
       (should (equal (plist-get (car rows) :additions) '("one-two")))
       (should (equal before (with-current-buffer buffer (buffer-string))))
       (should-not (buffer-modified-p buffer))))))

(ert-deftest jaunder-keyword-repair-preview-rejects-unconfigured-root-before-http ()
  (require 'jaunder-keyword-repair)
  (let ((jaunder-blogs nil))
    (cl-letf (((symbol-function 'jaunder--http-request)
               (lambda (&rest _) (ert-fail "Unconfigured preview attempted HTTP"))))
      (should-error (jaunder-keyword-repair-preview default-directory) :type 'user-error))))

(ert-deftest jaunder-keyword-repair-preview-preserves-infrastructure-source-safely ()
  (require 'jaunder-keyword-repair)
  (dolist (stage '(collection collection-decode member-read local-read member-decode member-http normalization member-validate audience-validation))
    (jaunder-keyword-repair-test--with-preview
     "#+PROPERTY: JAUNDER_ID 42\n#+KEYWORDS: two words\n\nBody\n"
     (jaunder-keyword-repair-test--member)
     (setq collection-xml (concat "<feed>" member-xml "</feed>"))
     (pcase stage
       ('collection (setq request-error 'collection))
       ('collection-decode (setq collection-xml "<feed>malformed sensitive body"))
       ('member-read (setq request-error 'member))
       ('member-decode (setq member-xml "<entry>malformed sensitive body"))
       ('member-http (setq member-status 503)))
     (let ((condition
            (if (memq stage '(local-read normalization member-validate audience-validation))
                (cl-letf (((symbol-function (pcase stage
                                              ('local-read 'insert-file-contents-literally)
                                              ('normalization 'jaunder-normalize-keyword)
                                              ('audience-validation 'jaunder--synchronized-response-audiences)
                                              (_ 'jaunder--parse-pulled-member)))
                           (lambda (&rest _) (signal 'file-error '("sensitive injected cause")))))
                  (should-error (jaunder-keyword-repair-preview root)
                                :type 'jaunder-keyword-repair-preview-error))
              (should-error (jaunder-keyword-repair-preview root)
                            :type 'jaunder-keyword-repair-preview-error))))
       (should (eq (plist-get (cdr condition) :stage)
                   (pcase stage ('member-http 'member-read) ('normalization 'row-validate)
                          ('audience-validation 'member-validate) ('collection-decode 'collection) (_ stage))))
       (should (equal (plist-get (cdr condition) :id) (unless (memq stage '(collection collection-decode)) "42")))
       (should (consp (jaunder-keyword-repair-error-source condition)))
       (unless (memq stage '(member-decode collection-decode member-http))
         (should (eq (car (jaunder-keyword-repair-error-source condition)) 'file-error)))
       (when (eq stage 'member-http)
         (should (equal (jaunder-keyword-repair-error-source condition)
                        '(jaunder-keyword-repair-http-error 503))))
       (should-not (string-match-p "sensitive" (error-message-string condition)))))))

(provide 'jaunder-keyword-repair-test)
;;; jaunder-keyword-repair-test.el ends here
