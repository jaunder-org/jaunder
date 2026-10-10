;;; jaunder-reconcile-debug-test.el --- Batch/row diagnostic contracts -*- lexical-binding: t; -*-

;;; Commentary:
;; Selected-row owners and the real confirmed executor preserve native outcomes,
;; ordering/checkpoints and deferred cancellation.  Field producers use only
;; literal enum outcomes, never IDs, paths, authored data or condition details.

;;; Code:
(require 'ert)
(require 'cl-lib)
(require 'jaunder)
(load (expand-file-name "jaunder-debug-boundary-fixture.el"
                        (file-name-directory (or load-file-name buffer-file-name))) nil t)

(defun jaunder-reconcile-debug--row-call (action row)
  "Call the actual row owner for ACTION and ROW."
  (pcase action
    ('push (jaunder--reconcile-push-row row))
    ('pull (jaunder--reconcile-pull-row row))
    ('keep-local (jaunder--reconcile-keep-local-row row))
    ('keep-remote (jaunder--reconcile-keep-remote-row row))
    ('delete (jaunder--reconcile-delete-row row '(:outcome blocked :detail "private-review")))))

(ert-deftest jaunder-reconcile-debug-real-row-owners-standalone-noop-blocked ()
  "All five actual row owners produce standalone action/decision traces."
  (dolist (action '(push pull keep-local keep-remote delete))
    (dolist (state '(unchanged private-ineligible))
      (let ((row (jaunder--make-reconcile-row :state state :key "private-row"
                                              :member (jaunder--make-inventory-member :id "42" :slug "private-slug"))) results)
        (jaunder-debug-boundary--with-session
         (dolist (enabled '(nil t))
           (setq jaunder-debug enabled)
           (push (jaunder-reconcile-debug--row-call action row) results)
           (unless enabled (should (zerop jaunder--debug-id-counter))
                   (should-not (get-buffer jaunder--debug-buffer-name))))
         (should (equal (car results) (cadr results)))
         (let ((text (jaunder-debug-boundary--text))
               (decision (if (and (eq state 'unchanged) (memq action '(push pull))) "no-op" "blocked")))
           (jaunder-debug-boundary--assert-tree text '("reconcile.row"))
           (should (string-match-p (concat "action=" (symbol-name action)) text))
           (should (string-match-p (concat "phase=end .*decision=" decision) text))
           (should-not (string-match-p "private" text))))))))

(defun jaunder-reconcile-debug--batch (enabled mode)
  "Run real row owners through the actual executor with ENABLED and MODE."
  (jaunder-debug-boundary--with-session
   (let* ((jaunder--active-blog '(:base-url "https://example.test" :username "alice"))
          (first (jaunder--make-reconcile-row :state 'unchanged :key "private-one"))
          (second (jaunder--make-reconcile-row :state 'orphan :key "private-two"))
          (foreign (jaunder--make-reconcile-row :state 'unchanged :key "private-foreign"))
          (buffer (generate-new-buffer " *batch diagnostic proof*"))
          (report (jaunder--make-reconcile-report :root "/private-root" :rows (list first second)))
          (quit-flag nil) (checks 0) invoked checkpoints messages result)
     (unwind-protect
         (progn
           (jaunder--render-reconcile-report report buffer)
           (setq jaunder-debug enabled)
           (cl-letf (((symbol-function 'message) (lambda (format &rest args) (push (apply #'format format args) messages) nil))
                     ((symbol-function 'jaunder--reconcile-refresh-buffer)
                      (lambda (target)
                        (should-not quit-flag)
                        (if (eq mode 'refresh-fail) (error "private-refresh")
                          (jaunder--render-reconcile-report report target)))))
             (setq result
                   (jaunder--reconcile-execute-batch
                    buffer (list second foreign first first) 'push
                    (lambda (row)
                      (push (jaunder-reconcile-row-key row) invoked)
                      (push (with-current-buffer buffer (length jaunder-reconcile-last-batch-results)) checkpoints)
                      (when (and (eq mode 'error) (eq row first)) (error "private-operation"))
                      (let ((value (jaunder--reconcile-push-row row)))
                        (when (eq mode 'queued-quit) (setq quit-flag t))
                        value))
                    (when (memq mode '(cancel before))
                      (lambda () (= (setq checks (1+ checks)) (if (eq mode 'before) 1 2)))))))
           (when enabled
             (let ((text (jaunder-debug-boundary--text)))
               (jaunder-debug-boundary--assert-tree text '("reconcile.batch")
                                                    '(("reconcile.row" . "reconcile.batch")))
               (should (string-match-p
                        (concat "label=reconcile.batch phase=end .*action=push.*decision="
                                (pcase mode ('before "no-op") ('error "unknown")
                                       ((or 'cancel 'queued-quit 'refresh-fail) "partial") (_ "blocked"))) text))
               (should-not (string-match-p "private" text))))
           (unless enabled (should (zerop jaunder--debug-id-counter))
                   (should-not (get-buffer jaunder--debug-buffer-name)))
           (with-current-buffer buffer
             (list result (nreverse invoked) (nreverse checkpoints) (nreverse messages)
                   (mapcar (lambda (entry) (list (jaunder-reconcile-result-row-key entry)
                                                 (jaunder-reconcile-result-outcome entry)
                                                 (jaunder-reconcile-result-detail entry)))
                           jaunder-reconcile-last-batch-results)
                   (buffer-substring-no-properties (point-min) (point-max)))))
       (setq quit-flag nil)
       (when (buffer-live-p buffer) (with-current-buffer buffer (set-buffer-modified-p nil)) (kill-buffer buffer))))))

(ert-deftest jaunder-reconcile-debug-executor-order-cancellation-results-refresh ()
  "Actual displayed ordering/foreign filtering/append/cancel/refresh agree off/on."
  (dolist (mode '(normal cancel before queued-quit error refresh-fail))
    (let ((off (jaunder-reconcile-debug--batch nil mode))
          (on (jaunder-reconcile-debug--batch t mode)))
      (should (equal off on))
      (should (eq (car on) (pcase mode ('refresh-fail 'refresh-failed)
                                  ((or 'cancel 'before 'queued-quit) 'cancelled) (_ 'completed))))
      (should (equal (nth 1 on) (pcase mode ('before nil) ((or 'cancel 'queued-quit) '("private-one"))
                                       (_ '("private-one" "private-two")))))
      (should (equal (nth 2 on) (pcase mode ('before nil) ((or 'cancel 'queued-quit) '(0)) (_ '(0 1))))))))

(ert-deftest jaunder-reconcile-debug-row-result-field-producer-privacy-and-native-identity ()
  "The real row wrapper maps native outcomes only and retains opaque result identity."
  (jaunder-debug-boundary--with-session
   (dolist (case '((success . "proceed") (blocked . "blocked") (no-op . "no-op")
                   (partial . "partial") (unknown . "remote-unknown")
                   (failed . "unknown") (private-outcome . "unknown")))
     (let ((value (list :outcome (car case) :detail "private-result")))
       (setq jaunder-debug t)
       (should (eq value (jaunder--with-reconcile-row-debug "push" value)))
       (should (string-match-p (concat "decision=" (cdr case)) (jaunder-debug-boundary--text)))))
   (should-not (string-match-p "private" (jaunder-debug-boundary--text)))))

(ert-deftest jaunder-reconcile-debug-batch-retained-outcome-projection ()
  "Aggregation projects only retained outcome enums, including remote uncertainty."
  (with-temp-buffer
    (dolist (case '((nil . "no-op") ((no-op) . "no-op") ((success no-op) . "proceed")
                    ((blocked no-op) . "blocked") ((success blocked) . "partial")
                    ((partial) . "partial") ((unknown partial) . "remote-unknown")
                    ((failed) . "unknown") ((private-outcome) . "unknown")))
      (setq-local jaunder-reconcile-last-batch-results
                  (mapcar (lambda (outcome) (jaunder--make-reconcile-result
                                             :outcome outcome :detail "private-native-detail")) (car case)))
      (should (equal (cdr case) (jaunder--reconcile-debug-batch-decision (current-buffer) 'completed))))))

(ert-deftest jaunder-reconcile-debug-disabled-field-laziness-and-native-conditions ()
  "Real owners do not project fields when off; native error/quit data stays exact."
  (cl-letf (((symbol-function 'jaunder--reconcile-debug-action) (lambda (&rest _) (error "eager action")))
            ((symbol-function 'jaunder--reconcile-debug-decision) (lambda (&rest _) (error "eager row decision")))
            ((symbol-function 'jaunder--reconcile-debug-batch-decision) (lambda (&rest _) (error "eager batch decision"))))
    (jaunder-reconcile-debug--batch nil 'normal))
  (dolist (kind '(error quit))
    (jaunder-debug-boundary--with-session
     (let ((data (list "private-native-condition")) results
           (row (jaunder--make-reconcile-row :state 'unchanged)))
       (dolist (enabled '(nil t))
         (setq jaunder-debug enabled)
         (cl-letf (((symbol-function 'jaunder--reconcile-row-post-id) (lambda (_) (signal kind data))))
           (push (condition-case err (jaunder--reconcile-push-row row)
                   (error err) (quit err)) results)))
       (should (equal (car results) (cons kind data)))
       (should (equal (car results) (cadr results)))
       (let ((text (jaunder-debug-boundary--text)))
         (jaunder-debug-boundary--assert-tree text '("reconcile.row"))
         (should (string-match-p (concat "outcome=" (if (eq kind 'quit) "cancelled" "error")) text))
         (should-not (string-match-p "private" text)))))))

(provide 'jaunder-reconcile-debug-test)
;;; jaunder-reconcile-debug-test.el ends here
