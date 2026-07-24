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

;; #628: one shared server for the whole suite — the readiness gates run once,
;; not once per test.  Tests reuse it through `jaunder-test--with-live-server',
;; which passes through when the harness globals are already bound.  Teardown
;; must precede exit, so this mirrors `ert-run-tests-batch-and-exit' by hand.
(let ((st (jaunder-test--server-up-retrying)))
  (jaunder-test--set-globals st)
  (let ((stats (unwind-protect
                   (ert-run-tests-batch)
                 (jaunder-test--server-down st))))
    (kill-emacs (if (zerop (ert-stats-completed-unexpected stats)) 0 1))))

;;; run-integration-tests.el ends here
