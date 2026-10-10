;;; jaunder-reconcile-page-budget-test.el --- Public traversal budgets -*- lexical-binding: t; -*-

;; Copyright (C) 2026 Jaunder contributors

;;; Commentary:
;; Opening, operation and independent refresh are separate observations.  The
;; deterministic HTTP fixture measures requests, never production latency.

;;; Code:

(require 'ert)
(load (expand-file-name "jaunder-reconcile-batch-fixture.el"
                        (file-name-directory (or load-file-name buffer-file-name))) nil t)
(load (expand-file-name "jaunder-reconcile-write-fixture.el"
                        (file-name-directory (or load-file-name buffer-file-name))) nil t)

(ert-deftest jaunder-reconcile-read-actions-bound-pages-not-selected-posts ()
  "One and three Posts retain selected Member reads alongside shared discovery."
  (dolist (ids '((1) (1 50 100)))
    (dolist (action '(pull keep-remote))
      (let ((proof (jaunder-test--confirmed-batch action (lambda (&rest _) nil) ids)))
        (should (= (plist-get proof :initial-pages) 4))
        (should (= (plist-get proof :operation-pages) 4))
        (should (= (- (plist-get proof :pages) (plist-get proof :operation-pages)) 4))
        (should (cl-every (lambda (result) (eq (jaunder-reconcile-result-outcome result) 'success))
                          (plist-get proof :results)))
        (dolist (id ids)
          (should (= (gethash id (plist-get proof :member-reads))
                     (if (eq action 'pull) 2 3))))))))

(ert-deftest jaunder-reconcile-create-refresh-follows-new-page-topology ()
  "Creating across the 100-Member boundary uses four then five pages, not a retry."
  (dolist (ids '((1) (1 50 100)))
    (let ((proof (jaunder-test--confirmed-write-batch
                  'push (lambda (event state)
                          (when (eq (plist-get event :phase) 'setup)
                            (jaunder-test--write-selected-as-creates state))) ids t)))
      (should (= (plist-get proof :initial-pages) 4))
      (should (= (plist-get proof :operation-pages) 4))
      (should (= (- (plist-get proof :pages) (plist-get proof :operation-pages)) 5))
      (should (cl-every (lambda (result) (eq (jaunder-reconcile-result-outcome result) 'success))
                        (plist-get proof :results)))
      (should (= (length (plist-get proof :writes)) (length ids)))
      (dolist (write (plist-get proof :writes))
        (should (equal (plist-get write :method) "POST"))
        (should (cdr (assoc "Idempotency-Key" (plist-get write :headers))))))))

(provide 'jaunder-reconcile-page-budget-test)
;;; jaunder-reconcile-page-budget-test.el ends here
