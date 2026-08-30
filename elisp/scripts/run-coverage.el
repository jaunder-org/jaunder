;;; run-coverage.el --- combined Elisp coverage batch runner -*- lexical-binding: t; -*-

;;; Commentary:
;; Invoked only by the hermetic producer VM.  It deliberately exits zero for
;; controlled coverage outcomes; the artifact consumer owns the final verdict.

;;; Code:

(let* ((this (file-name-directory
              (or load-file-name buffer-file-name default-directory)))
       (root (file-name-directory (directory-file-name this)))
       (output (or (getenv "JAUNDER_ELISP_COVERAGE_DIR")
                   (error "JAUNDER_ELISP_COVERAGE_DIR is required"))))
  (load (expand-file-name "jaunder-coverage.el" this) nil t)
  (jaunder-coverage-run root output))

;;; run-coverage.el ends here
