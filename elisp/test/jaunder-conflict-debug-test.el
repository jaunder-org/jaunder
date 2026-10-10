;;; jaunder-conflict-debug-test.el --- Conflict/Ediff lifecycle diagnostics -*- lexical-binding: t; -*-

;;; Commentary:
;; Reuse existing native mutation/fault assertions, then require the new owner
;; traces. Ediff itself is controlled by the existing fixtures (no display).
;; Scratch cancellation/discard have independent callback roots and no publish.

;;; Code:
(require 'ert)
(require 'cl-lib)
(require 'jaunder)
(defconst jaunder-conflict-debug--test-directory
  (file-name-directory (or load-file-name buffer-file-name)))
(load (expand-file-name "jaunder-debug-boundary-fixture.el" jaunder-conflict-debug--test-directory) nil t)

(defun jaunder-conflict-debug--replay (test labels)
  "Replay native TEST off/on and require complete owner LABELS when enabled.
Existing native assertions authorize mutation/order/partial-effect claims, not
live-network claims. The shared checker independently verifies graph topology."
  ;; The full runner registers all files before execution. Focused execution
  ;; loads the legacy contracts here, never ahead of that runner's census.
  (unless (ert-test-boundp test)
    (load (expand-file-name "jaunder-reconcile-test.el" jaunder-conflict-debug--test-directory) nil t))
  (dolist (enabled '(nil t))
    (jaunder-debug-boundary--with-session
     (setq jaunder-debug enabled)
     (save-window-excursion (funcall (ert-test-body (ert-get-test test))))
     (if (not enabled)
         (progn (should (zerop jaunder--debug-id-counter))
                (should-not (get-buffer jaunder--debug-buffer-name)))
       (let* ((text (jaunder-debug-boundary--text))
              (roots (cl-count-if (lambda (line) (and (string-match-p " phase=start" line)
                                                      (not (string-match-p " parent=" line))))
                                  (split-string text "\n" t))))
         (jaunder-debug-boundary--assert-tree text roots)
         (dolist (label labels)
           (should (> (jaunder-debug-boundary--label-count label text) 0))))))))

(ert-deftest jaunder-conflict-debug-confirmation-command-owners ()
  "Actual selected resolution commands preserve prompts, eligibility and mutation."
  (jaunder-conflict-debug--replay
   'jaunder-reconcile-keep-local-confirms-selection-and-refuses-other-states
   '("conflict.local" "reconcile.row"))
  (jaunder-conflict-debug--replay
   'jaunder-reconcile-keep-remote-confirms-selection-before-post-mutation
   '("conflict.remote")))

(ert-deftest jaunder-conflict-debug-actual-command-batch-row-parentage ()
  "Confirmed native commands really enclose the native executor and row owner."
  (dolist (case '((jaunder-reconcile-keep-local-selected . "conflict.local")
                  (jaunder-reconcile-keep-remote-selected . "conflict.remote")))
    (jaunder-debug-boundary--with-session
     (let* ((jaunder-blogs '(("/private-root/" :base-url "https://example.test" :username "alice")))
            (row (jaunder--make-reconcile-row :state 'orphan :key "private-command-row"))
            (report (jaunder--make-reconcile-report :root "/private-root/" :rows (list row)))
            (buffer (generate-new-buffer " *owned command report*")))
       (unwind-protect
           (with-current-buffer buffer
             (jaunder--render-reconcile-report report buffer)
             (puthash (jaunder-reconcile-row-key row) t jaunder-reconcile-marks)
             (setq jaunder-debug t)
             (cl-letf (((symbol-function 'y-or-n-p) (lambda (_) t))
                       ((symbol-function 'jaunder--reconcile-refresh-buffer)
                        (lambda (target) (jaunder--render-reconcile-report report target))))
               (should (eq (funcall (car case)) 'completed)))
             (should (eq (jaunder-reconcile-result-outcome (car jaunder-reconcile-last-batch-results)) 'blocked))
             (let ((text (jaunder-debug-boundary--text)))
               (jaunder-debug-boundary--assert-tree text (list (cdr case))
                                                    (list (cons "reconcile.batch" (cdr case))
                                                          '("reconcile.row" . "reconcile.batch")))
               (should-not (string-match-p "private" text))))
         (when (buffer-live-p buffer) (kill-buffer buffer)))))))

(ert-deftest jaunder-conflict-debug-ediff-stage-and-independent-callbacks ()
  "Real stage/Ediff setup, finish, cancel and discard retain native lifecycle."
  (jaunder-conflict-debug--replay
   'jaunder-reconcile-merge-ediff-keeps-independent-authored-scratch
   '("conflict.merge" "merge.stage" "merge.cancel"))
  (jaunder-conflict-debug--replay
   'jaunder-reconcile-merge-finish-installs-only-after-confirmed-put
   '("merge.finish"))
  (jaunder-conflict-debug--replay
   'jaunder-reconcile-merge-discard-requires-explicit-consent
   '("merge.discard")))

(ert-deftest jaunder-conflict-debug-staging-setup-and-finish-failures ()
  "Existing actual staging/startup/drift/partial/recovery assertions run off/on."
  (dolist (case '((jaunder-reconcile-merge-post-staging-drift-does-not-open-scratch "conflict.merge" "merge.stage")
                  (jaunder-reconcile-merge-ediff-startup-failure-cannot-publish-local-only "conflict.merge")
                  (jaunder-reconcile-merge-partial-ediff-startup-retains-unpublishable-result "conflict.merge")
                  (jaunder-reconcile-merge-finish-retains-scratch-after-each-failure "merge.finish")
                  (jaunder-reconcile-merge-finish-reports-preparation-failure-and-retains-racing-edit "merge.finish")
                  (jaunder-reconcile-merge-finish-blocks-post-ediff-local-and-remote-drift "merge.finish")))
    (jaunder-conflict-debug--replay (car case) (cdr case))))

(ert-deftest jaunder-conflict-debug-cancel-discard-roots-preserve-private-scratch ()
  "Delayed user callbacks are separate roots; cancellation never retires scratch."
  (jaunder-debug-boundary--with-session
   (let* ((scratch (generate-new-buffer " *private merge scratch*"))
          (session (jaunder--make-reconcile-merge-session :scratch scratch :ediff-ready t)))
     (unwind-protect
         (with-current-buffer scratch
           (jaunder-reconcile-merge-mode)
           (setq-local jaunder-reconcile-merge-session session)
           (insert "private authored merge result")
           (setq jaunder-debug t)
           (jaunder-reconcile-merge-cancel)
           (should (buffer-live-p scratch))
           (should (equal (buffer-string) "private authored merge result"))
           (cl-letf (((symbol-function 'y-or-n-p) (lambda (_) nil)))
             (jaunder-reconcile-merge-discard))
           (should (buffer-live-p scratch))
           (cl-letf (((symbol-function 'y-or-n-p) (lambda (_) t)))
             (jaunder-reconcile-merge-discard))
           (should-not (buffer-live-p scratch))
           (let ((text (jaunder-debug-boundary--text)))
             (jaunder-debug-boundary--assert-tree text '("merge.cancel" "merge.discard" "merge.discard"))
             (should-not (string-match-p "private" text))))
       (when (buffer-live-p scratch)
         (with-current-buffer scratch
           (setq-local jaunder-reconcile-merge-allow-kill t)
           (set-buffer-modified-p nil))
         (kill-buffer scratch))))))

(ert-deftest jaunder-conflict-debug-owner-native-error-quit-and-private-payloads ()
  "All command/callback owners preserve exact native conditions, without payloads."
  (dolist (case '((jaunder-reconcile-keep-local-selected jaunder-reconcile-selected-rows "conflict.local")
                  (jaunder-reconcile-keep-remote-selected jaunder-reconcile-selected-rows "conflict.remote")
                  (jaunder-reconcile-merge-selected get-text-property "conflict.merge")
                  (jaunder--reconcile-merge-stage jaunder--reconcile-conflict-preflight "merge.stage")
                  (jaunder-reconcile-merge-finish y-or-n-p "merge.finish")
                  (jaunder-reconcile-merge-cancel message "merge.cancel")
                  (jaunder-reconcile-merge-discard y-or-n-p "merge.discard")))
    (dolist (kind '(error quit))
      (jaunder-debug-boundary--with-session
       (with-temp-buffer
         (setq-local jaunder-reconcile-merge-session
                     (jaunder--make-reconcile-merge-session
                      :row (jaunder--make-reconcile-row) :report-buffer (current-buffer) :ediff-ready t))
         (let ((data (list "private-native-failure")) results)
           (dolist (enabled '(nil t))
             (setq jaunder-debug enabled)
             (cl-letf (((symbol-function (nth 1 case)) (lambda (&rest _) (signal kind data))))
               (push (condition-case err
                         (if (eq (car case) 'jaunder--reconcile-merge-stage)
                             (funcall (car case) nil) (funcall (car case)))
                       (error err) (quit err)) results)))
           (should (equal (car results) (cons kind data)))
           (should (equal (car results) (cadr results)))
           (let ((text (jaunder-debug-boundary--text)))
             (jaunder-debug-boundary--assert-tree text (list (nth 2 case)))
             (should (string-match-p (concat "outcome=" (if (eq kind 'quit) "cancelled" "error")) text))
             (should-not (string-match-p "private" text)))))))))

(provide 'jaunder-conflict-debug-test)
;;; jaunder-conflict-debug-test.el ends here
