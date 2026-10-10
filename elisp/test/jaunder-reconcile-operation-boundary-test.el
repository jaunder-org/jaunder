;;; jaunder-reconcile-operation-boundary-test.el --- Evidence boundary outcomes -*- lexical-binding: t; -*-

;; Copyright (C) 2026 Jaunder contributors

;;; Commentary:
;; Supported standalone/nested scalar consumers and confirmed commands retain
;; native cancellation, unresolved-create protection and failed target proof.
;; HTTP responses are deterministic doubles; filesystem and owner logic are real.

;;; Code:

(require 'ert)
(require 'jaunder)
(load (expand-file-name "jaunder-reconcile-write-fixture.el"
                        (file-name-directory (or load-file-name buffer-file-name))) nil t)

(ert-deftest jaunder-operation-standalone-post-link-evidence-acquires-current-inventory ()
  "An unscoped link consumer gets complete discovery and real current locals."
  (let* ((root (make-temp-file "jaunder-standalone-proof-" t))
         (path (expand-file-name "post-001.org" root))
         (jaunder--active-blog '(:base-url "https://example.test" :username "alice"))
         (jaunder-blogs (list (cons (file-name-as-directory root) jaunder--active-blog)))
         (reads 0))
    (unwind-protect
        (progn
          (with-temp-file path (insert "#+PROPERTY: JAUNDER_ID 1\n#+PROPERTY: JAUNDER_SLUG post-001\n\nBody.\n"))
          (cl-letf (((symbol-function 'jaunder--http-request)
                     (lambda (method url &rest _)
                       (should (equal method "GET"))
                       (should (equal url (jaunder--collection-url)))
                       (setq reads (1+ reads))
                       (list :status 200 :body
                             (concat "<feed xmlns=\"http://www.w3.org/2005/Atom\">"
                                     (jaunder-test--write-entry "1" "post-001" "https://example.test/~alice/post-001")
                                     "</feed>")))))
            (should-not (jaunder--operation-active-p))
            (pcase-let ((`(,members ,locals) (jaunder--operation-post-link-evidence root)))
              (should (= (length members) 1))
              (should (= (length locals) 1))
              (should (equal (jaunder-inventory-member-id (car members)) "1"))
              (should (equal (jaunder-inventory-local-path (car locals)) path)))
            (should (= reads 1))))
      (delete-directory root t))))

(ert-deftest jaunder-operation-final-local-proof-rejects-missing-or-moved-post ()
  "One current ID at another path is not the reviewed path; absence is not proof."
  (let* ((root (make-temp-file "jaunder-final-local-" t))
         (old (expand-file-name "old.org" root))
         (new (expand-file-name "new.org" root)))
    (unwind-protect
        (progn
          (with-temp-file old (insert "#+PROPERTY: JAUNDER_ID 7\n\nBody.\n"))
          (should (plist-get (jaunder--operation-local-unique-match root "7" old) :ok))
          (rename-file old new)
          (should (eq (plist-get (jaunder--operation-local-unique-match root "7" old) :reason)
                      'matched-identity-changed))
          (should (plist-get (jaunder--operation-local-unique-match root "7" new) :ok))
          (delete-file new)
          (should (eq (plist-get (jaunder--operation-local-unique-match root "7" old) :reason)
                      'matched-identity-changed)))
      (delete-directory root t))))

(ert-deftest jaunder-operation-isolated-checkpoint-retains-unresolved-create-guards ()
  "An isolated save advances owned lineage, not guessed ID write authority."
  (let* ((root (make-temp-file "jaunder-isolated-checkpoint-" t))
         (path (expand-file-name "created.org" root))
         (buffer (find-file-noselect path))
         (jaunder-blogs (list (cons (file-name-as-directory root)
                                    '(:base-url "https://example.test" :username "alice"))))
         (response (list :status 201 :headers '(("location" . "not-a-url/1") ("etag" . "\"written\""))
                         :body (jaunder-test--write-entry "101" "created" "https://example.test/~alice/created")))
         (xml (jaunder--atom-entry->xml
               (jaunder--make-entry :title "Created" :draft t :content-type "text/org" :body "Authored.\n")))
         (sends 0) report-buffer)
    (unwind-protect
        (jaunder--call-with-blog
         root
         (lambda ()
           (with-current-buffer buffer
             (insert "#+TITLE: Created\n#+PROPERTY: JAUNDER_DATE_TZ UTC\n\nAuthored.\n")
             (save-buffer)
             (cl-letf (((symbol-function 'y-or-n-p) (lambda (&rest _) t))
                       ((symbol-function 'jaunder--http-request)
                        (lambda (method url &rest _)
                          (if (equal method "POST")
                              (progn (setq sends (1+ sends)) response)
                            (should (equal method "GET"))
                            (should (equal url (jaunder--collection-url)))
                            (list :status 200 :body "<feed xmlns=\"http://www.w3.org/2005/Atom\"/>")))))
               (jaunder--call-with-reconcile-operation
                root (jaunder--active-base-url) (jaunder--active-username)
                (lambda ()
                  (jaunder--call-with-operation-write-receipt
                   (lambda ()
                     (jaunder--operation-send-post-write "POST" (jaunder--collection-url) xml jaunder--entry-content-type nil)
                     (should (eq (jaunder--operation-write-phase) 'confirmed))
                     (should (jaunder--operation-created-identity-unresolved-p))))
                  (jaunder--call-without-reconcile-operation
                   (lambda ()
                     ;; Write-back really persists the returned Location ID but
                     ;; cannot establish its contradictory Member correlation.
                     (jaunder--write-back response t)))
                  (should (equal (jaunder--buffer-property "JAUNDER_ID") "1"))
                  (should (jaunder--operation-unresolved-create-path-p path))
                  (let ((proof (jaunder--operation-unique-match root "1" path)))
                    (should (eq (plist-get proof :reason) 'fresh-inventory-failed))
                    (should (string-match-p "create identity is unresolved" (plist-get proof :detail))))
                  (let ((condition (should-error (jaunder--operation-send-post-write
                                                  "PUT" (jaunder--member-url "1") xml jaunder--entry-content-type nil))))
                    (should (string-match-p "local header cannot authorize an update" (error-message-string condition))))
                  (let ((row (jaunder--make-reconcile-row
                              :key "post:1" :state 'unchanged
                              :local (jaunder--make-inventory-local :id "1" :slug "created" :path path)
                              :member (jaunder--make-inventory-member :id "1" :slug "created"
                                                                      :edit-uri (jaunder--member-url "1")))))
                    (setq report-buffer (jaunder--render-reconcile-report
                                         (jaunder--make-reconcile-report :root root :rows (list row))
                                         (generate-new-buffer " *unresolved delete*")))
                    (with-current-buffer report-buffer
                      (puthash (jaunder--reconcile-stable-row-key row) t jaunder-reconcile-marks)
                      (should (eq (jaunder-reconcile-delete-selected) 'completed))
                      (let ((result (car jaunder-reconcile-last-batch-results)))
                        (should (eq (jaunder-reconcile-result-outcome result) 'blocked))
                        (should (string-match-p "create identity is unresolved"
                                                (jaunder-reconcile-result-detail result))))))))
               (should (= sends 1))))))
      (when (buffer-live-p report-buffer) (kill-buffer report-buffer))
      (when (buffer-live-p buffer) (with-current-buffer buffer (set-buffer-modified-p nil)) (kill-buffer buffer))
      (delete-directory root t))))

(ert-deftest jaunder-reconcile-initial-create-rejection-is-not-unknown-or-committed ()
  "A rejected first keyed send keeps recovery intent without claiming commitment."
  (let ((proof (jaunder-test--confirmed-write-batch
                'push (lambda (event state)
                        (pcase (plist-get event :phase)
                          ('setup (jaunder-test--write-selected-as-creates state))
                          ('write '(:status 403 :no-commit t)))) '(1))))
    (should (= (length (plist-get proof :writes)) 1))
    (should (eq (jaunder-reconcile-result-outcome (car (plist-get proof :results))) 'failed))
    (should (string-match-p "HTTP 403" (jaunder-reconcile-result-detail (car (plist-get proof :results)))))
    (should (string-match-p "JAUNDER_CREATE_KEY" (cdr (assoc "post-001.org" (plist-get proof :files)))))
    (should-not (string-match-p "JAUNDER_ID" (cdr (assoc "post-001.org" (plist-get proof :files)))))))

(ert-deftest jaunder-reconcile-malformed-confirmed-create-retains-partial-outcome ()
  "A malformed accepted create response is a decode failure after commitment."
  (let ((proof (jaunder-test--confirmed-write-batch
                'push (lambda (event state)
                        (pcase (plist-get event :phase)
                          ('setup (jaunder-test--write-selected-as-creates state))
                          ('write '(:body "<entry xmlns=\"http://www.w3.org/2005/Atom\">")))) '(1))))
    (should (= (length (plist-get proof :writes)) 1))
    (should (eq (jaunder-reconcile-result-outcome (car (plist-get proof :results))) 'partial))
    (should (string-match-p "Remote Post committed" (jaunder-reconcile-result-detail (car (plist-get proof :results)))))
    (should (string-match-p "JAUNDER_CREATE_KEY" (cdr (assoc "post-001.org" (plist-get proof :files)))))))

(ert-deftest jaunder-reconcile-target-http-failure-is-retained-without-row-retry ()
  "An unavailable dirty target fails later sources without another traversal/read."
  (let ((proof (jaunder-test--confirmed-write-batch
                'push (lambda (event state)
                        (pcase (plist-get event :phase)
                          ('setup (dolist (id '("50" "100"))
                                    (jaunder-test--write-source-links state id "post-001")))
                          ('write (when (equal (plist-get event :id) "1")
                                    (signal 'plz-error '("lost accepted PUT"))))
                          ('member (when (equal (plist-get event :id) "1") '(:status 503))))))))
    (should (= (length (plist-get proof :writes)) 1))
    (should (= (gethash "1" (plist-get proof :reads) 0) 1))
    (should (= (plist-get proof :operation-pages) 4))
    (should (= (plist-get proof :pages) 8))
    (should (equal (mapcar #'jaunder-reconcile-result-outcome (plist-get proof :results)) '(unknown failed failed)))
    (dolist (result (cdr (plist-get proof :results)))
      (should (string-match-p "targeted Member GET failed (HTTP 503)" (jaunder-reconcile-result-detail result))))))

(ert-deftest jaunder-reconcile-pre-send-quit-remains-native-cancellation ()
  "Quit while acquiring required link proof is not a failed or unknown send."
  (let ((sends 0) cancelled)
    (condition-case condition
        (jaunder-test--confirmed-write-batch
         'push (lambda (event _state)
                 (when (eq (plist-get event :phase) 'write) (setq sends (1+ sends)))
                 (when (and (eq (plist-get event :phase) 'collection) (not (plist-get event :refresh)))
                   (signal 'quit nil))) '(1) t)
      (quit (setq cancelled condition quit-flag nil)))
    (should (eq (car cancelled) 'quit))
    (should (= sends 0))))

(provide 'jaunder-reconcile-operation-boundary-test)
;;; jaunder-reconcile-operation-boundary-test.el ends here
