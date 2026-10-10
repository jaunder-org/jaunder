;;; jaunder-reconcile-operation-test.el --- Owned read proof -*- lexical-binding: t; -*-

;; Copyright (C) 2026 Jaunder contributors

;;; Commentary:
;; Scalar consumers exercise discovery lifetime without inspecting private state.

;;; Code:

(require 'ert)
(require 'jaunder-reconcile-operation)

(ert-deftest jaunder-operation-captures-scope-before-first-read ()
  "Root and User drift fail before acquisition, even while evidence is lazy."
  (let ((jaunder--active-blog '(:base-url "https://one.test" :username "one"))
        (reads 0))
    (cl-letf (((symbol-function 'jaunder--fetch-collection-members)
               (lambda () (setq reads (1+ reads)) nil)))
      (jaunder--call-with-reconcile-operation
       "/one" "https://one.test" "one"
       (lambda ()
         (should-error (jaunder--operation-remote-members "/other"))
         (let ((jaunder--active-blog '(:base-url "https://two.test" :username "one")))
           (should-error (jaunder--operation-remote-members "/one")))
         (let ((jaunder--active-blog '(:base-url "https://one.test" :username "two")))
           (should-error (jaunder--operation-remote-members "/one")))
         (should (= reads 0))
         (should-not (jaunder--operation-remote-members "/one"))
         (should-not (jaunder--operation-remote-members "/one"))
         (should (= reads 1))))
      (should-not (jaunder--operation-active-p)))))

(ert-deftest jaunder-operation-retains-condition-and-retries-only-in-new-scope ()
  "Original condition data survive repeated consumers; new operations may retry."
  (let ((jaunder--active-blog '(:base-url "https://one.test" :username "one"))
        (reads 0))
    (cl-letf (((symbol-function 'jaunder--fetch-collection-members)
               (lambda () (setq reads (1+ reads)) (signal 'file-error '("offline" "path")))))
      (dotimes (operation 2)
        (jaunder--call-with-reconcile-operation
         "/one" "https://one.test" "one"
         (lambda ()
           (dotimes (_ 2)
             (condition-case err
                 (progn (jaunder--operation-remote-members "/one") (ert-fail "must fail"))
               (file-error (should (equal err '(file-error "offline" "path"))))))))
        (should (= reads (1+ operation)))))))

(provide 'jaunder-reconcile-operation-test)
;;; jaunder-reconcile-operation-test.el ends here
