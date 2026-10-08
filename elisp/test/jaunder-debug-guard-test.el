;;; jaunder-debug-guard-test.el --- Diagnostic setup and rewrite guards -*- lexical-binding: t; -*-

;;; Commentary:
;; Invalid setup and a damaged retained event are ancillary diagnostic faults.
;; Their guards must preserve native work, ancestor fields and bounded evidence.

;;; Code:
(require 'ert)
(require 'cl-lib)
(require 'jaunder-debug)
(load (expand-file-name "jaunder-debug-boundary-fixture.el"
                        (file-name-directory (or load-file-name buffer-file-name))) nil t)

(ert-deftest jaunder-debug-invalid-initial-fields-preserve-body-and-ancestor ()
  "Rejected initial fields do not skip native work or target an ancestor."
  (jaunder-debug-boundary--with-session
   (let ((jaunder-debug t) (native (list "private native value")) (warnings 0))
     (cl-letf (((symbol-function 'display-warning)
                (lambda (type message &rest _)
                  (should (eq type 'jaunder))
                  (should (equal message "jaunder: diagnostic output unavailable"))
                  (cl-incf warnings))))
       (jaunder--with-debug-operation "report.open" (count 1)
                                      (should (eq native
                                                  (jaunder--with-debug-operation "publish.validate"
                                                                                 (count "private invalid count")
                                                                                 (jaunder--debug-fields count 9)
                                                                                 native)))))
     (should (= warnings 1))
     (let ((text (jaunder-debug-boundary--text)))
       (jaunder-debug-boundary--assert-tree text '("report.open"))
       (should (string-match-p "count=1" text))
       (should-not (string-match-p "count=9\\|private\\|publish.validate" text))))))

(ert-deftest jaunder-debug-oversize-terminal-rewrite-is-contained ()
  "A damaged retained line cannot grow past the event bound during cancellation."
  (jaunder-debug-boundary--with-session
   (let* ((jaunder-debug t) (warnings 0)
          (state (jaunder--make-debug-span :label "report.open" :id "debug-2"))
          (line "at=2026-10-07T12:00:00.000Z correlation=debug-1 span=debug-2 label=report.open phase=end elapsed-ms=0 outcome=success"))
     ;; Model a formatter/sink fault at the supported maximum. Changing success
     ;; to cancelled would add two bytes; the retained evidence must stay put.
     (setq line (concat line (make-string (- 1024 (string-bytes line)) ?\s)))
     (with-current-buffer (jaunder--debug-buffer)
       (let ((inhibit-read-only t)) (insert line "\n")))
     (cl-letf (((symbol-function 'display-warning)
                (lambda (type message &rest _)
                  (should (eq type 'jaunder))
                  (should (equal message "jaunder: diagnostic output unavailable"))
                  (cl-incf warnings))))
       (should-not (jaunder--debug-safe
                    (lambda () (jaunder--debug-cancel-terminal state)))))
     (should (= warnings 1))
     (should (equal (concat line "\n") (jaunder-debug-boundary--text))))))

(provide 'jaunder-debug-guard-test)
;;; jaunder-debug-guard-test.el ends here
