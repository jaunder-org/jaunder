;;; run-tests.el --- ERT batch runner for jaunder -*- lexical-binding: t; -*-

;;; Commentary:
;; Loads the jaunder package and every test/*-test.el, then runs ERT in
;; batch mode.  Self-locating via `load-file-name' so it works both from the
;; repo root (the `ert' step, run via `devtool check ert') and from the nix
;; store (the `static-checks' derivation, via `devtool check --all').

;;; Code:

(require 'ert)

(let* ((this (file-name-directory
              (or load-file-name buffer-file-name default-directory)))
       (root (file-name-directory (directory-file-name this)))
       (test-dir (expand-file-name "test" root)))
  (add-to-list 'load-path root)
  (require 'jaunder)
  (dolist (f (directory-files test-dir t "-test\\.el\\'"))
    (load f nil t)))

(ert-run-tests-batch-and-exit)

;;; run-tests.el ends here
