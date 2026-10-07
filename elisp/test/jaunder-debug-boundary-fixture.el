;;; jaunder-debug-boundary-fixture.el --- Shared diagnostic boundary fixtures -*- lexical-binding: t; -*-

;;; Commentary:
;; Client operation proofs share one isolated diagnostic session and exact
;; event-label reader.  Sessions restore all
;; global diagnostic state and dispose only of their owned evidence buffer.

;;; Code:

(require 'cl-lib)
(require 'ert)
(require 'rx)
(require 'jaunder-debug)

(defmacro jaunder-debug-boundary--with-session (&rest body)
  "Run BODY with isolated diagnostics, initially disabled."
  (declare (indent 0) (debug t))
  `(let ((jaunder-debug nil)
         (jaunder--debug-buffer-name " *Jaunder boundary diagnostic tests*")
         (jaunder--debug-id-counter 0)
         (jaunder--debug-event-count 0)
         (jaunder--debug-discarded 0)
         (jaunder--debug-operation-stack nil))
     (unwind-protect (progn ,@body)
       (when-let* ((buffer (get-buffer jaunder--debug-buffer-name)))
         (kill-buffer buffer)))))

(defun jaunder-debug-boundary--text ()
  "Return the current session's retained diagnostic text."
  (with-current-buffer jaunder--debug-buffer-name (buffer-string)))

(defun jaunder-debug-boundary--label-count (label text)
  "Count complete events with exact LABEL in TEXT."
  (cl-count-if (lambda (line)
                 (string-match-p (concat " label=" (regexp-quote label) " phase=") line))
               (split-string text "\n" t)))

(defun jaunder-debug-boundary--assert-tree (text roots)
  "Assert paired spans and valid parent/correlation links in TEXT with ROOTS."
  (let ((spans (make-hash-table :test #'equal)) (root-count 0))
    (dolist (line (split-string text "\n" t))
      (should (string-match
               (rx " correlation=" (group (+ (not space)))
                   " span=" (group (+ (not space)))
                   " label=" (group (+ (not space)))
                   " phase=" (group (or "start" "end"))
                   (optional " parent=" (group (+ (not space))))) line))
      (let ((id (match-string 2 line))
            (event (list (match-string 4 line) (match-string 3 line)
                         (match-string 1 line) (match-string 5 line))))
        (puthash id (append (gethash id spans) (list event)) spans)))
    (maphash
     (lambda (_ events)
       (should (= 2 (length events)))
       (should (equal (mapcar #'car events) '("start" "end")))
       (should (equal (cdar events) (cdadr events)))
       (let* ((event (car events)) (parent (nth 3 event)))
         (if parent
             (let ((ancestor (car (gethash parent spans))))
               (should ancestor)
               (should (equal (nth 2 event) (nth 2 ancestor))))
           (setq root-count (1+ root-count)))))
     spans)
    (should (= roots root-count))))

(provide 'jaunder-debug-boundary-fixture)
;;; jaunder-debug-boundary-fixture.el ends here
