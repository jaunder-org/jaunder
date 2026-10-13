;;; jaunder-reconcile-batch-link-test.el --- Current batch Post-link evidence -*- lexical-binding: t; -*-

;; Copyright (C) 2026 Jaunder contributors

;;; Commentary:
;; Server-only and mixed batches retain current local target proof while sharing
;; one remote Collection across all link-localization and replacement checks.

;;; Code:

(require 'ert)
(load (expand-file-name "jaunder-reconcile-batch-fixture.el"
                        (file-name-directory (or load-file-name buffer-file-name))) nil t)

(ert-deftest jaunder-reconcile-server-only-batch-localizes-new-targets-with-one-collection ()
  "A later Post can link locally to an earlier pull, but never a missing target."
  (let* ((proof
          (jaunder-test--confirmed-batch
           'pull
           (lambda (event state)
             (when (eq (plist-get event :phase) 'setup)
               (dolist (row (plist-get state :rows))
                 (delete-file (jaunder-inventory-local-path (jaunder-reconcile-row-local row)))
                 (setf (jaunder-reconcile-row-state row) 'server-only
                       (jaunder-reconcile-row-local row) nil))
               (dolist (id '(1 100))
                 (puthash id (jaunder-test--batch-entry id "[[https://example.test/~alice/post-050][Target]]")
                          (plist-get state :entries))))
             nil)))
         (bytes (plist-get proof :bytes)))
    (should (equal (mapcar #'jaunder-reconcile-result-outcome (plist-get proof :results))
                   '(success success success)))
    (should (string-match-p (regexp-quote "[[https://example.test/~alice/post-050][Target]]") (car bytes)))
    (should (string-match-p (regexp-quote "[[./post-050.org][Target]]") (caddr bytes)))
    (should (= (plist-get proof :operation-pages) 4))
    (should (= (plist-get proof :pages) 8))))

(ert-deftest jaunder-reconcile-mixed-batch-rechecks-current-post-link-targets ()
  "A changed target's identity prevents localization from stale report evidence."
  (let* ((proof
          (jaunder-test--confirmed-batch
           'pull
           (lambda (event state)
             (pcase (plist-get event :phase)
               ('setup
                (let ((first (car (plist-get state :rows))))
                  (delete-file (jaunder-inventory-local-path (jaunder-reconcile-row-local first)))
                  (setf (jaunder-reconcile-row-state first) 'server-only
                        (jaunder-reconcile-row-local first) nil))
                (with-temp-file (expand-file-name "post-050.org" (plist-get state :root))
                  (insert "#+PROPERTY: JAUNDER_ID 50\n#+PROPERTY: JAUNDER_SLUG post-050\n\nTarget.\n"))
                (dolist (id '(1 100))
                  (puthash id (jaunder-test--batch-entry id "[[https://example.test/~alice/post-050][Target]]")
                           (plist-get state :entries))))
               ('member
                (when (= (plist-get event :id) 1)
                  (with-temp-file (expand-file-name "post-050.org" (plist-get state :root))
                    (insert "#+PROPERTY: JAUNDER_ID 99\n#+PROPERTY: JAUNDER_SLUG post-050\n\nOther identity.\n")))))
             nil) '(1 100)))
         (bytes (plist-get proof :bytes)))
    (should (equal (mapcar #'jaunder-reconcile-result-outcome (plist-get proof :results))
                   '(success success)))
    ;; The matched row precedes the server-only row in the displayed report.
    (should (string-match-p (regexp-quote "[[./post-050.org][Target]]") (cadr bytes)))
    (should (string-match-p (regexp-quote "[[https://example.test/~alice/post-050][Target]]") (car bytes)))
    (should (= (plist-get proof :operation-pages) 4))
    (should (= (plist-get proof :pages) 8))))

(ert-deftest jaunder-reconcile-matched-batch-link-proof-follows-renames-and-duplicates ()
  "Earlier canonical renames are usable; newly ambiguous target identities are not."
  (dolist (action '(pull keep-remote))
    (dolist (change '(renamed duplicate))
      (let* ((proof
              (jaunder-test--confirmed-batch
               action
               (lambda (event state)
                 (pcase (plist-get event :phase)
                   ('setup
                    (dolist (id '(1 100))
                      (puthash id (jaunder-test--batch-entry id "[[https://example.test/~alice/post-050][Target]]")
                               (plist-get state :entries))))
                   ('member
                    (when (and (eq change 'duplicate) (= (plist-get event :id) 100))
                      (with-temp-file (expand-file-name "duplicate.org" (plist-get state :root))
                        (insert "#+PROPERTY: JAUNDER_ID 50\n#+PROPERTY: JAUNDER_SLUG duplicate\n\nDuplicate target.\n")))))
                 nil)
               nil (when (eq change 'renamed) '(50))))
             (bytes (plist-get proof :bytes))
             (canonical "[[https://example.test/~alice/post-050][Target]]")
             (local "[[./post-050.org][Target]]"))
        (should (equal (mapcar #'jaunder-reconcile-result-outcome (plist-get proof :results))
                       '(success success success)))
        (should (string-match-p (regexp-quote (if (eq change 'renamed) canonical local)) (car bytes)))
        (should (string-match-p (regexp-quote (if (eq change 'renamed) local canonical)) (caddr bytes)))
        (should (= (plist-get proof :operation-pages) 4))
        (should (= (plist-get proof :pages) 8))))))

(provide 'jaunder-reconcile-batch-link-test)
;;; jaunder-reconcile-batch-link-test.el ends here
