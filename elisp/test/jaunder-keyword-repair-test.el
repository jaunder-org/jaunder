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

(ert-deftest jaunder-keyword-repair-preview-rejects-nested-scalar-markup ()
  "Metadata and native text cannot be flattened during category replacement."
  (require 'jaunder-keyword-repair)
  (dolist (case '(("<title>Newer server title</title>" "<title><b>Nested</b></title>" member-scalar-markup)
                  ("<content type=\"text/org\">Newer body</content>" "<content type=\"text/org\"><b>Nested</b></content>" member-text-markup)
                  ("<app:control><app:draft>no</app:draft></app:control>" "<app:control><app:draft>no</app:draft><other/></app:control>" invalid-member-draft)))
    (jaunder-keyword-repair-test--with-preview
     "#+PROPERTY: JAUNDER_ID 42\n#+KEYWORDS: two words\n\nBody\n"
     (jaunder-keyword-repair-test--member)
     (setq collection-xml (concat "<feed>" member-xml "</feed>"))
     (setq member-xml (replace-regexp-in-string (regexp-quote (car case)) (cadr case) member-xml t t))
     (let ((row (car (plist-get (jaunder-keyword-repair-report-data
                                 (jaunder-keyword-repair-preview root)) :rows))))
       (should (eq (plist-get row :status) 'blocked))
       (should (eq (plist-get row :reason) (nth 2 case)))))))

(ert-deftest jaunder-keyword-repair-preview-interactive-preserves-control-whitespace ()
  "The interactive entry retains valid whitespace around lifecycle metadata."
  (require 'jaunder-keyword-repair)
  (jaunder-keyword-repair-test--with-preview
   "#+PROPERTY: JAUNDER_ID 42\n#+KEYWORDS: two words\n\nBody\n"
   (replace-regexp-in-string
    "<app:control><app:draft>no</app:draft></app:control>"
    "<app:control>\n  <app:draft>no</app:draft>\n</app:control>"
    (jaunder-keyword-repair-test--member) t t)
   (let* ((default-directory (file-name-as-directory root))
          (report (call-interactively #'jaunder-keyword-repair-preview))
          (row (car (plist-get (jaunder-keyword-repair-report-data report) :rows))))
     (should (eq (plist-get row :status) 'eligible))
     (should (equal (plist-get row :additions) '("two-words")))
     (should (equal (plist-get row :member-xml) member-xml))
     (should (= (length requests) 2)))))

(ert-deftest jaunder-keyword-repair-report-data-detaches-long-and-dotted-lists ()
  "Inspection copies retain shape without stack growth along label lists."
  (require 'jaunder-keyword-repair)
  (let* ((labels (mapcar (lambda (n) (format "tag-%d" n)) (number-sequence 1 1024)))
         (pair (cons (copy-sequence "name") (copy-sequence "value")))
         (data (list :rows (list (list :existing-labels labels :detail pair))))
         (report (jaunder--make-keyword-repair-report :data data))
         (copy (jaunder-keyword-repair-report-data report))
         (row (car (plist-get copy :rows)))
         (copied-labels (plist-get row :existing-labels))
         (copied-pair (plist-get row :detail)))
    (should (equal copy data))
    (should-not (eq copied-labels labels))
    (dolist (index '(0 255 1023))
      (should-not (eq (nth index copied-labels) (nth index labels)))
      (aset (nth index copied-labels) 0 ?X)
      (should (= (aref (nth index labels) 0) ?t)))
    (should-not (eq copied-pair pair))
    (aset (car copied-pair) 0 ?X)
    (aset (cdr copied-pair) 0 ?X)
    (should (equal pair '("name" . "value")))
    (setcar copied-labels "replacement")
    (should (equal (car labels) "tag-1"))))

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

(ert-deftest jaunder-keyword-repair-apply-confirmed-conditional-and-declined ()
  (require 'jaunder-keyword-repair)
  (jaunder-keyword-repair-test--with-preview
   "#+PROPERTY: JAUNDER_ID 42\n#+KEYWORDS: Rust, 100 bullets\n\nOld body\n"
   (jaunder-keyword-repair-test--member)
   (let ((report (jaunder-keyword-repair-preview root))
         (read-request (symbol-function 'jaunder--http-request))
         (puts nil)
         (confirm nil))
     (cl-letf (((symbol-function 'yes-or-no-p) (lambda (&rest _) confirm))
               ((symbol-function 'jaunder--http-request)
                (lambda (method url &rest args)
                  (if (equal method "GET") (apply read-request method url args)
                    (push args puts)
                    (setq member-xml (car args))
                    (list :status 200)))))
       (should (plist-get (jaunder-keyword-repair-apply report '(0)) :cancelled))
       (should-not puts)
       (setq confirm t)
       (let ((result (jaunder-keyword-repair-apply report '(0))))
         (should (eq (plist-get (car (plist-get result :results)) :status) 'repaired)))
       (should (= (length puts) 1))
       (should (equal (nth 1 (car puts)) "application/atom+xml"))
       (should (equal (nth 2 (car puts)) '(("If-Match" . "\"current\""))))
       (should (string-match-p "Newer server title" member-xml))
       (should (string-match-p "Newer body" member-xml))
       (should (string-match-p "term=\"RUST\"" member-xml))
       (should (string-match-p "term=\"100-bullets\"" member-xml))
       (should (eq (plist-get (car (plist-get (jaunder-keyword-repair-report-data
                                               (jaunder-keyword-repair-preview root)) :rows)) :status)
                   'unchanged))))))

(ert-deftest jaunder-keyword-repair-apply-blocks-stale-authority ()
  (require 'jaunder-keyword-repair)
  (dolist (case '(etag weak-etag duplicate-etag bytes modified missing-file symlink
                       duplicate-local duplicate-remote duplicate-slug missing-remote foreign
                       config-base config-user config-root draft cap invalid-member))
    (jaunder-keyword-repair-test--with-preview
     "#+PROPERTY: JAUNDER_ID 42\n#+KEYWORDS: 100 bullets\n\nBody\n"
     (jaunder-keyword-repair-test--member)
     (let ((report (jaunder-keyword-repair-preview root)))
       (pcase case
         ('etag (setq member-headers '(("etag" . "\"newer\""))))
         ('weak-etag (setq member-headers '(("etag" . "W/\"current\""))))
         ('duplicate-etag (setq member-headers '(("etag" . "\"current\"") ("etag" . "\"current\""))))
         ('bytes (with-temp-file path (insert "#+PROPERTY: JAUNDER_ID 42\n#+KEYWORDS: changed words\n")))
         ('modified (with-current-buffer (find-file-noselect path) (insert "Unsaved")))
         ('missing-file (delete-file path))
         ('symlink (rename-file path (expand-file-name "source" root))
                   (make-symbolic-link (expand-file-name "source" root) path))
         ('duplicate-local (copy-file path (expand-file-name "other.org" root)))
         ('duplicate-remote (setq collection-xml (concat "<feed>" member-xml member-xml "</feed>")))
         ('duplicate-slug (setq collection-xml
                                (concat "<feed>" member-xml
                                        (replace-regexp-in-string "/posts/42" "/posts/43" member-xml) "</feed>")))
         ('missing-remote (setq collection-xml "<feed/>"))
         ('foreign (setq member-xml (replace-regexp-in-string "https://h/atompub/alice" "https://foreign/atompub/bob" member-xml)))
         ('config-base (setcdr (car jaunder-blogs) '(:base-url "https://elsewhere" :username "alice")))
         ('config-user (setcdr (car jaunder-blogs) '(:base-url "https://h" :username "bob")))
         ('config-root (setq jaunder-blogs nil))
         ('draft (setq member-xml (replace-regexp-in-string "<app:draft>no" "<app:draft>yes" member-xml)))
         ('cap (setq member-xml (jaunder-keyword-repair-test--member
                                 (mapconcat (lambda (n) (format "<category term=\"tag-%d\"/>" n))
                                            (number-sequence 1 256) ""))))
         ('invalid-member (setq member-xml (replace-regexp-in-string "<j:audience>public" "<j:audience>invalid" member-xml))))
       (cl-letf (((symbol-function 'yes-or-no-p) (lambda (&rest _) t)))
         (let ((result (car (plist-get (jaunder-keyword-repair-apply report '(0)) :results))))
           (should (eq (plist-get result :status) 'blocked))))))))

(ert-deftest jaunder-keyword-repair-apply-blocks-path-disappearing-after-local-check ()
  "A checked file is not authority for the subsequent fresh local inventory."
  (require 'jaunder-keyword-repair)
  (jaunder-keyword-repair-test--with-preview
   "#+PROPERTY: JAUNDER_ID 42\n#+KEYWORDS: two words\n\nBody\n"
   (jaunder-keyword-repair-test--member)
   (let ((report (jaunder-keyword-repair-preview root))
         (check-local (symbol-function 'jaunder--keyword-repair-check-local)))
     (cl-letf (((symbol-function 'yes-or-no-p) (lambda (&rest _) t))
               ((symbol-function 'jaunder--keyword-repair-check-local)
                (lambda (row)
                  (prog1 (funcall check-local row)
                    (rename-file path (expand-file-name "moved.txt" root))))))
       (let ((result (car (plist-get (jaunder-keyword-repair-apply report '(0)) :results))))
         (should (eq (plist-get result :status) 'blocked))
         (should (eq (plist-get result :reason) 'local-path-changed))
         (should (= (length requests) 3)))))))

(ert-deftest jaunder-keyword-repair-apply-preflight-source-is-inspectable ()
  (require 'jaunder-keyword-repair)
  (jaunder-keyword-repair-test--with-preview
   "#+PROPERTY: JAUNDER_ID 42\n#+KEYWORDS: two words\n\nBody\n"
   (jaunder-keyword-repair-test--member)
   (let ((report (jaunder-keyword-repair-preview root)))
     (setq request-error 'collection)
     (cl-letf (((symbol-function 'yes-or-no-p) (lambda (&rest _) t)))
       (let ((result (car (plist-get (jaunder-keyword-repair-apply report '(0)) :results))))
         (should (eq (plist-get result :status) 'blocked))
         (should (equal (jaunder-keyword-repair-error-source (cons 'error result))
                        '(file-error "sensitive transport cause")))
         (should-not (string-match-p "sensitive" (format "%S" result))))))))

(ert-deftest jaunder-keyword-repair-apply-terminal-http-and-response-loss ()
  (require 'jaunder-keyword-repair)
  (dolist (case '((204 . repaired) (201 . repaired) (412 . blocked)
                  (400 . failed) (500 . failed) (loss . unknown-remote-outcome)
                  (nil . unknown-remote-outcome) (quit . unknown-remote-outcome)))
    (jaunder-keyword-repair-test--with-preview
     "#+PROPERTY: JAUNDER_ID 42\n#+KEYWORDS: two words\n\nBody\n"
     (jaunder-keyword-repair-test--member)
     (let ((report (jaunder-keyword-repair-preview root))
           (read-request (symbol-function 'jaunder--http-request))
           (puts 0))
       (cl-letf (((symbol-function 'yes-or-no-p) (lambda (&rest _) t))
                 ((symbol-function 'jaunder--http-request)
                  (lambda (method url &rest args)
                    (if (equal method "GET") (apply read-request method url args)
                      (cl-incf puts)
                      (pcase (car case)
                        ('loss (signal 'file-error '("private response loss")))
                        ('quit (signal 'quit nil))
                        (_ (list :status (car case))))))))
         (let* ((application (jaunder-keyword-repair-apply report '(0)))
                (result (car (plist-get application :results))))
           (should (eq (plist-get result :status) (cdr case)))
           (should (= puts 1))
           (should (equal application jaunder-keyword-repair-last-application))
           (when (eq (car case) 'quit) (should (plist-get application :cancelled)))
           (when (eq (car case) 'loss)
             (should (equal (jaunder-keyword-repair-error-source (cons 'error result))
                            '(file-error "private response loss")))
             (should-not (string-match-p "private" (format "%S" result))))))))))

(ert-deftest jaunder-keyword-repair-apply-preserves-qualified-remote-dom ()
  (require 'jaunder-keyword-repair)
  (jaunder-keyword-repair-test--with-preview
   "#+PROPERTY: JAUNDER_ID 42\n#+PROPERTY: JAUNDER_SYNCED old\n#+KEYWORDS: two words\n\nOld body\n"
   (concat "<a:entry xmlns:a=\"http://www.w3.org/2005/Atom\" xmlns=\"urn:other\""
           " xmlns:app=\"http://www.w3.org/2007/app\" xmlns:j=\"https://jaunder.org/ns/atompub\">"
           "<a:title>New &amp; newer</a:title><a:summary>Summary &lt;source&gt;</a:summary>"
           "<a:link rel=\"edit\" href=\"https://h/atompub/alice/posts/42\"/>"
           "<a:link rel=\"alternate\" href=\"https://h/alice/2099/01/01/post-2\"/>"
           "<j:slug>post-2</j:slug><j:audience>private</j:audience>"
           "<a:published>2099-01-01T00:00:00Z</a:published>"
           "<app:control><app:draft>no</app:draft></app:control>"
           "<a:content type=\"xhtml\"><div xmlns=\"http://www.w3.org/1999/xhtml\">"
           "<p>New <strong>body</strong> &amp; text</p></div></a:content>"
           "<a:category term=\"RUST\"/><a:category term=\"rust\"/>"
           "<other xmlns=\"urn:extension\" data=\"kept\">opaque</other></a:entry>")
   (setq collection-xml (concat "<feed>" (jaunder-keyword-repair-test--member) "</feed>"))
   (let ((report (jaunder-keyword-repair-preview root))
         (original member-xml)
         (before (with-temp-buffer (insert-file-contents-literally path) (buffer-string)))
         (read-request (symbol-function 'jaunder--http-request))
         (payload nil))
     (cl-letf (((symbol-function 'yes-or-no-p) (lambda (&rest _) t))
               ((symbol-function 'jaunder--http-request)
                (lambda (method url &rest args)
                  (if (equal method "GET") (apply read-request method url args)
                    (setq payload (car args)) (list :status 200)))))
       (should (eq (plist-get (car (plist-get (jaunder-keyword-repair-apply report '(0)) :results))
                              :status) 'repaired)))
     (should payload)
     ;; Compare all non-category DOM, not just the authored-field projection.
     (let ((parse (lambda (xml)
                    (with-temp-buffer
                      (insert xml)
                      (let* ((dom (car (xml-parse-region (point-min) (point-max))))
                             (context (jaunder--atom-namespace-context dom nil))
                             (categories (jaunder--atom-direct-elements-in-namespace
                                          dom 'category jaunder--atom-ns context)))
                        (setcdr (cdr dom) (cl-remove-if (lambda (node) (memq node categories))
                                                        (dom-children dom)))
                        dom)))))
       (should (equal (funcall parse original) (funcall parse payload))))
     (should (equal before (with-temp-buffer (insert-file-contents-literally path) (buffer-string))))
     (should (equal (cdr (assq 'categories (jaunder--harvest-response-fields payload)))
                    '("RUST" "two-words"))))))

(ert-deftest jaunder-keyword-repair-apply-ordered-batch-and-cancellation ()
  (require 'jaunder-keyword-repair)
  (dolist (mode '(complete between during decline-quit))
    (jaunder-keyword-repair-test--with-preview
     "#+PROPERTY: JAUNDER_ID 42\n#+KEYWORDS: two words\n\nBody\n"
     (jaunder-keyword-repair-test--member)
     (let ((second-xml (replace-regexp-in-string
                        "post-2" "second" (replace-regexp-in-string "/posts/42" "/posts/43" member-xml)))
           (puts nil)
           (reads 0)
           (read-request (symbol-function 'jaunder--http-request)))
       (with-temp-file (expand-file-name "second.org" root)
         (insert "#+PROPERTY: JAUNDER_ID 43\n#+KEYWORDS: second words\n\nBody\n"))
       (setq collection-xml (concat "<feed>" member-xml second-xml "</feed>"))
       (cl-letf (((symbol-function 'jaunder--http-request)
                  (lambda (method url &rest args)
                    (cond
                     ((equal method "GET")
                      (cl-incf reads)
                      (when (and (eq mode 'between) puts) (signal 'quit nil))
                      (if (equal url "https://h/atompub/alice/posts/43")
                          (list :status 200 :headers member-headers :body second-xml)
                        (apply read-request method url args)))
                     (t (push url puts)
                        (when (eq mode 'during) (signal 'quit nil))
                        (list :status (if (equal url "https://h/atompub/alice/posts/42") 500 200)))))))
         (let ((report (jaunder-keyword-repair-preview root)))
           (cl-letf (((symbol-function 'yes-or-no-p)
                      (lambda (&rest _) (when (eq mode 'decline-quit) (signal 'quit nil)) t)))
             (let* ((application (jaunder-keyword-repair-apply report '(1 0)))
                    (results (plist-get application :results)))
               (pcase mode
                 ('complete
                  (should (equal (mapcar (lambda (result) (plist-get result :id)) results) '("43" "42")))
                  (should (equal (mapcar (lambda (result) (plist-get result :status)) results) '(repaired failed)))
                  (should (= (length puts) 2)))
                 ('between
                  (should (equal (mapcar (lambda (result) (plist-get result :id)) results) '("43" "42")))
                  (should (eq (plist-get (car results) :status) 'repaired))
                  (should (eq (plist-get (cadr results) :status) 'blocked))
                  (should (= (length puts) 1)))
                 ('during
                  (should (= (length results) 1))
                  (should (eq (plist-get (car results) :status) 'unknown-remote-outcome))
                  (should (= (length puts) 1)))
                 ('decline-quit (should-not results) (should-not puts)))
               (should (eq (not (not (plist-get application :cancelled))) (not (eq mode 'complete))))))))))))

(ert-deftest jaunder-keyword-repair-apply-requires-explicit-selection-and-noop ()
  (require 'jaunder-keyword-repair)
  (jaunder-keyword-repair-test--with-preview
   "#+PROPERTY: JAUNDER_ID 42\n#+KEYWORDS: rust\n\nBody\n"
   (jaunder-keyword-repair-test--member)
   (let ((report (jaunder-keyword-repair-preview root)))
     (cl-letf (((symbol-function 'yes-or-no-p)
                (lambda (&rest _) (ert-fail "Invalid report reached confirmation"))))
       (should-error (jaunder-keyword-repair-apply nil '(0)) :type 'user-error))
     (dolist (selection '(nil (0 0) (-1) (1) ("42")))
       (should-error (jaunder-keyword-repair-apply report selection) :type 'user-error))
     (cl-letf (((symbol-function 'yes-or-no-p) (lambda (&rest _) t)))
       (let ((result (car (plist-get (jaunder-keyword-repair-apply report '(0)) :results))))
         (should (eq (plist-get result :status) 'unchanged)))))))

(ert-deftest jaunder-keyword-repair-apply-rechecks-after-remote-read ()
  (require 'jaunder-keyword-repair)
  (dolist (change '(bytes duplicate-local config))
    (jaunder-keyword-repair-test--with-preview
     "#+PROPERTY: JAUNDER_ID 42\n#+KEYWORDS: two words\n\nBody\n"
     (jaunder-keyword-repair-test--member)
     (let ((report (jaunder-keyword-repair-preview root))
           (read-request (symbol-function 'jaunder--http-request)))
       (cl-letf (((symbol-function 'yes-or-no-p) (lambda (&rest _) t))
                 ((symbol-function 'jaunder--http-request)
                  (lambda (method url &rest args)
                    (should (equal method "GET"))
                    (let ((response (apply read-request method url args)))
                      (when (equal url "https://h/atompub/alice/posts/42")
                        (pcase change
                          ('bytes (with-temp-file path (insert "Changed while reading")))
                          ('duplicate-local (copy-file path (expand-file-name "new.org" root)))
                          ('config (setcdr (car jaunder-blogs) '(:base-url "https://elsewhere" :username "alice")))))
                      response))))
         (should (eq (plist-get (car (plist-get (jaunder-keyword-repair-apply report '(0)) :results))
                                :status) 'blocked)))))))

(ert-deftest jaunder-keyword-repair-apply-known-predispatch-failure ()
  (require 'jaunder-keyword-repair)
  (jaunder-keyword-repair-test--with-preview
   "#+PROPERTY: JAUNDER_ID 42\n#+KEYWORDS: two words\n\nBody\n"
   (jaunder-keyword-repair-test--member)
   (let ((report (jaunder-keyword-repair-preview root)))
     (cl-letf (((symbol-function 'yes-or-no-p) (lambda (&rest _) t))
               ((symbol-function 'dom-print)
                (lambda (&rest _) (signal 'file-error '("private encoding failure")))))
       (let ((result (car (plist-get (jaunder-keyword-repair-apply report '(0)) :results))))
         (should (eq (plist-get result :status) 'failed))
         (should (eq (plist-get result :stage) 'payload))
         (should (equal (jaunder-keyword-repair-error-source (cons 'error result))
                        '(file-error "private encoding failure"))))))))

(ert-deftest jaunder-keyword-repair-apply-checkpoints-before-pending-quit ()
  (require 'jaunder-keyword-repair)
  (dolist (stop-after '(1 2 3))
    (jaunder-keyword-repair-test--with-preview
     "#+PROPERTY: JAUNDER_ID 42\n#+KEYWORDS: two words\n\nBody\n"
     (jaunder-keyword-repair-test--member)
     (let ((xmls (list (cons "42" member-xml)))
           (puts nil)
           (completed 0)
           (read-request (symbol-function 'jaunder--http-request))
           (apply-row (symbol-function 'jaunder--keyword-repair-apply-row)))
       (dolist (id '("43" "44" "45"))
         (with-temp-file (expand-file-name (concat "z" id ".org") root)
           (insert (concat "#+PROPERTY: JAUNDER_ID " id "\n#+KEYWORDS: two words\n\nBody\n")))
         (setq xmls (append xmls
                            (list (cons id (replace-regexp-in-string
                                            "post-2" (concat "post-" id)
                                            (replace-regexp-in-string "/posts/42" (concat "/posts/" id) member-xml)))))))
       (setq collection-xml (concat "<feed>" (mapconcat #'cdr xmls "") "</feed>"))
       (cl-letf (((symbol-function 'jaunder--http-request)
                  (lambda (method url &rest args)
                    ;; Checkpoint protection must not suppress transport quits.
                    (should-not inhibit-quit)
                    (if (equal method "GET")
                        (if (equal url "https://h/atompub/alice/posts")
                            (apply read-request method url args)
                          (list :status 200 :headers member-headers
                                :body (cdr (assoc (car (last (split-string url "/"))) xmls))))
                      (push url puts)
                      (pcase (length puts)
                        (1 (list :status 200))
                        (2 (list :status 500))
                        (3 (signal 'file-error '("private response loss")))
                        (_ (ert-fail "PUT after pending cancellation")))))))
         (let ((report (jaunder-keyword-repair-preview root)))
           (cl-letf (((symbol-function 'yes-or-no-p) (lambda (&rest _) t))
                     ((symbol-function 'jaunder--keyword-repair-apply-row)
                      (lambda (data row)
                        (prog1 (funcall apply-row data row)
                          ;; Inject precisely after a terminal row returns,
                          ;; before the executor appends/checkpoints its result.
                          (cl-incf completed)
                          (when (= completed stop-after) (setq quit-flag t))))))
             (unwind-protect
                 (let* ((application (jaunder-keyword-repair-apply report '(0 1 2 3)))
                        (results (plist-get application :results)))
                   (should (plist-get application :cancelled))
                   (should (equal (mapcar (lambda (result) (plist-get result :id)) results)
                                  (cl-subseq '("42" "43" "44") 0 stop-after)))
                   (should (equal (mapcar (lambda (result) (plist-get result :status)) results)
                                  (cl-subseq '(repaired failed unknown-remote-outcome) 0 stop-after)))
                   (should (= (length puts) stop-after))
                   (should (equal application jaunder-keyword-repair-last-application))
                   (should-not quit-flag))
               (setq quit-flag nil)))))))))

(ert-deftest jaunder-keyword-repair-apply-revalidates-reviewed-unchanged ()
  (require 'jaunder-keyword-repair)
  (dolist (change '(bytes modified etag removed-member removed-tag
                          missing-path duplicate-local duplicate-remote weak-etag config fresh))
    (jaunder-keyword-repair-test--with-preview
     "#+PROPERTY: JAUNDER_ID 42\n#+KEYWORDS: rust\n\nBody\n"
     (jaunder-keyword-repair-test--member)
     (let ((report (jaunder-keyword-repair-preview root)))
       (should (eq (plist-get (car (plist-get (jaunder-keyword-repair-report-data report) :rows))
                              :status) 'unchanged))
       (pcase change
         ('bytes (with-temp-file path (insert "#+PROPERTY: JAUNDER_ID 42\n#+KEYWORDS: rust, new words\n\nBody\n")))
         ('modified (with-current-buffer (find-file-noselect path) (insert "Unsaved")))
         ('etag (setq member-headers '(("etag" . "\"newer\""))))
         ('removed-member (setq member-status 404))
         ;; Even with an unchanged ETag, freshly missing tags must never turn
         ;; the reviewed no-op into unreviewed repair authority.
         ('removed-tag (setq member-xml (jaunder-keyword-repair-test--member "")))
         ('missing-path (delete-file path))
         ('duplicate-local (copy-file path (expand-file-name "other.org" root)))
         ('duplicate-remote (setq collection-xml (concat "<feed>" member-xml member-xml "</feed>")))
         ('weak-etag (setq member-headers '(("etag" . "W/\"current\""))))
         ('config (setcdr (car jaunder-blogs) '(:base-url "https://elsewhere" :username "alice"))))
       (cl-letf (((symbol-function 'yes-or-no-p) (lambda (&rest _) t)))
         (let ((result (car (plist-get (jaunder-keyword-repair-apply report '(0)) :results))))
           (should (eq (plist-get result :status) (if (eq change 'fresh) 'unchanged 'blocked)))
           ;; The fixture rejects every non-GET request.  Fresh no-op must
           ;; nevertheless fetch both the current Collection and Member.
           (when (eq change 'fresh) (should (= (length requests) 4)))))))))

(ert-deftest jaunder-keyword-repair-preview-admits-server-published-control-omission ()
  "Published and scheduled server output omits control, not lifecycle evidence."
  (dolist (published '("2024-01-01T00:00:00Z" "2099-01-01T00:00:00Z" "" "invalid"))
    (jaunder-keyword-repair-test--with-preview
     "#+PROPERTY: JAUNDER_ID 42\n#+KEYWORDS: 100 bullets\n\nBody\n"
     (replace-regexp-in-string
      "<published>2026-01-01T00:00:00Z</published>"
      (if (equal published "") "" (concat "<published>" published "</published>"))
      (replace-regexp-in-string "<app:control><app:draft>no</app:draft></app:control>" ""
                                (jaunder-keyword-repair-test--member)))
     (let* ((report (jaunder-keyword-repair-preview root))
            (row (car (plist-get (jaunder-keyword-repair-report-data report) :rows))))
       (should (eq (plist-get row :status)
                   (if (member published '("" "invalid")) 'blocked 'eligible)))
       (unless (member published '("" "invalid"))
         (should-not (string-match-p "app:control"
                                     (jaunder--keyword-repair-payload member-xml (plist-get row :union)))))))))

(provide 'jaunder-keyword-repair-test)
;;; jaunder-keyword-repair-test.el ends here
