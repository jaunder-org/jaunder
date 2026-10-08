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
        (should (equal (condition-case err
                           (jaunder-debug-edebug--owner
                            (lambda () (cl-incf fields) "private unsafe format")
                            (lambda () (signal kind data)))
                         (error err) (quit err))
                       (cons kind data)))
        (should (= fields 2))
        (let ((text (jaunder-debug-boundary--text)))
          (jaunder-debug-boundary--assert-tree text '("publish.validate"))
          (should (string-match-p "format=unknown count=7" text))
          (should (string-match-p (concat "outcome=" (if (eq kind 'quit) "cancelled" "error")) text))
          (should-not (string-match-p "private" text))))))))

(provide 'jaunder-debug-edebug-test)
;;; jaunder-debug-edebug-test.el ends here
