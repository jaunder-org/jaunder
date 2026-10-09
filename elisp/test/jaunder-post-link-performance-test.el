;;; jaunder-post-link-performance-test.el --- Bounded pull proof -*- lexical-binding: t; -*-

;; Copyright (C) 2026 Jaunder contributors

;;; Commentary:
;; Real local files and public canonical hrefs exercise the production proof
;; branch.  A work-count bound detects all-to-all validation independently of
;; machine speed, stopping an excessive traversal before it burns minutes.

;;; Code:

(require 'ert)
(require 'cl-lib)
(require 'jaunder)

(ert-deftest jaunder-pulled-post-links-bound-filesystem-proof-at-inventory-scale ()
  "Public Members join local identity before filesystem proof at inventory scale."
  (dolist (count '(100 1000))
    (let* ((root (make-temp-file "jaunder-link-scale-" t))
           (jaunder-blogs
            (list (cons (file-name-as-directory root)
                        '(:base-url "https://example.test" :username "alice"))))
           (body (concat "An image: [[https://example.test/media/image.png]]\n"
                         "A Post: [[https://example.test/~alice/post-001][description]]\n"))
           (expected (concat "An image: [[https://example.test/media/image.png]]\n"
                             "A Post: [[./post-001.org][description]]\n"))
           (real-proof (symbol-function 'jaunder--pulled-post-link-target-p))
           (checks 0)
           members locals)
      (unwind-protect
          (progn
            (dolist (id (number-sequence 1 count))
              (let* ((slug (format "post-%03d" id))
                     (path (expand-file-name (concat slug ".org") root)))
                (with-temp-file path
                  (insert (format "#+PROPERTY: JAUNDER_ID %d\n#+PROPERTY: JAUNDER_SLUG %s\n\nBody.\n"
                                  id slug)))
                (push (jaunder--make-inventory-local
                       :id (number-to-string id) :slug slug :path path) locals)
                (push (jaunder--make-inventory-member
                       :id (number-to-string id) :slug slug
                       :edit-uri (format "https://example.test/atompub/alice/posts/%d" id)
                       :alternate-href (format "https://example.test/~alice/%s" slug))
                      members)))
            (cl-letf (((symbol-function 'jaunder--pulled-post-link-target-p)
                       (lambda (local member directory)
                         (setq checks (1+ checks))
                         (should (<= checks count))
                         (funcall real-proof local member directory))))
              (should (equal (jaunder--reverse-pulled-post-links body root members locals)
                             expected)))
            (should (<= checks count)))
        (delete-directory root t)))))

(ert-deftest jaunder-pulled-post-links-index-retains-competing-local-and-href-proof ()
  "Indexed joins cannot discard competing candidates and invent a winner."
  (let* ((root (make-temp-file "jaunder-link-ambiguity-" t))
         (one (expand-file-name "one.org" root))
         (two (expand-file-name "two.org" root))
         (href "https://example.test/~alice/shared")
         (body (format "[[%s][unchanged]]" href))
         (local-one (jaunder--make-inventory-local :path one :id "1" :slug "one"))
         (local-two (jaunder--make-inventory-local :path two :id "2" :slug "two"))
         (member-one (jaunder--make-inventory-member :id "1" :slug "one" :alternate-href href))
         (member-two (jaunder--make-inventory-member :id "2" :slug "two" :alternate-href href)))
    (unwind-protect
        (progn
          (with-temp-file one (insert "#+PROPERTY: JAUNDER_ID 1\n#+PROPERTY: JAUNDER_SLUG one\n"))
          (with-temp-file two (insert "#+PROPERTY: JAUNDER_ID 2\n#+PROPERTY: JAUNDER_SLUG two\n"))
          (should (equal body (jaunder--reverse-pulled-post-links
                               body root (list member-one) (list local-one local-one))))
          (should (equal body (jaunder--reverse-pulled-post-links
                               body root (list member-one member-two) (list local-one local-two)))))
      (delete-directory root t))))

(ert-deftest jaunder-pulled-post-links-reject-all-duplicate-inventory-evidence ()
  "Duplicate IDs or hrefs stay ambiguous even when one candidate is stale."
  (let* ((root (make-temp-file "jaunder-link-duplicate-" t))
         (path (expand-file-name "one.org" root))
         (href "https://example.test/~alice/one")
         (other-href "https://example.test/~alice/other")
         (body (format "[[%s][one]] [[%s][other]]" href other-href))
         (local (jaunder--make-inventory-local :path path :id "1" :slug "one"))
         (member (jaunder--make-inventory-member :id "1" :slug "one" :alternate-href href)))
    (unwind-protect
        (progn
          (with-temp-file path (insert "#+PROPERTY: JAUNDER_ID 1\n#+PROPERTY: JAUNDER_SLUG one\n"))
          (dolist (case
                   (list
                    (list (list member (jaunder--make-inventory-member
                                        :id "1" :slug "one" :alternate-href other-href))
                          (list local))
                    (list (list member) (list local (jaunder--make-inventory-local
                                                     :path path :id "1" :slug "stale")))
                    (list (list member (jaunder--make-inventory-member
                                        :id "2" :slug "absent" :alternate-href href))
                          (list local))))
            (let ((checks 0))
              (cl-letf (((symbol-function 'jaunder--pulled-post-link-target-p)
                         (lambda (&rest _) (setq checks (1+ checks)) t)))
                (should (equal body (jaunder--reverse-pulled-post-links
                                     body root (car case) (cadr case)))))
              (should (= 0 checks)))))
      (delete-directory root t))))

(ert-deftest jaunder-pulled-post-links-index-preserves-current-file-evidence ()
  "Stale, missing, mismatched and out-of-root local proof leaves source intact."
  (let* ((root (make-temp-file "jaunder-link-current-" t))
         (outside (make-temp-file "jaunder-link-outside-" t))
         (target (expand-file-name "target.org" root))
         (escaped (expand-file-name "target.org" outside))
         (href "https://example.test/~alice/target")
         (body (format "[[%s][unchanged]]" href))
         (member (jaunder--make-inventory-member :id "1" :slug "target" :alternate-href href)))
    (unwind-protect
        (progn
          (with-temp-file target (insert "#+PROPERTY: JAUNDER_ID 2\n#+PROPERTY: JAUNDER_SLUG target\n"))
          (with-temp-file escaped (insert "#+PROPERTY: JAUNDER_ID 1\n#+PROPERTY: JAUNDER_SLUG target\n"))
          (dolist (local (list (jaunder--make-inventory-local :path target :id "1" :slug "target")
                               (jaunder--make-inventory-local :path target :id "2" :slug "target")
                               (jaunder--make-inventory-local :path target :id "1" :slug "wrong")
                               (jaunder--make-inventory-local :path escaped :id "1" :slug "target")
                               (jaunder--make-inventory-local :path (expand-file-name "missing/target.org" root)
                                                              :id "1" :slug "target")))
            (should (equal body (jaunder--reverse-pulled-post-links body root (list member) (list local))))))
      (delete-directory root t)
      (delete-directory outside t))))

(provide 'jaunder-post-link-performance-test)
;;; jaunder-post-link-performance-test.el ends here
