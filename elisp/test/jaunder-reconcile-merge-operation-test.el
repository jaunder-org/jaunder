;;; jaunder-reconcile-merge-operation-test.el --- Short-lived merge proof -*- lexical-binding: t; -*-

;; Copyright (C) 2026 Jaunder contributors

;;; Commentary:
;; Public preparation/completion own separate discovery, fresh target proof and
;; honest receipts.  Editing never holds operation evidence or mutation authority.

;;; Code:
(require 'ert)
(load (expand-file-name "jaunder-reconcile-merge-operation-fixture.el"
                        (file-name-directory (or load-file-name buffer-file-name))) nil t)

(ert-deftest jaunder-reconcile-merge-owns-separate-bounded-walks ()
  "Preparation ends before Ediff; finish reacquires current target href proof."
  (let ((proof
         (jaunder-test--merge-lifecycle
          (lambda (event state)
            (when (eq (plist-get event :phase) 'editing)
              (should-not (jaunder--operation-active-p))
              (puthash "25" '(:slug "post-025" :href "https://example.test/~alice/current-target" :etag "\"changed\"")
                       (plist-get state :members)))
            nil))))
    (should (= (gethash 'opening (plist-get proof :counts) 0) 4))
    (should (= (gethash 'prepare (plist-get proof :counts) 0) 4))
    (should (= (gethash 'finish (plist-get proof :counts) 0) 4))
    (should (= (gethash 'refresh (plist-get proof :counts) 0) 4))
    (should (= (gethash "1" (plist-get proof :reads) 0) 5))
    (should (= (length (plist-get proof :writes)) 1))
    (should (equal (plist-get (car (plist-get proof :writes)) :headers) '(("If-Match" . "\"old\""))))
    (should (string-match-p "https://example.test/~alice/current-target" (plist-get (car (plist-get proof :writes)) :xml)))
    (should (eq (plist-get (car (plist-get proof :results)) :outcome) 'success))
    (should-not (plist-get proof :retained))))

(ert-deftest jaunder-reconcile-merge-editing-drift-blocks-with-retained-scratch ()
  "Remote/local/target changes cannot borrow preparation proof at completion."
  (dolist (fault '(remote local duplicate target))
    (let ((proof
           (jaunder-test--merge-lifecycle
            (lambda (event state)
              (when (eq (plist-get event :phase) 'editing)
                (pcase fault
                  ('remote (puthash "1" '(:slug "post-001" :href "https://example.test/~alice/post-001" :etag "\"newer\"") (plist-get state :members)))
                  ('local (with-temp-file (plist-get state :path) (insert "New authored local text")))
                  ('duplicate (with-temp-file (expand-file-name "duplicate.org" (plist-get state :root))
                                (insert "#+PROPERTY: JAUNDER_ID 1\n#+PROPERTY: JAUNDER_SLUG duplicate\n\nDuplicate.")))
                  ('target (remhash "25" (plist-get state :members)))))
              nil))))
      (should (plist-get proof :retained))
      (should-not (plist-get proof :writes))
      (should (memq (plist-get (plist-get proof :last) :outcome) '(blocked failed)))
      (when (eq fault 'target)
        (should (= (gethash 'finish (plist-get proof :counts) 0) 4))
        (should (eq (plist-get (plist-get proof :last) :reason) 'merge-preparation-failed))))))

(ert-deftest jaunder-reconcile-merge-late-selected-drift-preserves-reviewed-authority ()
  "The final selected Member guard still blocks after preparation/link work."
  (let ((proof
         (jaunder-test--merge-lifecycle
          (lambda (event _state)
            (when (and (eq (plist-get event :phase) 'finish)
                       (eq (plist-get event :kind) 'member) (= (plist-get event :read) 5))
              '(:headers (("etag" . "\"newer\""))))))))
    (should (= (gethash 'finish (plist-get proof :counts) 0) 4))
    (should-not (plist-get proof :writes))
    (should (plist-get proof :retained))
    (should (eq (plist-get (plist-get proof :last) :reason) 'etag-stale))))

(ert-deftest jaunder-reconcile-merge-unknown-retry-has-fresh-proof-and-receipt ()
  "A 5xx is unknown, a later rejected attempt is failed, never an automatic retry."
  (let ((attempt 0) first second)
    (let ((proof
           (jaunder-test--merge-lifecycle
            (lambda (event state)
              (cond
               ((eq (plist-get event :phase) 'editing)
                (with-current-buffer (plist-get state :scratch)
                  (setq first (jaunder-reconcile-merge-finish))
                  (should (buffer-live-p (current-buffer)))
                  (setq second (jaunder-reconcile-merge-finish))))
               ((eq (plist-get event :kind) 'write)
                (setq attempt (1+ attempt))
                (pcase attempt
                  (1 '(:status 503 :no-commit t))
                  (2 '(:status 400 :no-commit t)))))))))
      (should (eq (plist-get first :outcome) 'unknown))
      (should (eq (plist-get second :outcome) 'failed))
      (should (eq (plist-get (car (plist-get proof :results)) :outcome) 'success))
      (should (= (length (plist-get proof :writes)) 3))
      (should (= (gethash 'finish (plist-get proof :counts) 0) 12))
      (should (= (gethash 'refresh (plist-get proof :counts) 0) 12))
      (dolist (write (plist-get proof :writes))
        (should (equal (plist-get write :headers) '(("If-Match" . "\"old\""))))))))

(ert-deftest jaunder-reconcile-merge-lost-response-retains-scratch-and-retries-freshly ()
  "A lost response is unknown; only another explicit completion attempts a PUT."
  (let ((attempt 0) first)
    (let ((proof
           (jaunder-test--merge-lifecycle
            (lambda (event state)
              (cond
               ((eq (plist-get event :phase) 'editing)
                (with-current-buffer (plist-get state :scratch)
                  (setq first (jaunder-reconcile-merge-finish))
                  (should (buffer-live-p (current-buffer)))))
               ((eq (plist-get event :kind) 'write)
                (setq attempt (1+ attempt))
                (when (= attempt 1)
                  (puthash "1" '(:slug "post-001" :href "https://example.test/~alice/post-001" :etag "\"old\"") (plist-get state :members))
                  (error "injected lost PUT response"))))))))
      (should (eq (plist-get first :outcome) 'unknown))
      (should (string-match-p "injected lost PUT response" (plist-get first :detail)))
      (should (= (length (plist-get proof :writes)) 2))
      (should (= (gethash 'finish (plist-get proof :counts) 0) 8))
      (should (eq (plist-get (car (plist-get proof :results)) :outcome) 'success)))))

(ert-deftest jaunder-reconcile-merge-partial-and-refresh-failure-retain-recovery ()
  "Confirmed invalid metadata is partial, not unknown; failed refresh keeps it."
  (let ((proof
         (jaunder-test--merge-lifecycle
          (lambda (event _state)
            (cond
             ((eq (plist-get event :kind) 'write)
              '(:headers (("etag" . "\"written\"") ("location" . "https://foreign.test/atompub/alice/posts/1"))))
             ((eq (plist-get event :phase) 'refresh) '(:status 503)))))))
    (should (eq (plist-get (plist-get proof :last) :outcome) 'partial))
    (should (eq (plist-get (plist-get proof :last) :reason) 'response-identity-invalid))
    (should (plist-get proof :retained))
    (should (equal (plist-get proof :bytes) (plist-get proof :original)))
    (should (eq (jaunder-reconcile-result-outcome (car (plist-get proof :terminal))) 'partial))
    (should (= (gethash 'refresh (plist-get proof :counts) 0) 1))))

(ert-deftest jaunder-reconcile-merge-local-completion-failure-and-fresh-retry ()
  "A real installed merge followed by checkpoint failure retains scratch."
  (let (first)
    (let ((proof
           (jaunder-test--merge-lifecycle
            (lambda (event state)
              (when (eq (plist-get event :phase) 'editing)
                (cl-letf (((symbol-function 'jaunder--write-back)
                           (lambda (&rest _) (error "injected checkpoint failure"))))
                  (with-current-buffer (plist-get state :scratch)
                    (setq first (jaunder-reconcile-merge-finish))))
                (should (buffer-live-p (plist-get state :scratch)))
                ;; Restore actual reviewed state before a separately authorized retry.
                (with-temp-file (plist-get state :path) (insert (plist-get state :original)))
                (puthash "1" '(:slug "post-001" :href "https://example.test/~alice/post-001" :etag "\"old\"")
                         (plist-get state :members)))
              nil))))
      (should (eq (plist-get first :outcome) 'partial))
      (should (string-match-p "injected checkpoint failure" (plist-get first :detail)))
      (should (= (length (plist-get proof :writes)) 2))
      (should (= (gethash 'finish (plist-get proof :counts) 0) 8))
      (should (eq (plist-get (car (plist-get proof :results)) :outcome) 'success)))))

(ert-deftest jaunder-reconcile-merge-local-completion-target-retry-is-fresh ()
  "Missing finish target fails visibly; a later attempt acquires its new href."
  (let (first)
    (let ((proof
           (jaunder-test--merge-lifecycle
            (lambda (event state)
              (when (eq (plist-get event :phase) 'editing)
                (remhash "25" (plist-get state :members))
                (with-current-buffer (plist-get state :scratch)
                  (setq first (jaunder-reconcile-merge-finish)))
                (should (buffer-live-p (plist-get state :scratch)))
                (puthash "25" '(:slug "post-025" :href "https://example.test/~alice/restored-target" :etag "\"restored\"")
                         (plist-get state :members)))
              nil))))
      (should (eq (plist-get first :reason) 'merge-preparation-failed))
      (should (= (gethash 'finish (plist-get proof :counts) 0) 8))
      (should (= (length (plist-get proof :writes)) 1))
      (should (string-match-p "restored-target" (plist-get (car (plist-get proof :writes)) :xml))))))

(ert-deftest jaunder-reconcile-merge-nested-operation-invalidates-without-lending-proof ()
  "A different-root same-User nested finish invalidates the parent's selected ID."
  (let (child started)
    (let ((proof
           (jaunder-test--merge-lifecycle
            (lambda (event state)
              (when (and (not started) (eq (plist-get event :phase) 'finish)
                         (eq (plist-get event :kind) 'member) (= (plist-get event :read) 4))
                (setq started t child (jaunder-test--merge-lifecycle (lambda (&rest _) nil)))
                (puthash "1" '(:slug "post-001" :href "https://example.test/~alice/post-001" :etag "\"written\"")
                         (plist-get state :members)))
              nil))))
      (should (eq (plist-get (car (plist-get child :results)) :outcome) 'success))
      (should (plist-get proof :retained))
      (should-not (plist-get proof :writes))
      (should (eq (plist-get (plist-get proof :last) :reason) 'etag-stale))
      (should (= (gethash "1" (plist-get proof :reads) 0) 6))
      (dolist (result (list child proof))
        (should (= (gethash 'prepare (plist-get result :counts) 0) 4))
        (should (= (gethash 'finish (plist-get result :counts) 0) 4))
        (should (= (gethash 'refresh (plist-get result :counts) 0) 4))))))

(ert-deftest jaunder-reconcile-merge-editing-drift-different-blog-does-not-reuse-preparation ()
  "A changed configured User cannot borrow the original preparation's Members."
  (let ((proof
         (jaunder-test--merge-lifecycle
          (lambda (event state)
            (when (eq (plist-get event :phase) 'editing)
              (setq jaunder-blogs (list (cons (plist-get state :root)
                                              '(:base-url "https://example.test" :username "bob")))))
            nil))))
    (should (plist-get proof :retained))
    (should-not (plist-get proof :writes))
    ;; The foreign namespace is rejected on the first fresh page, not borrowed.
    (should (= (gethash 'finish (plist-get proof :counts) 0) 1))
    (should (eq (plist-get (plist-get proof :last) :reason) 'fresh-inventory-failed))
    (should (eq (plist-get (plist-get proof :last) :outcome) 'blocked))))

(provide 'jaunder-reconcile-merge-operation-test)
;;; jaunder-reconcile-merge-operation-test.el ends here
