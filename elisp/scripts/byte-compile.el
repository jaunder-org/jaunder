;;; byte-compile.el --- byte-compile the jaunder package, warnings-as-errors -*- lexical-binding: t; -*-

;;; Commentary:
;; Byte-compiles every package module (the flat elisp/*.el files) with all
;; byte-compiler warnings promoted to errors, so any warning fails the gate.
;; Output goes to a throwaway temp dir — no .elc is left in the tree.
;; Self-locating via `load-file-name' so it works from the repo root (the
;; `byte-compile' step, via `devtool check byte-compile') and from the nix
;; store (the `static-checks' derivation, via `devtool check --all').

;;; Code:

;; Require bytecomp before let-binding its options: under lexical-binding,
;; binding `byte-compile-dest-file-function' before the library defines it as a
;; special variable errors with "Defining as dynamic an already lexical var".
(require 'bytecomp)

(let* ((this (file-name-directory
              (or load-file-name buffer-file-name default-directory)))
       (root (file-name-directory (directory-file-name this)))
       (dest (make-temp-file "jaunder-bytecomp" t))
       (byte-compile-dest-file-function
        (lambda (src)
          (expand-file-name (concat (file-name-nondirectory src) "c") dest)))
       ;; Promote every warning to a failure.  `-Q' already leaves
       ;; `byte-compile-warnings' at its default t (all warnings enabled), so
       ;; only error-on-warn is load-bearing here.
       (byte-compile-error-on-warn t)
       (ok t))
  (add-to-list 'load-path root)
  ;; Package modules only: the flat elisp/*.el files (scripts/ and test/ are
  ;; subdirectories, so a non-recursive listing excludes them).
  (dolist (f (directory-files root t "\\.el\\'"))
    (unless (byte-compile-file f)
      (setq ok nil)))
  (delete-directory dest t)
  (unless ok (kill-emacs 1)))

;;; byte-compile.el ends here
