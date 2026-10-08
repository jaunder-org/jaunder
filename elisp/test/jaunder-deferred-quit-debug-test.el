;;; jaunder-deferred-quit-debug-test.el --- Preserve owner-deferred cancellation -*- lexical-binding: t; -*-

;;; Commentary:
;; Diagnostic completion must not steal pending keyboard cancellation from an
;; enclosing owner that inhibits quit until its mutation/result checkpoint.

;;; Code:
(require 'ert)
(require 'cl-lib)
(require 'jaunder)
(load (expand-file-name "jaunder-debug-boundary-fixture.el"
                        (file-name-directory (or load-file-name buffer-file-name))) nil t)

(ert-deftest jaunder-debug-deferred-quit-preserves-native-value-and-pending-input ()
  "An enclosing inhibit-quit owns pending input, both from BODY and the sink."
  (dolist (sink-input '(nil t))
    (jaunder-debug-boundary--with-session
     (let ((jaunder-debug t) (inhibit-quit t) (quit-flag nil)
           (value (list "private-value")) (finish (symbol-function 'jaunder--debug-finish)))
       (unwind-protect
           (cl-letf (((symbol-function 'jaunder--debug-finish)
                      (lambda (state) (funcall finish state) (when sink-input (setq quit-flag t)))))
             (should (eq value
                         (condition-case condition
                             (jaunder--with-debug-operation "publish.post" nil
                                                            (unless sink-input (setq quit-flag t)) value)
                           (quit condition))))
             (should quit-flag)
             (setq quit-flag nil)
             (jaunder-debug-boundary--assert-tree (jaunder-debug-boundary--text) '("publish.post")))
         (setq quit-flag nil))))))

(ert-deftest jaunder-debug-deferred-quit-native-batch-records-before-cancellation ()
  "The actual executor records the completed row then refreshes with quit cleared."
  (dolist (enabled '(nil t))
    (jaunder-debug-boundary--with-session
     (let* ((row (jaunder--make-reconcile-row :state 'local-draft :key "private-row"))
            (buffer (generate-new-buffer " *deferred quit report*"))
            (report (jaunder--make-reconcile-report :root "/private-root" :rows (list row)))
            (quit-flag nil) refresh-saw-quit result)
       (unwind-protect
           (progn
             (jaunder--render-reconcile-report report buffer)
             (setq jaunder-debug enabled)
             (cl-letf (((symbol-function 'jaunder--reconcile-refresh-buffer)
                        (lambda (&rest _) (setq refresh-saw-quit quit-flag) nil)))
               (setq result
                     (condition-case condition
                         (jaunder--reconcile-execute-batch
                          buffer (list row) 'push
                          (lambda (_)
                            (jaunder--with-debug-operation "publish.post" nil
                                                           (setq quit-flag t)
                                                           '(:outcome success :local-effect created))))
                       (quit condition))))
             (should (eq result 'cancelled))
             (should-not refresh-saw-quit)
             (should-not quit-flag)
             (with-current-buffer buffer
               (should (equal (mapcar #'jaunder-reconcile-result-row-key jaunder-reconcile-last-batch-results)
                              '("private-row")))
               (should (equal (mapcar #'jaunder-reconcile-result-outcome jaunder-reconcile-last-batch-results)
                              '(success)))))
         (setq quit-flag nil)
         (when (buffer-live-p buffer) (with-current-buffer buffer (set-buffer-modified-p nil)) (kill-buffer buffer)))))))

(provide 'jaunder-deferred-quit-debug-test)
;;; jaunder-deferred-quit-debug-test.el ends here
