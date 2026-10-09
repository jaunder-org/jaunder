;;; jaunder-debug-edebug-test.el --- Lazy fields under Edebug -*- lexical-binding: t; -*-

;;; Commentary:
;; Coverage instruments real caller forms using the diagnostic macro's declared
;; Edebug specification. Literal plist keys are syntax, not function calls;
;; value forms must remain lazy and retain their enclosing bindings.

;;; Code:
(require 'ert)
(require 'cl-lib)
(require 'edebug)
(require 'jaunder-debug)
(load (expand-file-name "jaunder-debug-boundary-fixture.el"
                        (file-name-directory (or load-file-name buffer-file-name))) nil t)

(defmacro jaunder-debug-edebug--with-owner (&rest body)
  "Instrument the actual macro call syntax in a temporary owner, then run BODY."
  (declare (indent 0) (debug body))
  `(let* ((name 'jaunder-debug-edebug--owner)
          (saved-function (and (fboundp name) (symbol-function name)))
          (saved-plist (copy-sequence (symbol-plist name)))
          (edebug-initial-mode 'go)
          (edebug-coverage t))
     (unwind-protect
         (progn
           (with-temp-buffer
             (emacs-lisp-mode)
             (insert "(defun jaunder-debug-edebug--owner (field body)\n"
                     "  (jaunder--with-debug-operation \"publish.validate\"\n"
                     "      (format (funcall field) count (progn (funcall field) 7))\n"
                     "    (funcall body)))\n")
             (goto-char (point-min))
             (let ((edebug-all-defs t)) (edebug-eval-top-level-form)))
           ,@body)
       (if saved-function (fset name saved-function) (fmakunbound name))
       (setplist name saved-plist))))

(ert-deftest jaunder-debug-edebug-preserves-lazy-field-syntax-and-native-value ()
  "The instrumented macro retains literal keys, lazy forms and native identity."
  (jaunder-debug-boundary--with-session
   (jaunder-debug-edebug--with-owner
    (let ((value (list "private native value")) (fields 0) (calls 0))
      (dolist (enabled '(nil t))
        (setq jaunder-debug enabled)
        (should (eq value (jaunder-debug-edebug--owner
                           (lambda () (cl-incf fields) "org")
                           (lambda () (cl-incf calls) value))))
        (unless enabled (should (zerop fields))
                (should (zerop jaunder--debug-id-counter))
                (should-not (get-buffer jaunder--debug-buffer-name))))
      (should (= fields 2))
      (should (= calls 2))
      (let ((text (jaunder-debug-boundary--text)))
        (jaunder-debug-boundary--assert-tree text '("publish.validate"))
        (should (string-match-p "phase=start format=org count=7" text))
        (should-not (string-match-p "private" text)))))))

(ert-deftest jaunder-debug-edebug-native-condition-and-invalid-enum-privacy ()
  "Instrumented field values remain contained; native conditions are exact."
  (dolist (kind '(error quit))
    (jaunder-debug-boundary--with-session
     (jaunder-debug-edebug--with-owner
      (let ((data (list "private native failure")) (fields 0))
        (setq jaunder-debug t)
        (let ((caught (condition-case err
                          (jaunder-debug-edebug--owner
                           (lambda () (cl-incf fields) "private unsafe format")
                           (lambda () (signal kind data)))
                        (error err) (quit err))))
          (should (eq (car caught) kind))
          (should (eq (cdr caught) data)))
        (should (= fields 2))
        (let ((text (jaunder-debug-boundary--text)))
          (jaunder-debug-boundary--assert-tree text '("publish.validate"))
          (should (string-match-p "format=unknown count=7" text))
          (should (string-match-p (concat "outcome=" (if (eq kind 'quit) "cancelled" "error")) text))
          (should-not (string-match-p "private" text))))))))

(ert-deftest jaunder-debug-edebug-deferred-fields-retain-their-own-coverage ()
  "An instrumented relay cannot steal or erase deferred expression counters."
  (let ((definitions nil) (old-hook edebug-new-definition-function))
    (let ((edebug-new-definition-function
           (lambda (name)
             (push name definitions)
             (when old-hook (funcall old-hook name)))))
      (jaunder-debug-boundary--with-session
       (jaunder-debug-edebug--with-owner
        (let* ((relay 'jaunder-debug-edebug--relay)
               (saved-function (and (fboundp relay) (symbol-function relay)))
               (saved-plist (copy-sequence (symbol-plist relay)))
               (original-begin (symbol-function 'jaunder--debug-begin))
               (jaunder-debug t))
          (unwind-protect
              (progn
                (with-temp-buffer
                  (emacs-lisp-mode)
                  (insert "(defun jaunder-debug-edebug--relay (thunk) (funcall thunk))")
                  (goto-char (point-min))
                  (let ((edebug-all-defs t)) (edebug-eval-top-level-form)))
                ;; The real core still owns setup; relay's Edebug entry models
                ;; the producer instrumenting the helper invoking field thunks.
                (cl-letf (((symbol-function 'jaunder--debug-begin)
                           (lambda (label thunk)
                             (jaunder-debug-edebug--relay
                              (lambda () (funcall original-begin label thunk))))))
                  (should (eq 'native (jaunder-debug-edebug--owner
                                       (lambda () "org") (lambda () 'native)))))
                ;; Owner, relay and both deferred value definitions must exist;
                ;; treating the fields as data would erase the latter two.
                (should (= (length definitions) 4))
                (dolist (definition definitions)
                  (let ((counts (get definition 'edebug-freq-count)))
                    (should (vectorp counts))
                    (should (> (length counts) 0))
                    (should (cl-every (lambda (count) (> count 0)) counts)))))
            (if saved-function (fset relay saved-function) (fmakunbound relay))
            (setplist relay saved-plist))))))))

(ert-deftest jaunder-debug-edebug-batch-projections-retain-their-own-coverage ()
  "Both actual batch macro projections keep independent deferred counters."
  (require 'jaunder-reconcile)
  (let ((definitions nil) (old-hook edebug-new-definition-function)
        (edebug-initial-mode 'go))
    (let ((edebug-new-definition-function
           (lambda (name)
             (push name definitions)
             (when old-hook (funcall old-hook name)))))
      (jaunder-debug-boundary--with-session
       (let* ((name 'jaunder-debug-edebug--batch-owner)
              (saved-function (and (fboundp name) (symbol-function name)))
              (saved-plist (copy-sequence (symbol-plist name)))
              (jaunder-debug t))
         (unwind-protect
             (with-temp-buffer
               (emacs-lisp-mode)
               (insert "(defun jaunder-debug-edebug--batch-owner (action buffer)\n"
                       "  (jaunder--with-reconcile-batch-debug action buffer\n"
                       "    (list :status 'completed)))\n")
               (goto-char (point-min))
               (let ((edebug-all-defs t)) (edebug-eval-top-level-form))
               (should (equal '(:status completed)
                              (jaunder-debug-edebug--batch-owner 'pull (current-buffer))))
               (should (= (length definitions) 3))
               (dolist (definition definitions)
                 (let ((counts (get definition 'edebug-freq-count)))
                   (should (vectorp counts))
                   (should (> (length counts) 0))
                   (should (cl-every (lambda (count) (> count 0)) counts)))))
           (if saved-function (fset name saved-function) (fmakunbound name))
           (setplist name saved-plist)))))))

(provide 'jaunder-debug-edebug-test)
;;; jaunder-debug-edebug-test.el ends here
