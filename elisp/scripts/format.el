;;; format.el --- elisp formatting driver for jaunder -*- lexical-binding: t; -*-

;;; Commentary:
;; `jaunder-fmt-fix' reindents every .el under elisp/ in place;
;; `jaunder-fmt-check' exits non-zero if any is not canonically formatted.
;; Uses built-in emacs-lisp-mode indentation + trailing-whitespace removal.
;; Self-locating via `load-file-name' (repo root under xtask, nix store under
;; the hermetic check).

;;; Code:

(defun jaunder-fmt--files ()
  "Return all .el files under the elisp/ subproject."
  (let* ((this (file-name-directory
                (or load-file-name buffer-file-name default-directory)))
         (root (file-name-directory (directory-file-name this))))
    (directory-files-recursively root "\\.el\\'")))

(defun jaunder-fmt--canonical (file)
  "Return the canonically-formatted contents of FILE as a string."
  (with-temp-buffer
    (insert-file-contents file)
    (delay-mode-hooks (emacs-lisp-mode))
    (let ((indent-tabs-mode nil))
      (indent-region (point-min) (point-max)))
    (delete-trailing-whitespace)
    (buffer-string)))

(defun jaunder-fmt--raw (file)
  "Return the on-disk contents of FILE as a string."
  (with-temp-buffer
    (insert-file-contents file)
    (buffer-string)))

(defun jaunder-fmt-fix ()
  "Reindent every elisp file in place."
  (dolist (f (jaunder-fmt--files))
    (let ((formatted (jaunder-fmt--canonical f)))
      (unless (string= formatted (jaunder-fmt--raw f))
        (with-temp-file f (insert formatted))))))

(defun jaunder-fmt-check ()
  "Exit non-zero if any elisp file is not canonically formatted."
  (let ((bad '()))
    (dolist (f (jaunder-fmt--files))
      (unless (string= (jaunder-fmt--canonical f) (jaunder-fmt--raw f))
        (push f bad)))
    (when bad
      (message "elisp-fmt: not canonically formatted:\n%s"
               (mapconcat #'identity (nreverse bad) "\n"))
      (kill-emacs 1))))

;;; format.el ends here
