;;; jaunder-wait-test.el --- unit tests for the readiness poller -*- lexical-binding: t; -*-

;;; Commentary:
;; Pure-suite tests for `jaunder-test--wait' (the readiness poll shared by the
;; live-server harness's three gates).  No server needed — the predicate is a
;; plain closure — so these run in the fast `-test.el' suite and give the #628
;; budget change a deterministic, host-run check instead of a CI-only signal.

;;; Code:

(require 'ert)
;; The pure runner (run-tests.el) only puts elisp/ on `load-path'; add our own
;; directory (elisp/test/) so the harness helper resolves when loaded from there.
(add-to-list 'load-path
             (file-name-directory (or load-file-name buffer-file-name default-directory)))
(require 'jaunder-integration-helper)

(ert-deftest jaunder-test--wait-returns-value-after-slow-start ()
  "Succeeds once the predicate turns non-nil, well within the budget."
  (let ((n 0))
    (should (eq 'ok (jaunder-test--wait
                     (lambda () (if (>= (setq n (1+ n)) 3) 'ok nil))
                     "thing" 2)))))

(ert-deftest jaunder-test--wait-errors-with-what-on-timeout ()
  "A never-true predicate errors within its (small) budget, naming WHAT."
  (let ((err (should-error (jaunder-test--wait (lambda () nil) "widget readiness" 0.3))))
    (should (string-match-p "widget readiness" (error-message-string err)))))

(ert-deftest jaunder-test--wait-honors-env-timeout ()
  "With no explicit TIMEOUT, $JAUNDER_TEST_READY_TIMEOUT bounds the wait."
  (let ((process-environment (cons "JAUNDER_TEST_READY_TIMEOUT=0.2" process-environment))
        (start (float-time)))
    (should-error (jaunder-test--wait (lambda () nil) "thing"))
    (should (< (- (float-time) start) 2.0))))

(provide 'jaunder-wait-test)
;;; jaunder-wait-test.el ends here
