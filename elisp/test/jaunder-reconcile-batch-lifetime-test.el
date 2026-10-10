;;; jaunder-reconcile-batch-lifetime-test.el --- Scoped Collection proof failures -*- lexical-binding: t; -*-

;; Copyright (C) 2026 Jaunder contributors

;;; Commentary:
;; Failed/empty operation evidence, cancellation, retry and nested roots exercise
;; complete confirmed commands rather than treating a cache value as permission.

;;; Code:

(require 'ert)
(load (expand-file-name "jaunder-reconcile-batch-fixture.el"
                        (file-name-directory (or load-file-name buffer-file-name))) nil t)

(ert-deftest jaunder-reconcile-batches-retain-failed-or-empty-collection-proof ()
  "Each dependent row fails closed without retrying partial or empty evidence."
  (dolist (action '(pull keep-remote))
    (dolist (fault '(transport duplicate cycle bad-page malformed empty))
      (let* ((proof
              (jaunder-test--confirmed-batch
               action
               (lambda (event _state)
                 (when (and (eq (plist-get event :phase) 'collection)
                            (not (plist-get event :refresh)))
                   (pcase fault
                     ('transport (error "injected Collection transport failure"))
                     ('duplicate
                      (list :body (concat "<feed xmlns=\"http://www.w3.org/2005/Atom\" xmlns:j=\"https://jaunder.org/ns/atompub\">"
                                          "<entry><link rel=\"edit\" href=\"https://example.test/atompub/alice/posts/1\"/><j:slug>one</j:slug></entry>"
                                          "<entry><link rel=\"edit\" href=\"https://example.test/atompub/alice/posts/1\"/><j:slug>one</j:slug></entry></feed>")))
                     ('cycle (when (= (plist-get event :page) 2)
                               '(:body "<feed xmlns=\"http://www.w3.org/2005/Atom\"><link rel=\"next\" href=\"https://example.test/page-2\"/></feed>")))
                     ('bad-page (when (= (plist-get event :page) 2) '(:status 503)))
                     ('malformed (when (= (plist-get event :page) 2) '(:body "<not-a-feed/>")))
                     ('empty '(:body "<feed xmlns=\"http://www.w3.org/2005/Atom\"/>")))))))
             (results (plist-get proof :results))
             (requests (if (memq fault '(cycle bad-page malformed)) 2 1)))
        (should (equal (mapcar #'jaunder-reconcile-result-outcome results) '(blocked blocked blocked)))
        (should (equal (plist-get proof :bytes)
                       (mapcar (lambda (id) (gethash id (plist-get proof :originals))) '(1 50 100))))
        (should (= (plist-get proof :operation-pages) requests))
        (should (= (plist-get proof :pages) (+ requests 4)))
        (when (eq fault 'transport)
          (dolist (result results)
            (should (string-match-p "injected Collection transport failure"
                                    (jaunder-reconcile-result-detail result)))))
        (when (eq fault 'duplicate)
          (should (equal (mapcar #'jaunder-reconcile-result-reason results)
                         '(duplicate-remote-id duplicate-remote-id duplicate-remote-id))))))))

(ert-deftest jaunder-reconcile-pull-collection-timeouts-retain-current-stage ()
  "Collection timeouts describe pending work, not the last completed page."
  (dolist (failed-page '(1 2))
    (let* ((proof
            (jaunder-test--confirmed-batch
             'pull
             (lambda (event _state)
               (when (and (eq (plist-get event :phase) 'collection)
                          (not (plist-get event :refresh))
                          (= (plist-get event :page) failed-page))
                 (error "Collection read timed out")))))
           (results (plist-get proof :results))
           (stage (if (= failed-page 1) "verifying fresh Collection"
                    "verifying Collection after page 1")))
      (should (equal (mapcar #'jaunder-reconcile-result-outcome results)
                     '(blocked blocked blocked)))
      (should (string-match-p
               (regexp-quote (concat stage ": Collection read timed out"))
               (jaunder-reconcile-result-detail (car results))))
      (should (= (plist-get proof :operation-pages) failed-page))
      (should (= (plist-get proof :pages) (+ failed-page 4)))
      (should (equal (plist-get proof :bytes)
                     (mapcar (lambda (id) (gethash id (plist-get proof :originals)))
                             '(1 50 100)))))))

(ert-deftest jaunder-reconcile-batches-cancel-between-posts-with-a-fresh-refresh ()
  "Cancellation preserves the installed first Post and leaves remaining Posts alone."
  (dolist (action '(pull keep-remote))
    (let* ((proof
            (jaunder-test--confirmed-batch
             action
             (lambda (event _state)
               (when (and (eq (plist-get event :phase) 'member)
                          (= (plist-get event :id) 1)
                          (= (plist-get event :read) (if (eq action 'pull) 2 3)))
                 (setq quit-flag t))
               nil)))
           (results (plist-get proof :results)))
      (should (eq (plist-get proof :status) 'cancelled))
      (should (equal (mapcar #'jaunder-reconcile-result-outcome results) '(success)))
      (should (equal (cdr (plist-get proof :bytes))
                     (mapcar (lambda (id) (gethash id (plist-get proof :originals))) '(50 100))))
      (should (= (plist-get proof :operation-pages) 4))
      (should (= (plist-get proof :pages) 8)))))

(ert-deftest jaunder-reconcile-batches-retain-results-when-final-refresh-fails ()
  "A refresh failure cannot reuse operation Members or erase committed outcomes."
  (dolist (action '(pull keep-remote))
    (let ((proof
           (jaunder-test--confirmed-batch
            action
            (lambda (event _state)
              (when (and (eq (plist-get event :phase) 'collection)
                         (plist-get event :refresh))
                '(:status 503))))))
      (should (eq (plist-get proof :status) 'refresh-failed))
      (should (equal (mapcar #'jaunder-reconcile-result-outcome (plist-get proof :results))
                     '(success success success)))
      (dolist (bytes (plist-get proof :bytes)) (should (string-match-p "Remote body" bytes)))
      (should (= (plist-get proof :operation-pages) 4))
      (should (= (plist-get proof :pages) 5)))))

(ert-deftest jaunder-reconcile-batches-retry-after-failed-operation-and-refresh ()
  "A later confirmed invocation acquires new proof in the same report/root."
  (dolist (action '(pull keep-remote))
    (let* ((failing t) later-status
           (proof
            (jaunder-test--confirmed-batch
             action
             (lambda (event _state)
               (pcase (plist-get event :phase)
                 ('collection (when failing '(:status 503)))
                 ('completed
                  (should (eq (plist-get event :status) 'refresh-failed))
                  (should (equal (mapcar #'jaunder-reconcile-result-outcome jaunder-reconcile-last-batch-results)
                                 '(blocked blocked blocked)))
                  (setq failing nil
                        later-status (funcall (if (eq action 'pull) #'jaunder-reconcile-pull-selected
                                                #'jaunder-reconcile-keep-remote-selected)))
                  nil))))))
      (should (eq later-status 'completed))
      (should (equal (mapcar #'jaunder-reconcile-result-outcome (plist-get proof :results))
                     '(success success success)))
      ;; One failed operation and refresh, followed by two complete four-page walks.
      (should (= (plist-get proof :pages) 10)))))

(ert-deftest jaunder-reconcile-nested-root-batches-do-not-share-remote-proof ()
  "A confirmed command for another root owns new evidence and leaves its caller intact."
  (let ((nested nil) inner)
    (let ((outer
           (jaunder-test--confirmed-batch
            'pull
            (lambda (event _state)
              (when (and (not nested) (eq (plist-get event :phase) 'member))
                (setq nested t
                      inner (jaunder-test--confirmed-batch 'keep-remote (lambda (&rest _) nil))))
              nil))))
      (dolist (proof (list outer inner))
        (should (eq (plist-get proof :status) 'completed))
        (should (equal (mapcar #'jaunder-reconcile-result-outcome (plist-get proof :results))
                       '(success success success)))
        (should (= (plist-get proof :operation-pages) 4))
        (should (= (plist-get proof :pages) 8))))))

(ert-deftest jaunder-reconcile-pull-captures-user-before-lazy-discovery ()
  "Changing the active User after staging cannot lend discovery to the operation."
  (let ((proof
         (jaunder-test--confirmed-batch
          'pull
          (lambda (event _state)
            (when (and (eq (plist-get event :phase) 'member)
                       (= (plist-get event :read) 1))
              (setq jaunder--active-blog
                    '(:base-url "https://example.test" :username "bob")))
            nil))))
    (should (= (plist-get proof :operation-pages) 0))
    (should (= (plist-get proof :pages) 4))
    (should (equal (mapcar #'jaunder-reconcile-result-outcome (plist-get proof :results))
                   '(blocked blocked blocked)))
    (dolist (result (plist-get proof :results))
      (should (string-match-p "scope mismatch" (jaunder-reconcile-result-detail result))))))

(ert-deftest jaunder-reconcile-read-batches-one-row-retain-targeted-checks ()
  "Batch size one shares discovery and final refresh without skipping Member reads."
  (dolist (action '(pull keep-remote))
    (let ((proof (jaunder-test--confirmed-batch action (lambda (&rest _) nil) '(1))))
      (should (eq (plist-get proof :status) 'completed))
      (should (= (plist-get proof :operation-pages) 4))
      (should (= (plist-get proof :pages) 8))
      (should (= (gethash 1 (plist-get proof :member-reads)) (if (eq action 'pull) 2 3)))
      (should (equal (mapcar #'jaunder-reconcile-result-outcome (plist-get proof :results))
                     '(success))))))

(ert-deftest jaunder-reconcile-delayed-read-install-renews-checkpoint-and-converges ()
  "A staging timestamp older than installed bytes cannot leave a local-ahead report."
  (dolist (action '(pull keep-remote))
    (let ((real-time (symbol-function 'current-time)) old-stage)
      (cl-letf (((symbol-function 'current-time)
                 (lambda ()
                   (let ((now (funcall real-time)))
                     (if old-stage (time-subtract now 30) now)))))
        (let ((proof
               (jaunder-test--confirmed-batch
                action
                (lambda (event _state)
                  (when (eq (plist-get event :phase) 'member)
                    (setq old-stage (= (plist-get event :read)
                                       (if (eq action 'pull) 1 2))))
                  (when (eq (plist-get event :phase) 'completed)
                    (should (eq (jaunder-reconcile-row-state
                                 (cl-find-if
                                  (lambda (row) (equal (jaunder--reconcile-row-post-id row) "1"))
                                  (jaunder-reconcile-report-rows jaunder-reconcile-report)))
                                'unchanged)))
                  nil)
                '(1))))
          (should (eq (plist-get proof :status) 'completed))
          (should (= (plist-get proof :pages) 8)))))))

(provide 'jaunder-reconcile-batch-lifetime-test)
;;; jaunder-reconcile-batch-lifetime-test.el ends here
