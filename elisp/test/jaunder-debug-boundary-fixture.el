;;; jaunder-debug-boundary-fixture.el --- Shared diagnostic boundary fixtures -*- lexical-binding: t; -*-

;;; Commentary:
;; Client operation proofs share an isolated diagnostic session, exact
;; event-label reader and ordered parent-tree assertions.  Sessions restore
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
  "Return the current session's retained diagnostic text, or an empty string."
  (if-let* ((buffer (get-buffer jaunder--debug-buffer-name)))
      (with-current-buffer buffer (buffer-string))
    ""))

(defun jaunder-debug-boundary--label-count (label text)
  "Count complete events with exact LABEL in TEXT."
  (cl-count-if (lambda (line)
                 (string-match-p (concat " label=" (regexp-quote label) " phase=") line))
               (split-string text "\n" t)))

(defun jaunder-debug-boundary--assert-tree (text roots &optional edges)
  "Assert complete ordered spans in TEXT with ROOTS and required label EDGES.
ROOTS is a count or ordered root-label list.  EDGES maps child labels to their
required immediate parent labels.  Distinct roots require distinct correlations."
  (let ((spans (make-hash-table :test #'equal)) active root-labels correlations)
    (dolist (line (split-string text "\n" t))
      (should (string-match
               (rx " correlation=" (group (+ (not space)))
                   " span=" (group (+ (not space)))
                   " label=" (group (+ (not space)))
                   " phase=" (group (or "start" "end"))
                   (optional " parent=" (group (+ (not space))))) line))
      (let* ((id (match-string 2 line))
             (event (list (match-string 4 line) (match-string 3 line)
                          (match-string 1 line) (match-string 5 line)))
             (parent (nth 3 event))
             (previous (gethash id spans)))
        (if (equal (car event) "start")
            (progn
              (should-not previous)
              (should-not (equal parent id))
              ;; An already-open immediate parent plus a unique ID excludes
              ;; cycles, orphan/late children and fabricated overlapping roots.
              (should (equal parent (car active)))
              (when-let* ((edge (assoc (nth 1 event) edges)))
                (should (equal (cdr edge) (nth 1 (car (gethash parent spans))))))
              (if parent
                  (should (equal (nth 2 event) (nth 2 (car (gethash parent spans)))))
                (should-not (member (nth 2 event) correlations))
                (push (nth 2 event) correlations)
                (push (nth 1 event) root-labels))
              (push id active))
          (should (equal id (car active)))
          (should (= 1 (length previous)))
          (should (equal (cdar previous) (cdr event)))
          (pop active))
        (puthash id (append previous (list event)) spans)))
    (should-not active)
    (maphash (lambda (_ events) (should (= 2 (length events)))) spans)
    (if (integerp roots)
        (should (= roots (length root-labels)))
      (should (equal roots (nreverse root-labels))))))

(provide 'jaunder-debug-boundary-fixture)
;;; jaunder-debug-boundary-fixture.el ends here
