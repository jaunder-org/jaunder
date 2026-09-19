;;; jaunder-post-link-test.el --- ERT suite for Local Post Links -*- lexical-binding: t; -*-

;;; Commentary:
;; Publish-preflight tracer bullets for Local Post Link candidate precedence and
;; exact identity proof.

;;; Code:

(require 'ert)
(require 'cl-lib)
(require 'jaunder)

(defun jaunder-post-link-test--member (id slug href &optional invalid-reason)
  "Return a Member fixture with ID, SLUG, HREF, and INVALID-REASON."
  (jaunder--make-inventory-member
   :id id :slug slug :edit-uri (format "https://blog/atompub/alice/posts/%s" id)
   :alternate-href href :alternate-invalid-reason invalid-reason))

(defun jaunder-post-link-test--write-target (path id slug)
  "Write PATH with local Post ID and SLUG evidence."
  (write-region (format "#+PROPERTY: JAUNDER_ID %s\n#+PROPERTY: JAUNDER_SLUG %s\n\nTarget"
                        (or id "") (or slug ""))
                nil path nil 'silent))

(defmacro jaunder-post-link-test--with-root (bindings &rest body)
  "Create ROOT and SOURCE configured for BODY."
  (declare (indent 1))
  (let ((root (car bindings))
        (source (cadr bindings)))
    `(let* ((,root (file-name-as-directory (make-temp-file "jt-post-link-" t)))
            (,source (expand-file-name "source.org" ,root))
            (jaunder-blogs (list (cons ,root '(:base-url "https://blog" :username "alice")))))
       (unwind-protect
           (progn ,@body)
         (when (get-file-buffer ,source) (kill-buffer (get-file-buffer ,source)))
         (delete-directory ,root t)))))

(ert-deftest jaunder-local-post-link-requires-a-configured-root ()
  "A Local Post Link cannot resolve outside configured Jaunder roots."
  (with-temp-buffer
    (let ((buffer-file-name "/tmp/unconfigured.org")
          (jaunder-blogs nil))
      (should-error (jaunder--local-post-link-root)))))

(ert-deftest jaunder-local-post-link-candidates-precede-media-and-reject-escapes ()
  "Unsupported candidate syntax and escaped targets never become media."
  (jaunder-post-link-test--with-root (root source)
                                     (let ((outside (expand-file-name "outside.org" (file-name-directory (directory-file-name root))))
                                           fetched)
                                       (jaunder-post-link-test--write-target outside "7" "outside")
                                       (dolist (link '("./target.org::heading" "./target.org?query" "./target.org#fragment"
                                                       "../outside.org"))
                                         (write-region (format "#+TITLE: Source\n\n[[%s]]" link) nil source nil 'silent)
                                         (when (get-file-buffer source) (kill-buffer (get-file-buffer source)))
                                         (with-current-buffer (find-file-noselect source)
                                           (cl-letf (((symbol-function 'jaunder--fetch-collection-members)
                                                      (lambda () (setq fetched t))))
                                             (let ((body (jaunder-entry-body (jaunder--org->atom))))
                                               (should-error (jaunder--localize-post-links body))
                                               (should-not (jaunder--collect-media-links)))))
                                         (setq fetched nil))
                                       (delete-file outside))
                                     (let* ((outside (make-temp-file "jt-post-link-outside-" nil ".org"))
                                            (link (expand-file-name "escape.org" root)))
                                       (unwind-protect
                                           (progn
                                             (jaunder-post-link-test--write-target outside "7" "outside")
                                             (make-symbolic-link outside link)
                                             (write-region "#+TITLE: Source\n\n[[./escape.org]]" nil source nil 'silent)
                                             (when (get-file-buffer source) (kill-buffer (get-file-buffer source)))
                                             (with-current-buffer (find-file-noselect source)
                                               (let ((body (jaunder-entry-body (jaunder--org->atom))))
                                                 (should-error (jaunder--localize-post-links body))
                                                 (should-not (jaunder--collect-media-links)))))
                                         (delete-file outside)))))

(ert-deftest jaunder-local-post-link-rejects-draft-orphan-and-bad-evidence ()
  "Only a referenced target's complete local identity proof permits a join."
  (jaunder-post-link-test--with-root (root source)
                                     (let ((target (expand-file-name "target.org" root))
                                           (member (jaunder-post-link-test--member "7" "target" "https://blog/@alice/target")))
                                       (dolist (fixture '((nil "target" "target.org" nil)
                                                          ("7" "target" "target.org" nil)
                                                          ("7" "other" "target.org" t)
                                                          ("7" "target" "wrong.org" t)))
                                         (let ((id (nth 0 fixture)) (slug (nth 1 fixture)) (name (nth 2 fixture))
                                               (has-member (nth 3 fixture)))
                                           (when (file-exists-p target) (delete-file target))
                                           (setq target (expand-file-name name root))
                                           (jaunder-post-link-test--write-target target id slug)
                                           (write-region (format "#+TITLE: Source\n\n[[./%s]]" name) nil source nil 'silent)
                                           (when (get-file-buffer source) (kill-buffer (get-file-buffer source)))
                                           (with-current-buffer (find-file-noselect source)
                                             (cl-letf (((symbol-function 'jaunder--fetch-collection-members)
                                                        (lambda () (if has-member (list member) nil))))
                                               (let ((body (jaunder-entry-body (jaunder--org->atom))))
                                                 (should-error (jaunder--localize-post-links body))
                                                 (should-not (jaunder--collect-media-links))))))))))

(ert-deftest jaunder-pulled-post-links-reverse-only-exact-proven-body-destinations ()
  "Pull reversal preserves everything except exact, uniquely proven Org targets."
  (jaunder-post-link-test--with-root (root source)
                                     (let* ((target (expand-file-name "target.org" root))
                                            (member (jaunder-post-link-test--member
                                                     "7" "target" "https://blog/@alice/target"))
                                            (local (jaunder--make-inventory-local
                                                    :path target :id "7" :slug "target"))
                                            (body (concat "before [[https://blog/@alice/target][kept description]] after\n"
                                                          "https://blog/@alice/target\n"
                                                          "#+DESCRIPTION: [[https://blog/@alice/target]]\n"
                                                          "#+begin_src text\n[[https://blog/@alice/target]]\n#+end_src\n"
                                                          "[[https://blog/%40alice/target]] [[https://blog/media/x]]")))
                                       (jaunder-post-link-test--write-target target "7" "target")
                                       (should
                                        (equal (jaunder--reverse-pulled-post-links
                                                body root (list member) (list local))
                                               (concat "before [[./target.org][kept description]] after\n"
                                                       "https://blog/@alice/target\n"
                                                       "#+DESCRIPTION: [[https://blog/@alice/target]]\n"
                                                       "#+begin_src text\n[[https://blog/@alice/target]]\n#+end_src\n"
                                                       "[[https://blog/%40alice/target]] [[https://blog/media/x]]"))))))

(ert-deftest jaunder-pulled-post-links-require-complete-unambiguous-evidence ()
  "Partial, invalid, or ambiguous inventories retain canonical destinations."
  (jaunder-post-link-test--with-root (root source)
                                     (let* ((target (expand-file-name "target.org" root))
                                            (member (jaunder-post-link-test--member
                                                     "7" "target" "https://blog/@alice/target"))
                                            (local (jaunder--make-inventory-local
                                                    :path target :id "7" :slug "target"))
                                            (body "[[https://blog/@alice/target]]"))
                                       (jaunder-post-link-test--write-target target "7" "target")
                                       (dolist (evidence
                                                (list (list (list member) nil)
                                                      (list (list member)
                                                            (list (jaunder--make-inventory-local
                                                                   :path target :id "8" :slug "target")))
                                                      (list (list member
                                                                  (jaunder-post-link-test--member
                                                                   "7" "target" "https://blog/@alice/target"))
                                                            (list local))))
                                         (should (equal (jaunder--reverse-pulled-post-links
                                                         body root (nth 0 evidence) (nth 1 evidence))
                                                        body)))
                                       ;; Inventory evidence is only a candidate path: pull re-reads
                                       ;; identity so an edit after reconciliation cannot authorize a rewrite.
                                       (jaunder-post-link-test--write-target target "8" "target")
                                       (should (equal (jaunder--reverse-pulled-post-links
                                                       body root (list member) (list local))
                                                      body)))))

(ert-deftest jaunder-local-post-link-uses-only-referenced-member-and-distinct-hrefs ()
  "A bad unrelated Member does not block exact distinct target substitutions."
  (jaunder-post-link-test--with-root (root source)
                                     (let* ((one (expand-file-name "one.org" root))
                                            (two (expand-file-name "two.org" root))
                                            (members (list (jaunder-post-link-test--member "1" "one" "https://blog/@alice/one")
                                                           (jaunder-post-link-test--member "2" "two" "https://blog/@alice/two")
                                                           (jaunder-post-link-test--member "3" "bad" nil 'alternate-missing))))
                                       (jaunder-post-link-test--write-target one "1" "one")
                                       (jaunder-post-link-test--write-target two "2" "two")
                                       (write-region "#+TITLE: Source\n\n[[./one.org][one]] [[file:two.org][two]]"
                                                     nil source nil 'silent)
                                       (with-current-buffer (find-file-noselect source)
                                         (cl-letf (((symbol-function 'jaunder--fetch-collection-members) (lambda () members)))
                                           (let ((body (jaunder-entry-body (jaunder--org->atom))))
                                             (should (equal (jaunder--localize-post-links body)
                                                            "[[https://blog/@alice/one][one]] [[https://blog/@alice/two][two]]")))))
                                       (setf (jaunder-inventory-member-alternate-href (car members)) nil
                                             (jaunder-inventory-member-alternate-invalid-reason (car members))
                                             'alternate-missing)
                                       (with-current-buffer (find-file-noselect source)
                                         (cl-letf (((symbol-function 'jaunder--fetch-collection-members) (lambda () members)))
                                           (should-error (jaunder--localize-post-links
                                                          (jaunder-entry-body (jaunder--org->atom)))))))))

;;; jaunder-post-link-test.el ends here
