;;; jaunder-reconcile-batch-race-test.el --- Batch replacement races -*- lexical-binding: t; -*-

;; Copyright (C) 2026 Jaunder contributors

;;; Commentary:
;; Inject concurrent changes at real HTTP/filesystem boundaries of confirmed
;; commands.  A complete remote Collection does not freeze local authority.

;;; Code:

(require 'ert)
(require 'cl-lib)
(load (expand-file-name "jaunder-reconcile-batch-fixture.el"
                        (file-name-directory (or load-file-name buffer-file-name))) nil t)

(ert-deftest jaunder-reconcile-batches-reject-duplicates-at-final-install-boundary ()
  "Both remote choices reject duplicate local identities after earlier checks."
  (dolist (action '(pull keep-remote))
    (dolist (boundary '(member media-installed))
      (let* ((injected nil)
             (proof
              (jaunder-test--confirmed-batch
               action
               (lambda (event state)
                 (when (and (not injected) (= (plist-get event :id) 1)
                            (eq (plist-get event :phase) boundary)
                            (or (eq boundary 'media-installed)
                                (= (plist-get event :read) (if (eq action 'pull) 2 3))))
                   (setq injected t)
                   (with-temp-file (expand-file-name "duplicate.org" (plist-get state :root))
                     (insert "#+PROPERTY: JAUNDER_ID 1\n#+PROPERTY: JAUNDER_SLUG duplicate\n\nConcurrent duplicate.\n")))
                 nil)))
             (results (plist-get proof :results)))
        (should injected)
        (should (equal (mapcar #'jaunder-reconcile-result-outcome results)
                       '(blocked success success)))
        (should (eq (jaunder-reconcile-result-reason (car results)) 'duplicate-local-id))
        (should (equal (car (plist-get proof :bytes)) (gethash 1 (plist-get proof :originals))))
        (should (= (plist-get proof :operation-pages) 4))
        (should (= (plist-get proof :pages) 8))))))

(ert-deftest jaunder-reconcile-batches-recheck-changes-between-selected-posts ()
  "A successful first row grants no permission to overwrite a changed next row."
  (dolist (action '(pull keep-remote))
    (dolist (change '(remote-deleted remote-etag remote-identity local-id local-path local-bytes buffer destination))
      (let* ((injected nil)
             (proof
              (jaunder-test--confirmed-batch
               action
               (lambda (event state)
                 (when (and (eq (plist-get event :phase) 'member)
                            (= (plist-get event :id) 50))
                   (if (memq change '(remote-deleted remote-etag remote-identity))
                       (progn
                         (setq injected t)
                         (pcase change
                           ('remote-deleted '(:status 404))
                           ('remote-etag '(:headers (("etag" . "\"changed\""))))
                           ('remote-identity (list :body (jaunder-test--batch-entry 99)))))
                     (unless injected
                       (setq injected t)
                       (let* ((row (cadr (plist-get state :rows)))
                              (path (jaunder-inventory-local-path (jaunder-reconcile-row-local row))))
                         (pcase change
                           ('local-id (with-temp-file path
                                        (insert "#+PROPERTY: JAUNDER_ID 99\n\nNew identity.\n")))
                           ('local-path (rename-file path (expand-file-name "moved.org" (plist-get state :root))))
                           ('local-bytes (write-region "Concurrent disk edit.\n" nil path t 'silent))
                           ('buffer (with-current-buffer (find-file-noselect path)
                                      (goto-char (point-max)) (insert "Unsaved local edit.\n")))
                           ('destination (with-temp-file (expand-file-name "post-050.org" (plist-get state :root))
                                           (insert "Another file owns the destination.\n")))))
                       nil))))
               nil (when (eq change 'destination) '(50))))
             (results (plist-get proof :results)))
        (should injected)
        (should (eq (jaunder-reconcile-result-outcome (car results)) 'success))
        (should (memq (jaunder-reconcile-result-outcome (cadr results)) '(blocked failed)))
        (should (eq (jaunder-reconcile-result-local-effect (cadr results)) 'unchanged))
        (should (eq (jaunder-reconcile-result-outcome (caddr results)) 'success))
        (when (memq change '(remote-deleted remote-etag remote-identity buffer destination))
          (should (equal (cadr (plist-get proof :bytes)) (gethash 50 (plist-get proof :originals)))))
        (should (= (plist-get proof :operation-pages) 4))
        (should (= (plist-get proof :pages) 8))))))

(provide 'jaunder-reconcile-batch-race-test)
;;; jaunder-reconcile-batch-race-test.el ends here
