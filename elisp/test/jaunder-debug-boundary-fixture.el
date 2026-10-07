;;; jaunder-debug-boundary-fixture.el --- Shared diagnostic boundary fixtures -*- lexical-binding: t; -*-

;;; Commentary:
;; Acquisition, transformation, and pull boundary proofs share one isolated
;; diagnostic session and exact event-label reader.  Sessions restore all
;; global diagnostic state and dispose only of their owned evidence buffer.

;;; Code:

(require 'cl-lib)
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

(provide 'jaunder-debug-boundary-fixture)
;;; jaunder-debug-boundary-fixture.el ends here
