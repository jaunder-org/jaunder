;;; jaunder-inventory-debug-test.el --- Inventory and report diagnostic proofs -*- lexical-binding: t; -*-

;;; Commentary:
;; Real local discovery, paginated Collection parsing, identity joining and
;; persistent report lifecycles retain their native results and read-only effects.

;;; Code:
(require 'ert)
(require 'cl-lib)
(require 'jaunder)
(load (expand-file-name "jaunder-debug-boundary-fixture.el"
                        (file-name-directory (or load-file-name buffer-file-name))) nil t)

(defun jaunder-inventory-debug--normalize (value root)
  "Normalize only per-run ROOT entropy in native VALUE projections."
  (cond ((stringp value) (replace-regexp-in-string (regexp-quote root) "ROOT" value t t))
        ((recordp value)
         (cons (type-of value)
               (cl-loop for index from 1 below (length value)
                        collect (jaunder-inventory-debug--normalize (aref value index) root))))
        ((vectorp value) (vconcat (mapcar (lambda (item) (jaunder-inventory-debug--normalize item root)) value)))
        ((consp value) (cons (jaunder-inventory-debug--normalize (car value) root)
                             (jaunder-inventory-debug--normalize (cdr value) root)))
        (t value)))

(defun jaunder-inventory-debug--page (id next)
  "Return a real Collection page with Post ID and optional NEXT link."
  (concat "<feed xmlns=\"http://www.w3.org/2005/Atom\" xmlns:j=\"https://jaunder.org/ns/atompub\">"
          (when next "<link rel=\"next\" href=\"https://private.example/page2\"/>")
          "<entry><link rel=\"edit\" href=\"https://private.example/atompub/private-user/posts/" id
          "\"/><j:slug>private-" id "</j:slug><j:etag>\"private-etag\"</j:etag></entry></feed>"))

(defun jaunder-inventory-debug--files (root)
  "Return root-level Org names and literal bytes under ROOT."
  (mapcar (lambda (path)
            (cons (file-name-nondirectory path)
                  (with-temp-buffer (set-buffer-multibyte nil)
                                    (insert-file-contents-literally path) (buffer-string))))
          (cl-remove-if-not #'file-regular-p (directory-files root t "\\.org\\'"))))

(defun jaunder-inventory-debug--run (enabled operation &optional kind)
  "Run real OPERATION with ENABLED and optional native failure KIND."
  (jaunder-debug-boundary--with-session
   (let* ((root (file-name-as-directory (make-temp-file "jaunder-inventory-private-" t)))
          (jaunder-blogs (list (cons root '(:base-url "https://private.example" :username "private-user"))))
          (jaunder--service-doc-cache nil)
          (old (get-buffer "*Jaunder Reconcile*"))
          (failure '("private-fault" 19))
          (org-hooks 0) requests progress displayed messages result before owned retained-text rendering-fault local-read-fault
          previous-report previous-marks previous-results
          (inserter (symbol-function 'insert))
          (reader (symbol-function 'insert-file-contents)))
     (when old (with-current-buffer old (rename-buffer (generate-new-buffer-name " *preserved report*"))))
     (dolist (entry '(("private-42.org" . "42") ("private-orphan.org" . "43")
                      ("private-draft.org" . nil) ("private-invalid.org" . "private-invalid")))
       (with-temp-file (expand-file-name (car entry) root)
         (insert "#+TITLE: private-title\n#+PROPERTY: JAUNDER_SLUG " (file-name-base (car entry)) "\n")
         (when (cdr entry) (insert "#+PROPERTY: JAUNDER_ID " (cdr entry) "\n"))
         (insert "#+PROPERTY: JAUNDER_SYNCED \"private-etag\"\n"
                 "#+PROPERTY: JAUNDER_SYNCED_AT 2099-01-01T00:00:00Z\n\nprivate-body\n")))
     (make-directory (expand-file-name "ignored.org" root))
     (setq before (jaunder-inventory-debug--files root))
     (unwind-protect
         (let ((org-mode-hook (list (lambda () (setq org-hooks (1+ org-hooks)))))
               (jaunder--inventory-page-progress (lambda (page) (push page progress))))
           (cl-letf (((symbol-function 'jaunder--http-request)
                      (lambda (method url &rest rest)
                        (push (cons method (cons url rest)) requests)
                        (let ((second (equal url "https://private.example/page2")))
                          (when (and second (memq kind '(error quit)) (not (eq operation 'refresh-render)))
                            (signal kind failure))
                          (list :status (if (and second (eq kind 'status)) 503 200)
                                :body (if (and second (eq kind 'malformed)) "private-malformed"
                                        (jaunder-inventory-debug--page
                                         (if (and second (not (eq kind 'duplicate))) "99" "42")
                                         (or (not second) (eq kind 'cycle))))))))
                     ((symbol-function 'insert-file-contents)
                      (lambda (path &rest args)
                        (when local-read-fault
                          (setq local-read-fault nil)
                          (signal kind failure))
                        (apply reader path args)))
                     ((symbol-function 'insert)
                      (lambda (&rest args)
                        (when (and rendering-fault (equal (buffer-name) "*Jaunder Reconcile*"))
                          (setq rendering-fault nil)
                          (signal kind failure))
                        (apply inserter args)))
                     ((symbol-function 'display-buffer) (lambda (buffer &rest _) (push (buffer-name buffer) displayed) nil))
                     ((symbol-function 'message) (lambda (format &rest args) (push (apply #'format format args) messages) nil)))
             ;; Prepare a real persistent report with marks/results, before enabling
             ;; diagnostics or the refresh fault.  No report/row owner is mocked.
             (when (memq operation '(refresh refresh-direct refresh-render))
               (let ((fault kind))
                 (setq kind nil)
                 (jaunder-reconcile root)
                 (setq owned (get-buffer "*Jaunder Reconcile*") kind fault)
                 (with-current-buffer owned
                   (puthash (jaunder--reconcile-stable-row-key (car (jaunder-reconcile-report-rows jaunder-reconcile-report)))
                            t jaunder-reconcile-marks)
                   (puthash 'stale t jaunder-reconcile-marks)
                   (setq-local jaunder-reconcile-last-batch-results
                               (list (jaunder--make-reconcile-result :action 'push :outcome 'blocked :detail "private-result")))
                   (jaunder--render-reconcile-report jaunder-reconcile-report owned)
                   (setq previous-report jaunder-reconcile-report previous-marks jaunder-reconcile-marks
                         previous-results jaunder-reconcile-last-batch-results))))
             (setq requests nil progress nil displayed nil messages nil jaunder-debug enabled
                   rendering-fault (eq operation 'refresh-render)
                   local-read-fault (and (eq operation 'local) kind))
             (setq result
                   (condition-case condition
                       (pcase operation
                         ('open (jaunder-reconcile root))
                         ((or 'refresh 'refresh-render) (with-current-buffer owned (jaunder-reconcile-refresh)))
                         ('refresh-direct (jaunder--reconcile-refresh-buffer owned))
                         ('local (jaunder--scan-root-locals root))
                         ('collection (jaunder--call-with-blog root #'jaunder--fetch-collection-members))
                         ('build
                          (let* ((local (jaunder--make-inventory-local :path (concat root "private-42.org") :id "42"))
                                 (member (jaunder--make-inventory-member :id "42" :slug "private-42"))
                                 (inventory (jaunder--join-inventory
                                             (list local) (list (if kind "private-invalid-member" member)))))
                            (should (eq local (jaunder-inventory-match-local (car (jaunder-inventory-matched inventory)))))
                            (should (eq member (jaunder-inventory-match-member (car (jaunder-inventory-matched inventory)))))
                            inventory)))
                     (error condition) (quit condition)))
             (cond ((memq kind '(error quit)) (should (equal result (cons kind failure))))
                   (kind (should (memq (car result) '(error wrong-type-argument jaunder-inventory-duplicate-remote-id)))))
             (unless kind
               (pcase operation
                 ('open (should (jaunder-reconcile-report-p result)))
                 ((or 'refresh 'refresh-direct) (should (eq result owned)))
                 ('local (should (= 4 (length result))))
                 ('collection (should (equal (mapcar #'jaunder-inventory-member-id result) '("42" "99"))))
                 ('build (should (jaunder-inventory-p result)))))
             (setq owned (or owned (get-buffer "*Jaunder Reconcile*")))
             (when owned
               (with-current-buffer owned
                 (should (equal (mapcar #'jaunder-reconcile-row-state
                                        (jaunder-reconcile-report-rows jaunder-reconcile-report))
                                '(unchanged orphan local-draft server-only inventory-conflict)))
                 (when previous-report
                   (should (eq previous-results jaunder-reconcile-last-batch-results))
                   (if (and kind (not (and (eq operation 'refresh-render) (eq kind 'quit))))
                       (progn
                         (should (eq previous-report jaunder-reconcile-report))
                         (should (eq previous-marks jaunder-reconcile-marks))
                         (should (= 2 (hash-table-count jaunder-reconcile-marks))))
                     (should (= 1 (hash-table-count jaunder-reconcile-marks))))
                   (when (and (eq operation 'refresh-render) (eq kind 'quit))
                     (should (= (point-min) (point-max)))
                     (should-not (eq previous-report jaunder-reconcile-report))))))
             (should (zerop org-hooks))
             (should (equal before (jaunder-inventory-debug--files root)))
             (when enabled
               (let* ((text (jaunder-debug-boundary--text))
                      (label (pcase operation ('open "report.open")
                                    ((or 'refresh 'refresh-direct 'refresh-render) "report.refresh")
                                    ('local "inventory.local") ('collection "inventory.collection") ('build "inventory.build"))))
                 (jaunder-debug-boundary--assert-tree
                  text (if (eq operation 'collection) (list "config.resolve" label) (list label))
                  (append '(("inventory.page" . "inventory.collection"))
                          (when (memq operation '(open refresh refresh-direct refresh-render))
                            (list (cons "inventory.local" label) (cons "inventory.collection" label)
                                  (cons "inventory.build" label)))))
                 (should (= 2 (jaunder-debug-boundary--label-count label text)))
                 (when (memq operation '(open refresh refresh-direct refresh-render collection))
                   (should (= (if (eq kind 'cycle) 6 4)
                              (jaunder-debug-boundary--label-count "inventory.page" text))))
                 (when (memq operation '(open refresh refresh-direct refresh-render))
                   (should (= 2 (jaunder-debug-boundary--label-count "inventory.local" text)))
                   (should (= (if (and kind (not (eq operation 'refresh-render))) 0 2)
                              (jaunder-debug-boundary--label-count "inventory.build" text))))
                 (setq retained-text text)
                 (should-not (string-match-p "private\\|https://\\|ROOT" text))))
             (unless enabled (should (zerop jaunder--debug-id-counter))
                     (should-not (get-buffer jaunder--debug-buffer-name)))
             (jaunder-inventory-debug--normalize
              (list (if (bufferp result) (buffer-name result) result) (nreverse requests) (nreverse progress)
                    (nreverse displayed) (nreverse messages)
                    (when owned
                      (with-current-buffer owned
                        (list (buffer-substring-no-properties (point-min) (point-max))
                              jaunder-reconcile-report jaunder-reconcile-last-batch-results
                              (hash-table-count jaunder-reconcile-marks) major-mode (point) (buffer-modified-p))))) root)))
       (when (buffer-live-p owned) (with-current-buffer owned (set-buffer-modified-p nil)) (kill-buffer owned))
       (when retained-text (should (equal retained-text (jaunder-debug-boundary--text))))
       (when (buffer-live-p old) (with-current-buffer old (rename-buffer "*Jaunder Reconcile*")))
       (delete-directory root t)))))

(ert-deftest jaunder-inventory-debug-real-report-open-refresh-and-standalone-off-on ()
  "Real read-only inventories, render/refresh, native identity and roots agree."
  (dolist (operation '(open refresh refresh-direct local collection build))
    (let ((off (jaunder-inventory-debug--run nil operation))
          (on (jaunder-inventory-debug--run t operation)))
      (should (equal off on))
      (when (memq operation '(open refresh refresh-direct collection))
        (should (= 2 (length (nth 1 on))))
        (should (equal (nth 2 on) '(1 2)))))))

(ert-deftest jaunder-inventory-debug-page-wire-errors-quits-preserve-report ()
  "Later page failures propagate exact conditions and retain reviewable reports."
  (dolist (operation '(open refresh collection))
    (dolist (kind '(error quit))
      (should (equal (jaunder-inventory-debug--run nil operation kind)
                     (jaunder-inventory-debug--run t operation kind))))))

(ert-deftest jaunder-inventory-debug-collection-validation-errors-off-on ()
  "Malformed/status/duplicate/cyclic pages retain native failure and progress."
  (dolist (kind '(malformed status duplicate cycle))
    (let ((off (jaunder-inventory-debug--run nil 'collection kind))
          (on (jaunder-inventory-debug--run t 'collection kind)))
      (should (equal off on))
      (should (= 2 (length (nth 1 on))))
      (should (equal (nth 2 on) (if (eq kind 'cycle) '(1 2) '(1)))))))

(ert-deftest jaunder-inventory-debug-local-and-join-native-failures ()
  "Standalone discovery and joining retain native errors and keyboard cancellation."
  (dolist (kind '(error quit))
    (should (equal (jaunder-inventory-debug--run nil 'local kind)
                   (jaunder-inventory-debug--run t 'local kind))))
  (should (equal (jaunder-inventory-debug--run nil 'build 'invalid-member)
                 (jaunder-inventory-debug--run t 'build 'invalid-member))))

(ert-deftest jaunder-inventory-debug-refresh-render-errors-quits-retain-native-effects ()
  "A partial render preserves native rollback/error and quit behavior exactly."
  (dolist (kind '(error quit))
    (should (equal (jaunder-inventory-debug--run nil 'refresh-render kind)
                   (jaunder-inventory-debug--run t 'refresh-render kind)))))

(ert-deftest jaunder-inventory-debug-disabled-factories-never-run ()
  "Actual inventory/report workflows bypass all diagnostic factories when disabled."
  (let ((calls 0))
    (cl-letf (((symbol-function 'jaunder--debug-begin)
               (lambda (&rest _) (setq calls (1+ calls)) (ert-fail "disabled setup")))
              ((symbol-function 'jaunder--debug-timestamp)
               (lambda (&rest _) (setq calls (1+ calls)) (ert-fail "disabled clock")))
              ((symbol-function 'jaunder--debug-common-event)
               (lambda (&rest _) (setq calls (1+ calls)) (ert-fail "disabled formatting")))
              ((symbol-function 'jaunder--debug-fields-internal)
               (lambda (&rest _) (setq calls (1+ calls)) (ert-fail "disabled fields")))
              ((symbol-function 'jaunder--debug-buffer)
               (lambda (&rest _) (setq calls (1+ calls)) (ert-fail "disabled buffer")))
              ((symbol-function 'jaunder--debug-complete)
               (lambda (&rest _) (setq calls (1+ calls)) (ert-fail "disabled terminal"))))
      (dolist (operation '(open refresh refresh-direct local collection build))
        (jaunder-inventory-debug--run nil operation)))
    (should (zerop calls))))

(provide 'jaunder-inventory-debug-test)
;;; jaunder-inventory-debug-test.el ends here
