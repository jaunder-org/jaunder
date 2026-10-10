;;; jaunder-reconcile-write-test.el --- Operation write consumers -*- lexical-binding: t; -*-

;; Copyright (C) 2026 Jaunder contributors

;;; Commentary:
;; Public command budgets and honest send-phase outcomes.

;;; Code:

(require 'ert)
(load (expand-file-name "jaunder-reconcile-write-fixture.el"
                        (file-name-directory (or load-file-name buffer-file-name))) nil t)

(ert-deftest jaunder-reconcile-write-budgets-and-conditional-validators ()
  "One and three selected Posts share needed discovery, never unused discovery."
  (dolist (ids '((1) (1 50 100)))
    (dolist (case '((push nil 0) (push t 4) (keep-local nil 4) (keep-local t 4) (delete nil 0)))
      (pcase-let* ((`(,action ,links ,expected) case)
                   (proof (jaunder-test--confirmed-write-batch action (lambda (&rest _) nil) ids links)))
        (should (= (plist-get proof :initial-pages) 4))
        (should (= (plist-get proof :operation-pages) expected))
        (should (= (plist-get proof :pages) (+ expected 4)))
        (should (= (length (plist-get proof :writes)) (length ids)))
        (dolist (id ids)
          (should (= (gethash (number-to-string id) (plist-get proof :reads) 0)
                     (pcase action ('keep-local 2) ('delete 1) (_ 0)))))
        (should (cl-every (lambda (result) (eq (jaunder-reconcile-result-outcome result) 'success))
                          (plist-get proof :results)))
        (dolist (write (plist-get proof :writes))
          (should (equal (cdr (assoc "If-Match" (plist-get write :headers))) "\"old\"")))))))

(ert-deftest jaunder-reconcile-write-failures-retain-commit-or-uncertainty ()
  "Failed local completion is partial; a lost PUT/DELETE is unknown, without retry."
  (dolist (action '(push keep-local delete))
    (dolist (failure '(lost local))
      (let* ((real-write-back (symbol-function 'jaunder--write-back))
             (real-delete (symbol-function 'jaunder--reconcile-delete-local-file))
             (proof
              (cl-letf (((symbol-function 'jaunder--write-back)
                         (lambda (&rest args) (if (eq failure 'local) (error "checkpoint failed")
                                                (apply real-write-back args))))
                        ((symbol-function 'jaunder--reconcile-delete-local-file)
                         (lambda (row) (if (eq failure 'local) (error "local removal failed")
                                         (funcall real-delete row)))))
                (jaunder-test--confirmed-write-batch
                 action
                 (lambda (event _state)
                   (when (and (eq failure 'lost) (eq (plist-get event :phase) 'write))
                     (signal 'plz-error '("response lost")))
                   nil) '(1)))))
        (should (= (length (plist-get proof :writes)) 1))
        (should (eq (jaunder-reconcile-result-outcome (car (plist-get proof :results)))
                    (if (eq failure 'lost) 'unknown 'partial)))))))

(defun jaunder-test--write-source-links (state id target)
  "Replace ID's authored body in STATE with an exact TARGET file link."
  (let ((path (gethash id (plist-get state :paths))))
    (with-temp-buffer
      (insert-file-contents path)
      (goto-char (point-max))
      (insert (format "[[file:./%s.org][Target]]\n" target))
      (write-region (point-min) (point-max) path nil 'silent))))

(ert-deftest jaunder-reconcile-later-links-restore-only-affected-target-after-write ()
  "Missing alternate metadata and lost responses use targeted, current proof."
  (dolist (action '(push keep-local))
    (dolist (failure '(missing-alternate lost checkpoint))
      (let* ((real-write-back (symbol-function 'jaunder--write-back))
             (proof
              (cl-letf (((symbol-function 'jaunder--write-back)
                         (lambda (&rest args)
                           (if (and (eq failure 'checkpoint)
                                    (equal (jaunder--buffer-property "JAUNDER_ID") "1"))
                               (error "checkpoint failed after commit")
                             (apply real-write-back args)))))
                (jaunder-test--confirmed-write-batch
                 action
                 (lambda (event state)
                   (pcase (plist-get event :phase)
                     ('setup (jaunder-test--write-source-links state "50" "post-001"))
                     ('write
                      (when (equal (plist-get event :id) "1")
                        (puthash "1" '(:slug "post-001" :href "https://example.test/~alice/current-one" :etag "\"written\"")
                                 (plist-get state :members))
                        (if (eq failure 'lost) (signal 'plz-error '("lost accepted PUT"))
                          (let ((body (jaunder-test--write-entry "1" "post-001" "https://example.test/~alice/current-one")))
                            (list :body (if (eq failure 'missing-alternate)
                                            (replace-regexp-in-string "<link rel=\"alternate\"[^>]*/>" "" body)
                                          body))))))))
                 nil t))))
        (should (= (plist-get proof :operation-pages) 4))
        (should (= (plist-get proof :pages) 8))
        (should (= (length (plist-get proof :writes)) 3))
        (should (equal (mapcar #'jaunder-reconcile-result-outcome (plist-get proof :results))
                       (cons (pcase failure ('lost 'unknown) ('checkpoint 'partial) (_ 'success))
                             '(success success))))
        (should (string-match-p "https://example.test/~alice/current-one"
                                (plist-get (cadr (plist-get proof :writes)) :xml)))
        (should (= (gethash "1" (plist-get proof :reads) 0)
                   (+ (if (eq action 'keep-local) 2 0)
                      (if (eq failure 'checkpoint) 0 1))))))))

(ert-deftest jaunder-reconcile-created-target-restores-known-identity-not-guessed-url ()
  "A valid create identity with duplicate alternate links is restored by Member GET."
  (let ((proof
         (jaunder-test--confirmed-write-batch
          'push
          (lambda (event state)
            (pcase (plist-get event :phase)
              ('setup (jaunder-test--write-selected-as-creates state)
                      (jaunder-test--write-source-links state "50" "created"))
              ('write (when (equal (plist-get event :id) "101")
                        (list :body (jaunder-test--write-entry "101" "created"
                                                               "https://example.test/~alice/created" t))))))
          nil t)))
    (should (= (plist-get proof :operation-pages) 4))
    (should (= (plist-get proof :pages) 9))
    (should (= (gethash "101" (plist-get proof :reads) 0) 1))
    (should (equal (mapcar #'jaunder-reconcile-result-outcome (plist-get proof :results))
                   '(success success success)))
    (should (string-match-p "https://example.test/~alice/created"
                            (plist-get (cadr (plist-get proof :writes)) :xml)))))

(ert-deftest jaunder-reconcile-unresolved-create-lineage-survives-successful-rename ()
  "Bogus Location IDs never authorize links, even after rename or buffer failure."
  (dolist (location '("not-a-url/1" "https://foreign.test/atompub/alice/posts/1"
                      "https://example.test/atompub/alice/posts/1"))
    (dolist (after-rename-failure '(nil t))
      (let* ((real-set-name (symbol-function 'set-visited-file-name))
             (proof
              (cl-letf (((symbol-function 'set-visited-file-name)
                         (lambda (path &rest args)
                           (if (and after-rename-failure (string-suffix-p "/created.org" path))
                               (error "post-rename buffer completion failed")
                             (apply real-set-name path args)))))
                (jaunder-test--confirmed-write-batch
                 'push
                 (lambda (event state)
                   (pcase (plist-get event :phase)
                     ('setup (jaunder-test--write-selected-as-creates state)
                             (jaunder-test--write-source-links state "50" "created")
                             (puthash "1" '(:slug "created" :href "https://example.test/~alice/existing" :etag "\"old\"")
                                      (plist-get state :members))
                             nil)
                     ('write
                      (when (equal (plist-get event :id) "101")
                        (puthash "101" '(:slug "actual-created" :href "https://example.test/~alice/actual-created" :etag "\"written\"")
                                 (plist-get state :members))
                        (list :body (jaunder-test--write-entry "101" "created" "https://example.test/~alice/actual-created")
                              :headers (list (cons "etag" "\"written\"")
                                             (cons "location" location)))))))
                 nil t))))
        (should (= (length (plist-get proof :writes)) 2))
        (should (equal (mapcar #'jaunder-reconcile-result-outcome (plist-get proof :results))
                       '(partial failed success)))
        (should (string-match-p "create identity is unresolved"
                                (jaunder-reconcile-result-detail (cadr (plist-get proof :results)))))
        (should (= (gethash "1" (plist-get proof :reads) 0) 0))
        (should (assoc "created.org" (plist-get proof :files)))
        (should-not (jaunder-reconcile-result-post-id (car (plist-get proof :results))))))))

(ert-deftest jaunder-reconcile-create-attempts-preserve-uncertainty-and-safe-replay ()
  "A final rejection cannot erase an earlier lost create; valid keyed replay can resolve it."
  (dolist (rejected '(nil t))
    (let* ((attempts 0)
           (proof
            (jaunder-test--confirmed-write-batch
             'push
             (lambda (event state)
               (pcase (plist-get event :phase)
                 ('setup (jaunder-test--write-selected-as-creates state))
                 ('write
                  (when (equal (plist-get event :method) "POST")
                    (setq attempts (1+ attempts))
                    (if (= attempts 1)
                        (signal 'plz-error '("first create response lost"))
                      (if rejected '(:status 409 :no-commit t) '(:status 200)))))))
             '(1))))
      (should (= attempts 2))
      (let ((writes (plist-get proof :writes)))
        (should (equal (cdr (assoc "Idempotency-Key" (plist-get (car writes) :headers)))
                       (cdr (assoc "Idempotency-Key" (plist-get (cadr writes) :headers))))))
      (should (eq (jaunder-reconcile-result-outcome (car (plist-get proof :results)))
                  (if rejected 'unknown 'success)))
      (when rejected
        (should (string-match-p "first create response lost"
                                (jaunder-reconcile-result-detail (car (plist-get proof :results)))))
        (should (string-match-p "JAUNDER_CREATE_KEY" (cdr (assoc "post-001.org" (plist-get proof :files)))))))))

(ert-deftest jaunder-reconcile-rejected-conditional-writes-preserve-selected-local-source ()
  "Literal stale preconditions reject writes without replacing reviewed authority."
  (dolist (action '(push keep-local delete))
    (let ((proof (jaunder-test--confirmed-write-batch
                  action (lambda (event _state)
                           (when (eq (plist-get event :phase) 'write) '(:status 412 :no-commit t))) '(1))))
      (should (= (length (plist-get proof :writes)) 1))
      (should (memq (jaunder-reconcile-result-outcome (car (plist-get proof :results))) '(blocked failed)))
      (should (equal (cdr (assoc "post-001.org" (plist-get proof :files)))
                     (gethash "1" (plist-get proof :originals)))))))

(ert-deftest jaunder-reconcile-write-target-proof-enforces-namespace-and-singletons ()
  "Invalid write metadata never lends a first-field Member or alternate to later rows."
  (dolist (fault '(edit-duplicate slug-duplicate edit-spoof alternate-spoof alternate-duplicate foreign transport malformed))
    (let* ((body (jaunder-test--write-entry "1" "post-001" "https://example.test/~alice/current-one"))
           (invalid
            (pcase fault
              ('edit-duplicate (replace-regexp-in-string "</entry>" "<link rel=\"edit\" href=\"https://example.test/atompub/alice/posts/1\"/></entry>" body t t))
              ('slug-duplicate (replace-regexp-in-string "</entry>" "<j:slug>post-001</j:slug></entry>" body t t))
              ('edit-spoof (replace-regexp-in-string "rel=\"edit\"" "xmlns=\"urn:fake\" rel=\"edit\"" body t t))
              ('alternate-spoof (replace-regexp-in-string "rel=\"alternate\"" "xmlns=\"urn:fake\" rel=\"alternate\"" body t t))
              ('alternate-duplicate (jaunder-test--write-entry "1" "post-001" "https://example.test/~alice/current-one" t))
              ('foreign (replace-regexp-in-string "https://example.test/atompub" "https://foreign.test/atompub" body t t))
              ('malformed "<entry xmlns=\"http://www.w3.org/2005/Atom\">")
              (_ nil)))
           (proof
            (jaunder-test--confirmed-write-batch
             'push
             (lambda (event state)
               (pcase (plist-get event :phase)
                 ('setup (dolist (id '("50" "100")) (jaunder-test--write-source-links state id "post-001")))
                 ('write (when (equal (plist-get event :id) "1")
                           (if (eq fault 'transport)
                               (list :body (replace-regexp-in-string "<link rel=\"alternate\"[^>]*/>" "" body))
                             (list :body invalid))))
                 ('member (when (equal (plist-get event :id) "1")
                            (if (eq fault 'transport) (signal 'file-error '("target read offline"))
                              (list :body invalid))))))
             nil t)))
      (should (= (plist-get proof :operation-pages) 4))
      (should (= (gethash "1" (plist-get proof :reads) 0) 1))
      (should (= (length (plist-get proof :writes)) 1))
      (should (equal (mapcar #'jaunder-reconcile-result-outcome (plist-get proof :results))
                     (list (if (memq fault '(edit-duplicate slug-duplicate foreign malformed)) 'partial 'success)
                           'failed 'failed)))
      (when (eq fault 'transport)
        (dolist (result (cdr (plist-get proof :results)))
          (should (string-match-p "target read offline" (jaunder-reconcile-result-detail result))))))))

(ert-deftest jaunder-reconcile-write-renames-do-not-repair-authored-target-paths ()
  "A new exact path is usable after rename; an authored obsolete path remains invalid."
  (dolist (target '("new-one" "post-001"))
    (let ((proof
           (jaunder-test--confirmed-write-batch
            'push
            (lambda (event state)
              (pcase (plist-get event :phase)
                ('setup (jaunder-test--write-source-links state "50" target))
                ('write
                 (when (equal (plist-get event :id) "1")
                   (puthash "1" '(:slug "new-one" :href "https://example.test/~alice/new-one" :etag "\"written\"")
                            (plist-get state :members))
                   (list :body (jaunder-test--write-entry "1" "new-one" "https://example.test/~alice/new-one"))))))
            nil t)))
      (should (= (plist-get proof :operation-pages) 4))
      (should (assoc "new-one.org" (plist-get proof :files)))
      (should (equal (mapcar #'jaunder-reconcile-result-outcome (plist-get proof :results))
                     (list 'success (if (equal target "new-one") 'success 'failed) 'success)))
      (when (equal target "new-one")
        (should (string-match-p "https://example.test/~alice/new-one"
                                (plist-get (cadr (plist-get proof :writes)) :xml)))))))

(ert-deftest jaunder-reconcile-nested-delete-invalidates-parent-link-proof ()
  "Confirmed or lost nested DELETE invalidates cached parent targets before later links."
  (dolist (lost '(nil t))
    (let (nested child)
      (let ((proof
             (jaunder-test--confirmed-write-batch
              'push
              (lambda (event state)
                (when (and (not nested) (eq (plist-get event :phase) 'write))
                  (setq nested t
                        child
                        (jaunder-test--confirmed-write-batch
                         'delete
                         (lambda (child-event _child-state)
                           (when (eq (plist-get child-event :phase) 'write)
                             (remhash "25" (plist-get state :members))
                             (when lost (signal 'plz-error '("nested DELETE response lost"))))
                           nil)
                         '(25))))
                nil)
              nil t)))
        (should (= (plist-get proof :operation-pages) 4))
        (should (= (plist-get child :operation-pages) 0))
        (should (= (length (plist-get child :writes)) 1))
        (should (eq (jaunder-reconcile-result-outcome (car (plist-get child :results)))
                    (if lost 'unknown 'success)))
        (should (equal (mapcar #'jaunder-reconcile-result-outcome (plist-get proof :results))
                       '(success failed failed)))
        (should (= (gethash "25" (plist-get proof :reads) 0) (if lost 1 0)))))))

(ert-deftest jaunder-reconcile-confirmed-write-keeps-receipt-on-late-user-drift ()
  "A scope change during HTTP cannot erase a commit or permit local completion."
  (let ((proof
         (jaunder-test--confirmed-write-batch
          'push
          (lambda (event _state)
            (when (eq (plist-get event :phase) 'write)
              (setq jaunder--active-blog '(:base-url "https://example.test" :username "bob")))
            nil)
          '(1))))
    (should (= (length (plist-get proof :writes)) 1))
    (should (eq (jaunder-reconcile-result-outcome (car (plist-get proof :results))) 'partial))
    (should (string-match-p "scope mismatch" (jaunder-reconcile-result-detail (car (plist-get proof :results)))))
    (should (equal (cdr (assoc "post-001.org" (plist-get proof :files)))
                   (gethash "1" (plist-get proof :originals))))))

(ert-deftest jaunder-reconcile-unresolved-lineage-does-not-taint-unrelated-path-replacement ()
  "Replacing a creating file's inode does not transfer its unresolved provenance."
  (let* ((real-set-name (symbol-function 'set-visited-file-name))
         (proof
          (cl-letf (((symbol-function 'set-visited-file-name)
                     (lambda (path &rest args)
                       (prog1 (apply real-set-name path args)
                         (when (string-suffix-p "/created.org" path)
                           (let ((replacement (concat path ".replacement")))
                             (with-temp-file replacement
                               (insert "#+TITLE: Replacement\n#+PROPERTY: JAUNDER_ID 1\n#+PROPERTY: JAUNDER_SLUG created\n#+PROPERTY: JAUNDER_SYNCED \"old\"\n#+PROPERTY: JAUNDER_DATE_TZ UTC\n#+PROPERTY: JAUNDER_STATUS draft\n\nIndependent existing Post.\n"))
                             (rename-file replacement path t)))))))
            (jaunder-test--confirmed-write-batch
             'push
             (lambda (event state)
               (pcase (plist-get event :phase)
                 ('setup (jaunder-test--write-selected-as-creates state)
                         (jaunder-test--write-source-links state "50" "created")
                         (puthash "1" '(:slug "created" :href "https://example.test/~alice/existing" :etag "\"old\"")
                                  (plist-get state :members))
                         nil)
                 ('write (when (equal (plist-get event :id) "101")
                           (list :headers '(("etag" . "\"written\"")
                                            ("location" . "https://foreign.test/atompub/alice/posts/1")))))))
             nil t))))
    (should (= (length (plist-get proof :writes)) 3))
    (should (equal (mapcar #'jaunder-reconcile-result-outcome (plist-get proof :results))
                   '(partial success success)))
    (should (string-match-p "https://example.test/~alice/existing"
                            (plist-get (cadr (plist-get proof :writes)) :xml)))))

(ert-deftest jaunder-reconcile-quit-during-write-retains-unknown-before-cancelling ()
  "Cancellation during a possible send retains an unknown terminal row, not rollback."
  (dolist (action '(push keep-local delete))
    (let ((proof
           (jaunder-test--confirmed-write-batch
            action (lambda (event _state)
                     (when (eq (plist-get event :phase) 'write) (signal 'quit nil)))
            nil nil)))
      (should (= (length (plist-get proof :writes)) 1))
      (should (= (length (plist-get proof :results)) 1))
      (should (eq (jaunder-reconcile-result-outcome (car (plist-get proof :results))) 'unknown))
      (should (eq (plist-get proof :status) 'cancelled))
      (should (equal (cdr (assoc "post-001.org" (plist-get proof :files)))
                     (gethash "1" (plist-get proof :originals)))))))

(ert-deftest jaunder-reconcile-put-identity-contradiction-protects-checkpoint-not-authority ()
  "Supplied contradiction is partial; omission is allowed and link proof restores by known ID."
  (dolist (action '(push keep-local))
    (dolist (fault '(location-off-origin location-conflicting edit-conflicting slug-duplicate omitted))
      (let* ((omitted-keep-local (and (eq action 'keep-local) (eq fault 'omitted)))
             (body (jaunder-test--write-entry "1" "post-001" "https://example.test/~alice/current-one"))
             (proof
              (jaunder-test--confirmed-write-batch
               action
               (lambda (event state)
                 (pcase (plist-get event :phase)
                   ('setup (jaunder-test--write-source-links state "50" "post-001"))
                   ('write
                    (when (equal (plist-get event :id) "1")
                      (puthash "1" '(:slug "post-001" :href "https://example.test/~alice/current-one" :etag "\"written\"")
                               (plist-get state :members))
                      (pcase fault
                        ('location-off-origin '(:headers (("etag" . "\"written\"") ("location" . "https://foreign.test/atompub/alice/posts/1"))))
                        ('location-conflicting '(:headers (("etag" . "\"written\"") ("location" . "https://example.test/atompub/alice/posts/2"))))
                        ('edit-conflicting (list :body (replace-regexp-in-string "posts/1" "posts/2" body t t)))
                        ('slug-duplicate (list :body (replace-regexp-in-string "</entry>" "<j:slug>other</j:slug></entry>" body t t)))
                        ('omitted '(:body "<entry xmlns=\"http://www.w3.org/2005/Atom\"/>")))))))
               nil t)))
        (ert-info ((format "action=%S fault=%S details=%S" action fault
                           (mapcar #'jaunder-reconcile-result-detail (plist-get proof :results))))
          (should (equal (mapcar #'jaunder-reconcile-result-outcome (plist-get proof :results))
                         (list (if (eq fault 'omitted) 'success 'partial)
                               ;; Legacy keep-local renames a missing-slug checkpoint to nil.org.
                               ;; Do not widen this guard to change omission semantics; the old link fails.
                               (if omitted-keep-local 'failed 'success) 'success))))
        (should (equal (jaunder-reconcile-result-post-id (car (plist-get proof :results))) "1"))
        (should (= (gethash "1" (plist-get proof :reads) 0)
                   (+ (if omitted-keep-local 0 1) (if (eq action 'keep-local) 2 0))))
        (should (= (gethash "2" (plist-get proof :reads) 0) 0))
        (should (= (plist-get proof :operation-pages) 4))
        (unless omitted-keep-local
          (should (string-match-p "https://example.test/~alice/current-one"
                                  (plist-get (cadr (plist-get proof :writes)) :xml)))
          (should (equal (cdr (assoc "If-Match" (plist-get (cadr (plist-get proof :writes)) :headers))) "\"old\"")))
        (unless (eq fault 'omitted)
          (should (equal (cdr (assoc "post-001.org" (plist-get proof :files)))
                         (gethash "1" (plist-get proof :originals)))))))))

(ert-deftest jaunder-reconcile-http-server-errors-retain-unknown-without-retrying ()
  "A 5xx can follow commitment; non-keyed writes remain unknown and are not retried."
  (dolist (action '(push keep-local delete))
    (let ((proof
           (jaunder-test--confirmed-write-batch
            action (lambda (event _state)
                     (when (eq (plist-get event :phase) 'write) '(:status 503))) '(1))))
      (should (= (length (plist-get proof :writes)) 1))
      (should (eq (jaunder-reconcile-result-outcome (car (plist-get proof :results))) 'unknown))
      (should (eq (jaunder-reconcile-result-reason (car (plist-get proof :results))) 'remote-outcome-unknown))
      (should (equal (cdr (assoc "post-001.org" (plist-get proof :files)))
                     (gethash "1" (plist-get proof :originals)))))))

(ert-deftest jaunder-reconcile-rejected-write-invalidates-later-target-proof ()
  "Rejected writes do not lend stale hrefs, membership or selected validators."
  (dolist (action '(push keep-local))
    (dolist (drift '(absent renamed href))
      (let* ((proof
              (jaunder-test--confirmed-write-batch
               action
               (lambda (event state)
                 (pcase (plist-get event :phase)
                   ('setup (jaunder-test--write-source-links state "50" "post-001"))
                   ('write
                    (when (equal (plist-get event :id) "1")
                      (if (eq drift 'absent) (remhash "1" (plist-get state :members))
                        (puthash "1" (list :slug (if (eq drift 'renamed) "external-rename" "post-001")
                                           :href "https://example.test/~alice/external" :etag "\"external\"")
                                 (plist-get state :members)))
                      (list :status (if (eq drift 'absent) 404 412))))))
               '(1 50) t))
             (writes (plist-get proof :writes)))
        (ert-info ((format "action=%S drift=%S" action drift))
          (should (= (length writes) (if (eq drift 'href) 2 1)))
          (should (equal (mapcar #'jaunder-reconcile-result-outcome (plist-get proof :results))
                         (list (if (and (eq action 'keep-local) (not (eq drift 'absent))) 'blocked 'failed)
                               (if (eq drift 'href) 'success 'failed))))
          (should (= (plist-get proof :operation-pages) 4))
          (should (= (plist-get proof :pages) 8))
          (should (= (gethash "1" (plist-get proof :reads) 0)
                     (+ (if (eq action 'keep-local) 2 0) (if (eq drift 'absent) 0 1))))
          (dolist (write writes)
            (should (equal (cdr (assoc "If-Match" (plist-get write :headers))) "\"old\"")))
          (when (eq drift 'href)
            (should (string-match-p "https://example.test/~alice/external" (plist-get (cadr writes) :xml))))
          (should (equal (cdr (assoc "post-001.org" (plist-get proof :files)))
                         (gethash "1" (plist-get proof :originals)))))))))

(ert-deftest jaunder-reconcile-rejected-outer-write-preserves-newer-nested-absence ()
  "An outer rejection must not overwrite a newer same-User child's absence proof."
  (dolist (status '(404 409 412))
    (let (nested child)
      (let ((proof
             (jaunder-test--confirmed-write-batch
              'push
              (lambda (event state)
                (pcase (plist-get event :phase)
                  ('setup (jaunder-test--write-source-links state "50" "post-001"))
                  ('write
                   (when (and (not nested) (equal (plist-get event :id) "1"))
                     (setq nested t
                           child
                           (jaunder-test--confirmed-write-batch
                            'delete
                            (lambda (child-event _child-state)
                              (when (eq (plist-get child-event :phase) 'write)
                                (remhash "1" (plist-get state :members)))
                              nil)
                            '(1)))
                     (list :status status)))))
              '(1 50) t)))
        (ert-info ((format "outer status=%S" status))
          (should (= (length (plist-get proof :writes)) 1))
          (should (equal (mapcar #'jaunder-reconcile-result-outcome (plist-get proof :results))
                         '(failed failed)))
          (should (= (gethash "1" (plist-get proof :reads) 0) 0))
          (should (= (plist-get proof :operation-pages) 4))
          (should (= (plist-get child :operation-pages) 0))
          (should (eq (jaunder-reconcile-result-outcome (car (plist-get child :results))) 'success)))))))

(ert-deftest jaunder-reconcile-unresolved-create-survives-owned-atomic-save-hook-error ()
  "Owned inode replacement precedes hook failure and cannot erase create provenance."
  (dolist (failure-at '(1 2))
    (dolist (drift '(nil t))
      (let* ((saves 0) before-saves after-saves
             (proof
              (jaunder-test--confirmed-write-batch
               'push
               (lambda (event state)
                 (pcase (plist-get event :phase)
                   ('setup (jaunder-test--write-selected-as-creates state)
                           (jaunder-test--write-source-links state "50" "post-001"))
                   ('write
                    (when (equal (plist-get event :id) "101")
                      (setq-local file-precious-flag t)
                      (setq-local before-save-hook
                                  (list (lambda ()
                                          (push (jaunder--operation-file-identity (buffer-file-name)) before-saves)
                                          (when drift
                                            (setq jaunder--active-blog '(:base-url "https://example.test" :username "bob"))))))
                      (setq-local after-save-hook
                                  (list (lambda ()
                                          (setq saves (1+ saves))
                                          (push (jaunder--operation-file-identity (buffer-file-name)) after-saves)
                                          (when (= saves failure-at) (error "failure after owned atomic save")))))
                      (list :body (jaunder-test--write-entry "101" "post-001" "https://example.test/~alice/actual-created")
                            :headers '(("etag" . "\"written\"")
                                       ("location" . "https://example.test/atompub/alice/posts/1")))))))
               nil t)))
        (ert-info ((format "save=%S drift=%S" failure-at drift))
          (should (= (length (plist-get proof :writes)) 2))
          (should (= saves failure-at))
          (should (= (length before-saves) failure-at))
          (should (= (length after-saves) failure-at))
          ;; An inode freed by the first replacement may be reused by the next.
          ;; Each owned save must replace its current inode, not every past inode.
          (cl-mapc (lambda (before after) (should-not (equal before after)))
                   (reverse before-saves) (reverse after-saves))
          (should (equal (mapcar #'jaunder-reconcile-result-outcome (plist-get proof :results))
                         '(partial failed success)))
          (should (string-match-p "failure after owned atomic save"
                                  (jaunder-reconcile-result-detail (car (plist-get proof :results)))))
          (should (string-match-p "create identity is unresolved"
                                  (jaunder-reconcile-result-detail (cadr (plist-get proof :results)))))
          (should (= (gethash "1" (plist-get proof :reads) 0) 0))
          (should (= (plist-get proof :operation-pages) 4)))))))

(ert-deftest jaunder-reconcile-save-hook-error-does-not-adopt-unrelated-replacement ()
  "A later hook's external replacement does not inherit the owned save's lineage."
  (let* ((proof
          (jaunder-test--confirmed-write-batch
           'push
           (lambda (event state)
             (pcase (plist-get event :phase)
               ('setup (jaunder-test--write-selected-as-creates state)
                       (jaunder-test--write-source-links state "50" "post-001"))
               ('write
                (when (equal (plist-get event :id) "101")
                  (setq-local file-precious-flag t)
                  (setq-local after-save-hook
                              (list (lambda ()
                                      (let* ((path (buffer-file-name)) (replacement (concat path ".replacement")))
                                        (with-temp-file replacement
                                          (insert "#+TITLE: Independent replacement\n#+PROPERTY: JAUNDER_ID 1\n#+PROPERTY: JAUNDER_SLUG post-001\n#+PROPERTY: JAUNDER_SYNCED \"old\"\n#+PROPERTY: JAUNDER_DATE_TZ UTC\n#+PROPERTY: JAUNDER_STATUS draft\n\nIndependent existing Post.\n"))
                                        (rename-file replacement path t))
                                      (error "failure after unrelated replacement"))))
                  (list :body (jaunder-test--write-entry "101" "post-001" "https://example.test/~alice/actual-created")
                        :headers '(("etag" . "\"written\"")
                                   ("location" . "https://example.test/atompub/alice/posts/1")))))))
           nil t)))
    (should (= (length (plist-get proof :writes)) 3))
    (should (equal (mapcar #'jaunder-reconcile-result-outcome (plist-get proof :results))
                   '(partial success success)))
    (should (string-match-p "failure after unrelated replacement"
                            (jaunder-reconcile-result-detail (car (plist-get proof :results)))))
    (should (string-match-p "https://example.test/~alice/post-001"
                            (plist-get (cadr (plist-get proof :writes)) :xml)))))

(ert-deftest jaunder-reconcile-rejected-nested-delete-keeps-parent-target-dirty ()
  "A child's stale DELETE and a rejected outer PUT require fresh parent link proof."
  (let (nested child)
    (let ((proof
           (jaunder-test--confirmed-write-batch
            'push
            (lambda (event state)
              (pcase (plist-get event :phase)
                ('setup (jaunder-test--write-source-links state "50" "post-001"))
                ('write
                 (when (and (not nested) (equal (plist-get event :id) "1"))
                   (setq nested t
                         child
                         (jaunder-test--confirmed-write-batch
                          'delete
                          (lambda (child-event child-state)
                            (when (eq (plist-get child-event :phase) 'write)
                              (dolist (members (list (plist-get state :members) (plist-get child-state :members)))
                                (puthash "1" '(:slug "post-001" :href "https://example.test/~alice/external" :etag "\"external\"") members))
                              '(:status 412)))
                          '(1)))
                   '(:status 409)))))
            '(1 50) t)))
      (should (equal (mapcar #'jaunder-reconcile-result-outcome (plist-get proof :results)) '(failed success)))
      (should (= (length (plist-get proof :writes)) 2))
      (should (= (length (plist-get child :writes)) 1))
      (should (eq (jaunder-reconcile-result-outcome (car (plist-get child :results))) 'failed))
      (should (= (gethash "1" (plist-get proof :reads) 0) 1))
      (should (= (plist-get proof :operation-pages) 4))
      (should (string-match-p "https://example.test/~alice/external"
                              (plist-get (cadr (plist-get proof :writes)) :xml)))
      (dolist (write (append (plist-get proof :writes) (plist-get child :writes)))
        (should (equal (cdr (assoc "If-Match" (plist-get write :headers))) "\"old\""))))))

(ert-deftest jaunder-reconcile-confirmed-outer-put-revalidates-nested-deletion ()
  "An older confirmed response cannot lend Member proof over a child's deletion."
  (let (child)
    (let ((proof
           (jaunder-test--confirmed-write-batch
            'push
            (lambda (event state)
              (pcase (plist-get event :phase)
                ('setup (jaunder-test--write-source-links state "50" "post-001"))
                ('write
                 (when (equal (plist-get event :id) "1")
                   (setq child
                         (jaunder-test--confirmed-write-batch
                          'delete
                          (lambda (nested nested-state)
                            (when (eq (plist-get nested :phase) 'setup)
                              (puthash "1" (gethash "1" (plist-get state :members))
                                       (plist-get nested-state :members)))
                            nil)
                          '(1)))
                   (remhash "1" (plist-get state :members))
                   nil))))
            '(1 50) t)))
      (should (= (length (plist-get proof :writes)) 1))
      (should (equal (mapcar #'jaunder-reconcile-result-outcome (plist-get proof :results)) '(success failed)))
      (should (eq (jaunder-reconcile-result-outcome (car (plist-get child :results))) 'success))
      (should (equal (cdr (assoc "If-Match" (plist-get (car (plist-get child :writes)) :headers))) "\"written\""))
      (should (equal (cdr (assoc "If-Match" (plist-get (car (plist-get proof :writes)) :headers))) "\"old\""))
      (should (= (gethash "1" (plist-get proof :reads) 0) 1))
      (should (= (plist-get proof :operation-pages) 4))
      (should (= (plist-get proof :pages) 8))
      (should (= (plist-get child :operation-pages) 0)))))

(ert-deftest jaunder-reconcile-confirmed-outer-put-revalidates-nested-href-transition ()
  "Current target proof is restored, not borrowed from child or older outer response."
  (let (child)
    (let ((proof
           (jaunder-test--confirmed-write-batch
            'push
            (lambda (event state)
              (pcase (plist-get event :phase)
                ('setup (jaunder-test--write-source-links state "50" "post-001"))
                ('write
                 (when (equal (plist-get event :id) "1")
                   (setq child
                         (jaunder-test--confirmed-write-batch
                          'keep-local
                          (lambda (nested nested-state)
                            (pcase (plist-get nested :phase)
                              ('setup
                               (puthash "1" (gethash "1" (plist-get state :members))
                                        (plist-get nested-state :members))
                               (setf (jaunder-reconcile-row-remote-etag (car (plist-get nested-state :rows))) "\"written\"")
                               nil)
                              ('write
                               (puthash "1" '(:slug "post-001" :href "https://example.test/~alice/child-current" :etag "\"child\"")
                                        (plist-get nested-state :members))
                               (list :body (jaunder-test--write-entry "1" "post-001" "https://example.test/~alice/child-current")
                                     :headers '(("etag" . "\"child\"")
                                                ("location" . "https://example.test/atompub/alice/posts/1"))))))
                          '(1)))
                   (puthash "1" '(:slug "post-001" :href "https://example.test/~alice/child-current" :etag "\"child\"")
                            (plist-get state :members))
                   nil))))
            '(1 50) t)))
      (should (= (gethash "1" (plist-get proof :reads) 0) 1))
      (should (= (length (plist-get proof :writes)) 2))
      (should (equal (mapcar #'jaunder-reconcile-result-outcome (plist-get proof :results)) '(success success)))
      (should (eq (jaunder-reconcile-result-outcome (car (plist-get child :results))) 'success))
      (should (= (length (plist-get child :writes)) 1))
      (should (equal (cdr (assoc "If-Match" (plist-get (car (plist-get child :writes)) :headers))) "\"written\""))
      (dolist (write (plist-get proof :writes))
        (should (equal (cdr (assoc "If-Match" (plist-get write :headers))) "\"old\"")))
      (should (string-match-p "https://example.test/~alice/child-current"
                              (plist-get (cadr (plist-get proof :writes)) :xml)))
      (should (= (plist-get proof :operation-pages) 4))
      (should (= (plist-get proof :pages) 8)))))

(ert-deftest jaunder-reconcile-unresolved-create-rename-retains-captured-owner ()
  "Successful save-hook context drift cannot erase owned rename provenance."
  (let ((proof
         (jaunder-test--confirmed-write-batch
          'push
          (lambda (event state)
            (pcase (plist-get event :phase)
              ('setup
               (jaunder-test--write-selected-as-creates state)
               (jaunder-test--write-source-links state "50" "created")
               (puthash "1" '(:slug "created" :href "https://example.test/~alice/existing" :etag "\"old\"")
                        (plist-get state :members))
               nil)
              ('write
               (when (equal (plist-get event :id) "101")
                 (setq-local file-precious-flag t)
                 (setq-local after-save-hook
                             (list (lambda ()
                                     (setq jaunder--active-blog
                                           '(:base-url "https://example.test" :username "bob")))))
                 (list :body (jaunder-test--write-entry "101" "created"
                                                        "https://example.test/~alice/actual-created")
                       :headers '(("etag" . "\"written\"")
                                  ("location" . "https://foreign.test/atompub/alice/posts/1")))))))
          nil t)))
    (should (assoc "created.org" (plist-get proof :files)))
    (should-not (assoc "post-001.org" (plist-get proof :files)))
    (should (= (length (plist-get proof :writes)) 2))
    (should (equal (mapcar #'jaunder-reconcile-result-outcome (plist-get proof :results))
                   '(partial failed success)))
    (should (= (gethash "1" (plist-get proof :reads) 0) 0))
    (should (= (plist-get proof :operation-pages) 4))
    (should (= (plist-get proof :pages) 9))))

(defun jaunder-test--publish-target-drift-proof (boundary kind)
  "Run selected push with target KIND drift during remote proof BOUNDARY."
  (let ((changed nil)
        (target-id (if (eq boundary 'discovery) "25" "1")))
    (let ((proof
           (jaunder-test--confirmed-write-batch
            'push
            (lambda (event state)
              (pcase (plist-get event :phase)
                ('setup
                 (when (eq boundary 'restoration)
                   (jaunder-test--write-source-links state "50" "post-001")
                   (puthash "50" (with-temp-buffer
                                   (insert-file-contents (gethash "50" (plist-get state :paths)))
                                   (buffer-string))
                            (plist-get state :originals))))
                ('write
                 (when (and (eq boundary 'restoration) (equal (plist-get event :id) "1"))
                   ;; Invalid returned alternate requires a targeted Member GET
                   ;; before the later selected source may use this target.
                   (list :body (jaunder-test--write-entry "1" "post-001"
                                                          "https://example.test/~alice/post-001" t))))
                ((or 'collection 'member)
                 (when (and (not changed)
                            (if (eq boundary 'discovery)
                                (and (eq (plist-get event :phase) 'collection)
                                     (not (plist-get event :refresh)))
                              (and (eq (plist-get event :phase) 'member)
                                   (equal (plist-get event :id) target-id))))
                   (setq changed t)
                   (let ((path (gethash target-id (plist-get state :paths))))
                     (if (eq kind 'rename)
                         (rename-file path (expand-file-name "moved.org" (plist-get state :root)))
                       (with-temp-buffer
                         (insert-file-contents path)
                         (pcase kind
                           ('body (goto-char (point-max)) (insert "Target body-only edit.\n"))
                           (_ (goto-char (point-min))
                              (re-search-forward (if (eq kind 'slug) "JAUNDER_SLUG [^\n]+" "JAUNDER_ID [0-9]+"))
                              (replace-match (if (eq kind 'slug) "JAUNDER_SLUG altered" "JAUNDER_ID 99"))))
                         (write-region (point-min) (point-max) path nil 'silent))))))))
            (if (eq boundary 'discovery) '(1) '(1 50)) (eq boundary 'discovery))))
      (should changed)
      (should (= (plist-get proof :initial-pages) 4))
      (should (= (plist-get proof :operation-pages) 4))
      (should (= (plist-get proof :pages) 8))
      (when (eq boundary 'restoration)
        (should (= (gethash "1" (plist-get proof :reads) 0) 1)))
      (if (eq kind 'body)
          (progn
            (should (cl-every (lambda (result) (eq (jaunder-reconcile-result-outcome result) 'success))
                              (plist-get proof :results)))
            (should (= (length (plist-get proof :writes)) (if (eq boundary 'discovery) 1 2)))
            (let* ((source (if (eq boundary 'discovery) "post-001.org" "post-050.org"))
                   (target (if (eq boundary 'discovery) "post-025" "post-001")))
              (should (string-match-p (regexp-quote (concat "[[file:./" target ".org][Target]]"))
                                      (cdr (assoc source (plist-get proof :files)))))))
        (let* ((source-id (if (eq boundary 'discovery) "1" "50"))
               (source-name (if (eq boundary 'discovery) "post-001.org" "post-050.org")))
          (should (= (length (plist-get proof :writes)) (if (eq boundary 'discovery) 0 1)))
          (should (eq (jaunder-reconcile-result-outcome (car (last (plist-get proof :results)))) 'failed))
          (should (equal (gethash source-id (plist-get proof :originals))
                         (cdr (assoc source-name (plist-get proof :files)))))
          (should (string-match-p "Local Post Link target local identity"
                                  (jaunder-reconcile-result-detail (car (last (plist-get proof :results)))))))))))

(ert-deftest jaunder-reconcile-push-rejects-target-rename-during-discovery ()
  "A singleton ID at a different path cannot repair the authored target."
  (jaunder-test--publish-target-drift-proof 'discovery 'rename))

(ert-deftest jaunder-reconcile-push-rejects-target-slug-drift-during-discovery ()
  "A singleton ID at the same path still needs its current canonical slug."
  (jaunder-test--publish-target-drift-proof 'discovery 'slug))

(ert-deftest jaunder-reconcile-push-rejects-target-id-drift-during-discovery ()
  "Changing the target ID cannot borrow the previously captured Member."
  (jaunder-test--publish-target-drift-proof 'discovery 'id))

(ert-deftest jaunder-reconcile-push-rechecks-target-after-targeted-restoration ()
  "The last remote proof operation cannot leave target path/slug evidence stale."
  (dolist (kind '(rename slug id))
    (jaunder-test--publish-target-drift-proof 'restoration kind)))

(ert-deftest jaunder-reconcile-push-allows-target-body-only-edits-during-proof ()
  "Target source digest is not Local Post Link identity or write authorization."
  (dolist (boundary '(discovery restoration))
    (jaunder-test--publish-target-drift-proof boundary 'body)))

(provide 'jaunder-reconcile-write-test)
;;; jaunder-reconcile-write-test.el ends here
