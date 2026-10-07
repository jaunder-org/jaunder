;;; jaunder-transform-debug-test.el --- Transformation diagnostic boundaries -*- lexical-binding: t; -*-

;;; Code:

(require 'ert)
(require 'cl-lib)
(require 'jaunder)

(defmacro jaunder-transform-debug-test--with-session (&rest body)
  "Run BODY with isolated diagnostic state."
  (declare (indent 0) (debug t))
  `(let ((jaunder-debug nil)
         (jaunder--debug-buffer-name " *Jaunder transform diagnostic tests*")
         (jaunder--debug-id-counter 0)
         (jaunder--debug-event-count 0)
         (jaunder--debug-discarded 0)
         (jaunder--debug-operation-stack nil))
     (unwind-protect (progn ,@body)
       (when-let* ((buffer (get-buffer jaunder--debug-buffer-name)))
         (kill-buffer buffer)))))

(defun jaunder-transform-debug-test--text ()
  "Return the retained diagnostic text."
  (with-current-buffer jaunder--debug-buffer-name (buffer-string)))

(defun jaunder-transform-debug-test--label-count (label text)
  "Return count of LABEL event lines in TEXT."
  (cl-count-if (lambda (line) (string-match-p (concat "label=" label) line))
               (split-string text "\n" t)))

(defun jaunder-transform-debug-test--member-xml (&optional body)
  "Return a valid draft Member XML containing BODY."
  (concat "<entry xmlns=\"http://www.w3.org/2005/Atom\""
          " xmlns:app=\"http://www.w3.org/2007/app\""
          " xmlns:j=\"https://jaunder.org/ns/atompub\">"
          "<title>title-sentinel</title>"
          "<link rel=\"edit\" href=\"https://private.example/atompub/alice/posts/1\"/>"
          "<j:slug>target</j:slug><content type=\"text/org\">"
          (or body "body-sentinel")
          "</content><app:control><app:draft>yes</app:draft></app:control></entry>"))

(defun jaunder-transform-debug-test--member ()
  "Return a parsed Member fixture without recording diagnostics."
  (jaunder--parse-pulled-member (jaunder-transform-debug-test--member-xml)
                                "\"sha256-test\"" (seconds-to-time 0) "UTC"))

(defmacro jaunder-transform-debug-test--with-root (bindings &rest body)
  "Create configured ROOT, SOURCE, and TARGET files for BODY."
  (declare (indent 1) (debug t))
  (let ((root (nth 0 bindings))
        (source (nth 1 bindings))
        (target (nth 2 bindings)))
    `(let* ((,root (file-name-as-directory (make-temp-file "jaunder-debug-link-" t)))
            (,source (expand-file-name "source.org" ,root))
            (,target (expand-file-name "target.org" ,root))
            (jaunder-blogs (list (cons ,root '(:base-url "https://private.example" :username "private-user")))))
       (unwind-protect (progn ,@body)
         (when (get-file-buffer ,source) (kill-buffer (get-file-buffer ,source)))
         (delete-directory ,root t)))))

(ert-deftest jaunder-transform-debug-org-and-member-boundaries-pair-and-hide-content ()
  "Real Org and Member transformations retain only bounded format fields."
  (jaunder-transform-debug-test--with-session
   (let ((jaunder-debug t))
     (with-temp-buffer
       (org-mode)
       (insert "#+TITLE: title-sentinel\n\nbody-sentinel")
       (let ((entry (jaunder--org->atom)))
         (should (equal (jaunder-entry-body entry) "body-sentinel"))))
     (let* ((xml (jaunder-transform-debug-test--member-xml))
            (member (jaunder--parse-pulled-member xml "\"sha256-test\"" (seconds-to-time 0) "UTC")))
       (should (equal (jaunder-pulled-member-body member) "body-sentinel"))
       (should (equal (jaunder--atom->org xml "\"sha256-test\"" (seconds-to-time 0) "UTC")
                      (jaunder-pulled-member-org member)))
       (should (equal (jaunder--pull-response-identity xml) '("1" . "target")))
       (should (equal (jaunder--render-pulled-member member "localized-sentinel")
                      (concat (jaunder-pulled-member-org-prefix member) "localized-sentinel"))))
     (let ((text (jaunder-transform-debug-test--text)))
       (dolist (label '("org.parse" "member.identity"))
         (should (= 2 (jaunder-transform-debug-test--label-count label text))))
       ;; Direct parsing, the adapter, and explicit rendering each produce Org bytes.
       (should (= 6 (jaunder-transform-debug-test--label-count "org.serialize" text)))
       (should (= 4 (jaunder-transform-debug-test--label-count "member.parse" text)))
       (should (>= (jaunder-transform-debug-test--label-count "atom.parse" text) 6))
       (should (string-match-p "format=org" text))
       (should-not (string-match-p
                    "title-sentinel\\|body-sentinel\\|localized-sentinel\\|private.example\\|sha256-test"
                    text))))))

(ert-deftest jaunder-transform-debug-member-format-producer-is-closed-and-byte-preserving ()
  "Every accepted wire format projects identically off/on without source leakage."
  (jaunder-transform-debug-test--with-session
   (dolist (case '(("text/org" . "org") ("text/markdown" . "markdown")
                   ("html" . "html") ("text/html" . "html") ("xhtml" . "html")))
     (let* ((body (if (equal (car case) "xhtml")
                      "<div xmlns=\"http://www.w3.org/1999/xhtml\"><p>body-sentinel</p></div>"
                    "body-sentinel"))
            (xml (replace-regexp-in-string
                  (regexp-quote "type=\"text/org\"") (concat "type=\"" (car case) "\"")
                  (jaunder-transform-debug-test--member-xml body) t t))
            members)
       (dolist (enabled '(nil t))
         (let ((jaunder-debug enabled))
           (push (jaunder--parse-pulled-member xml "\"sha256-test\"" (seconds-to-time 0) "UTC")
                 members)))
       (should (equal (car members) (cadr members)))
       (should (equal (cdr case) (jaunder-pulled-member-format (car members))))))
   (let ((jaunder-debug t))
     (should-error
      (jaunder--parse-pulled-member
       (replace-regexp-in-string "text/org" "format-sentinel"
                                 (jaunder-transform-debug-test--member-xml) t t)
       "\"sha256-test\"" (seconds-to-time 0) "UTC")))
   (let ((text (jaunder-transform-debug-test--text)))
     (dolist (format '("org" "markdown" "html"))
       (should (string-match-p (concat "format=" format) text)))
     (should-not (string-match-p "sentinel\\|private.example\\|sha256-test" text)))))

(ert-deftest jaunder-transform-debug-serialization-nests-under-member-parsing ()
  "The parser's original Org byte construction owns a serialization child span."
  (jaunder-transform-debug-test--with-session
   (let ((jaunder-debug t))
     (jaunder-transform-debug-test--member)
     (let ((text (jaunder-transform-debug-test--text)))
       (should (string-match "span=\\([^ ]+\\) label=member.parse phase=start" text))
       (let ((parent (match-string 1 text)))
         (should (string-match-p
                  (regexp-quote (concat "label=org.serialize phase=start parent=" parent " "))
                  text)))
       (should (= 2 (jaunder-transform-debug-test--label-count "org.serialize" text)))))))

(ert-deftest jaunder-transform-debug-post-link-boundaries-preserve-bytes-and-work ()
  "Publish/pull/evidence spans keep exact replacements and request/work bounds."
  (jaunder-transform-debug-test--with-session
   (jaunder-transform-debug-test--with-root (root source target)
                                            (write-region "#+PROPERTY: JAUNDER_ID 1\n#+PROPERTY: JAUNDER_SLUG target\n\nbody-sentinel"
                                                          nil target nil 'silent)
                                            (write-region "#+TITLE: Source\n\n[[./target.org][description]]" nil source nil 'silent)
                                            (let* ((member (jaunder--make-inventory-member
                                                            :id "1" :slug "target"
                                                            :edit-uri "https://private.example/atompub/alice/posts/1"
                                                            :alternate-href "https://private.example/@alice/target"))
                                                   (local (jaunder--make-inventory-local :path target :id "1" :slug "target"))
                                                   (body "[[https://private.example/@alice/target][description]]")
                                                   (requests 0)
                                                   (checks 0))
                                              (with-current-buffer (find-file-noselect source)
                                                (jaunder--call-with-blog
                                                 source
                                                 (lambda ()
                                                   (let ((jaunder-debug nil))
                                                     (cl-letf (((symbol-function 'jaunder--fetch-collection-members)
                                                                (lambda () (setq requests (1+ requests)) (list member))))
                                                       (should (equal (jaunder--localize-post-links "[[./target.org][description]]")
                                                                      "[[https://private.example/@alice/target][description]]"))
                                                       (should (= requests 1))))
                                                   (let ((jaunder-debug t)
                                                         (real-proof (symbol-function 'jaunder--pulled-post-link-target-p)))
                                                     (cl-letf (((symbol-function 'jaunder--fetch-collection-members)
                                                                (lambda () (setq requests (1+ requests)) (list member)))
                                                               ((symbol-function 'jaunder--pulled-post-link-target-p)
                                                                (lambda (candidate remote directory)
                                                                  (setq checks (1+ checks))
                                                                  (funcall real-proof candidate remote directory))))
                                                       (should (equal (jaunder--localize-post-links "[[./target.org][description]]")
                                                                      "[[https://private.example/@alice/target][description]]"))
                                                       (should (equal (jaunder--reverse-pulled-post-links body root (list member) (list local))
                                                                      "[[./target.org][description]]"))))))
                                                (should (= requests 2))
                                                (should (= checks 1)))
                                              (let ((text (jaunder-transform-debug-test--text)))
                                                (should (= 2 (jaunder-transform-debug-test--label-count "post-link.publish" text)))
                                                (should (= 2 (jaunder-transform-debug-test--label-count "post-link.pull" text)))
                                                (should (= 2 (jaunder-transform-debug-test--label-count "post-link.evidence" text)))
                                                (should (string-match-p "members=1" text))
                                                (should (string-match-p "count=1" text))
                                                (should-not (string-match-p "private.example\\|body-sentinel\\|description" text)))))))

(ert-deftest jaunder-transform-debug-preserves-errors-quit-and-disabled-bypass ()
  "Actual boundaries retain signals and disabled calls skip diagnostic work."
  (jaunder-transform-debug-test--with-session
   (with-temp-buffer
     (org-mode)
     (insert "#+TITLE: one\n#+TITLE: two\n\nbody")
     (let ((jaunder-debug t)) (should-error (jaunder--org->atom))))
   (let ((jaunder-debug t))
     (should-error (jaunder--pull-response-identity "<private-content")))
   (jaunder-transform-debug-test--with-root (root source target)
                                            (write-region "#+PROPERTY: JAUNDER_ID 1\n#+PROPERTY: JAUNDER_SLUG target\n" nil target nil 'silent)
                                            (let ((member (jaunder--make-inventory-member :id "1" :slug "target"
                                                                                          :alternate-href "https://private.example/target"))
                                                  (local (jaunder--make-inventory-local :path target :id "1" :slug "target")))
                                              (let ((jaunder-debug t))
                                                (cl-letf (((symbol-function 'jaunder--read-local-properties)
                                                           (lambda (&rest _) (signal 'quit '(private-quit)))))
                                                  (should (eq 'quit
                                                              (condition-case err
                                                                  (jaunder--reverse-pulled-post-links
                                                                   "[[https://private.example/target]]" root (list member) (list local))
                                                                (quit (car err))))))))
                                            (with-current-buffer (find-file-noselect source)
                                              (insert "#+TITLE: private-title\n\nprivate-body")
                                              (let ((jaunder-debug nil)
                                                    (before jaunder--debug-id-counter))
                                                (cl-letf (((symbol-function 'jaunder--debug-now) (lambda () (error "clock")))
                                                          ((symbol-function 'jaunder--debug-format-event) (lambda (_) (error "format")))
                                                          ((symbol-function 'jaunder--debug-buffer) (lambda () (error "buffer"))))
                                                  (should (jaunder--org->atom))
                                                  (should (jaunder--parse-pulled-member (jaunder-transform-debug-test--member-xml)
                                                                                        "\"sha256-test\"" (seconds-to-time 0) "UTC"))
                                                  (should (jaunder--render-pulled-member (jaunder-transform-debug-test--member) "private-body"))
                                                  (should (equal (jaunder--pull-response-identity (jaunder-transform-debug-test--member-xml))
                                                                 '("1" . "target")))
                                                  (should (equal "x" (jaunder--localize-post-links "x")))
                                                  (should (equal "x" (jaunder--reverse-pulled-post-links "x" root nil nil))))
                                                (should (= jaunder--debug-id-counter before))))
                                            (let ((text (jaunder-transform-debug-test--text)))
                                              (should (string-match-p "outcome=error" text))
                                              (should (string-match-p "outcome=cancelled" text))
                                              (should-not (string-match-p "private-content\\|private-quit\\|private-title\\|private-body" text))))))

;;; jaunder-transform-debug-test.el ends here
