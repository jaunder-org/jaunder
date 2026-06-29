;;; run-integration-tests.el --- live-server ERT runner for jaunder -*- lexical-binding: t; -*-

;;; Commentary:
;; Loads jaunder + the integration helper and every test/*-integration.el, then
;; runs ERT in batch.  Needs a built jaunder binary via JAUNDER_TEST_BINARY or
;; PATH (ADR-0035).  Parallel to run-tests.el, which globs -test.el and so
;; excludes these server-backed tests from the fast pure suite.

;;; Code:

(require 'ert)

(let* ((this (file-name-directory
              (or load-file-name buffer-file-name default-directory)))
       (root (file-name-directory (directory-file-name this)))
       (test-dir (expand-file-name "test" root)))
  (add-to-list 'load-path root)
  (add-to-list 'load-path test-dir)
  (require 'jaunder)
  (require 'jaunder-integration-helper)
  (dolist (f (directory-files test-dir t "-integration\\.el\\'"))
    (load f nil t)))

(ert-run-tests-batch-and-exit)

;;; run-integration-tests.el ends here
