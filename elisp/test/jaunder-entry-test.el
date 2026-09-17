;;; jaunder-entry-test.el --- ERT suite for jaunder-entry -*- lexical-binding: t; -*-

;;; Commentary:
;; Unit tests for the corresponding jaunder package module.

;;; Code:

(require 'ert)
(require 'jaunder)

(ert-deftest jaunder-tag-validation-uses-one-structured-grammar-result ()
  "Validation and repair diagnostics share each Tag grammar classification."
  (should (equal (jaunder--tag-label-defect nil) '(wrong-type . 0)))
  (should (equal (jaunder--tag-label-defect "  ") '(empty . 0)))
  (should (equal (jaunder--tag-label-defect "-topic") '(invalid-start . 0)))
  (should (equal (jaunder--tag-label-defect "two words")
                 '(invalid-character . 3)))
  (should-not (jaunder--tag-label-defect " Rust-2 "))
  (should (jaunder--valid-tag-label-p " Rust-2 "))
  (should (jaunder--valid-tag-slug-p "rust-2"))
  (should-not (jaunder--valid-tag-slug-p "Rust-2"))
  (should-not (jaunder--valid-tag-slug-p " rust-2 ")))

(provide 'jaunder-entry-test)
;;; jaunder-entry-test.el ends here
