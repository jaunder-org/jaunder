;;; jaunder-coverage.el --- combined Undercover coverage producer -*- lexical-binding: t; -*-

;;; Commentary:
;; This library owns the producer half of the Elisp coverage boundary.  It
;; discovers only flat production modules before loading `jaunder', installs
;; Undercover's file handlers first, and retains controlled-failure artifacts so
;; the Rust consumer—not the VM wrapper—makes the coverage verdict.

;;; Code:

(require 'cl-lib)
(require 'edebug)
(require 'ert)
(require 'json)
(require 'undercover)

(defconst jaunder-coverage--schema "elisp-coverage-v1"
  "The producer schema consumed by `xtask::elisp_coverage'.")

(defun jaunder-coverage--source-files (root)
  "Return sorted flat production source files below ROOT.

Only `elisp/*.el' belongs to the production denominator; tests, runners,
vendors, generated trees, and bytecode are structurally outside it."
  (sort (directory-files root t "\\.el\\'" t) #'string<))

(defun jaunder-coverage--form-kind (form)
  "Return FORM's source-reader kind, matching the consumer's contract."
  (if (and (consp form) (symbolp (car form)))
      (symbol-name (car form))
    "atom"))

(defun jaunder-coverage--read-forms (file)
  "Read FILE's top-level forms without evaluating them.

The returned plists preserve source order and opening lines, so unsupported
Edebug forms remain explicit census entries rather than silent exclusions."
  (with-temp-buffer
    (insert-file-contents file)
    (emacs-lisp-mode)
    (goto-char (point-min))
    (let (forms)
      (condition-case err
          (while (progn
                   (forward-comment (point-max))
                   (not (eobp)))
            (let ((start-line (line-number-at-pos))
                  (form (read (current-buffer))))
              (push (list :start_line start-line
                          :kind (jaunder-coverage--form-kind form))
                    forms)))
        (error (error "coverage census could not read %s: %s"
                      file (error-message-string err))))
      (nreverse forms))))

(defun jaunder-coverage--edebug-lines (file)
  "Return FILE's unique Undercover/Edebug stop lines.

Undercover records stop offsets on symbols represented in `edebug-form-data'.
Reading those offsets directly preserves executable positions before LCOV folds
multiple stops that share a source line."
  (with-current-buffer (find-file-noselect file)
    (let (lines)
      (dolist (entry edebug-form-data)
        (let* ((symbol (car entry))
               (edebug-data (get symbol 'edebug))
               (start (car edebug-data)))
          (when (and edebug-data start)
            (dolist (point (append (nth 2 edebug-data) nil))
              (push (line-number-at-pos (+ start point)) lines)))))
      (delete-dups (sort lines #'<)))))

(defun jaunder-coverage--point (line kind)
  "Return the status JSON representation for LINE and KIND."
  `((line . ,line) (kind . ,kind)))

(defun jaunder-coverage--census-file (file)
  "Build FILE's source form census from its currently installed Edebug data.

A form with no Edebug stop gets exactly one synthetic opening-line point."
  (let* ((forms (jaunder-coverage--read-forms file))
         (stops (jaunder-coverage--edebug-lines file))
         (starts (mapcar (lambda (form) (plist-get form :start_line)) forms)))
    (vconcat
     (mapcar
      (lambda (form)
        (let* ((start (plist-get form :start_line))
               (next (cadr (member start starts)))
               (points (cl-remove-if-not
                        (lambda (line) (and (>= line start)
                                            (or (null next) (< line next))))
                        stops)))
          `((start_line . ,start)
            (kind . ,(plist-get form :kind))
            (points . ,(if points
                           (vconcat
                            (mapcar (lambda (line)
                                      (jaunder-coverage--point line "ordinary"))
                                    points))
                         (vector (jaunder-coverage--point start "synthetic")))))))
      forms))))

(defun jaunder-coverage--module-census (root)
  "Return the producer census for every production module beneath ROOT."
  (vconcat
   (mapcar
    (lambda (file)
      `((path . ,(concat "elisp/" (file-relative-name file root)))
        (forms . ,(jaunder-coverage--census-file file))))
    (jaunder-coverage--source-files root))))

(defun jaunder-coverage--write-json (path value)
  "Write VALUE as deterministic JSON at PATH."
  (with-temp-file path
    (insert (json-serialize value))
    (insert "\n")))

(defun jaunder-coverage--write-controlled-artifacts (output outcome modules)
  "Ensure OUTPUT has all fixed artifacts for controlled OUTCOME and MODULES.

Placeholders are intentional only until report finalization overwrites them;
this guarantees a later controlled error cannot erase the diagnostic contract."
  (make-directory output t)
  (dolist (name '("lcov.info" "summary.txt"))
    (let ((path (expand-file-name name output)))
      (when (or (not (file-exists-p path))
                (zerop (file-attribute-size (file-attributes path))))
        (with-temp-file path
          (insert (format "Elisp coverage producer outcome: %s\n" outcome))))))
  (jaunder-coverage--write-json
   (expand-file-name "status.json" output)
   `((schema . ,jaunder-coverage--schema)
     (outcome . ,outcome)
     (modules . ,modules))))

(defun jaunder-coverage--load-production (root)
  "Install Undercover before source loading, then eagerly load all modules."
  (let ((files (jaunder-coverage--source-files root)))
    ;; Nix invokes Emacs outside a CI provider; force only this pinned local
    ;; engine so no package lookup or external-service detection participates.
    (setq undercover-force-coverage t)
    (undercover--set-edebug-handlers)
    (undercover--edebug-files files)
    (add-to-list 'load-path root)
    ;; `jaunder' establishes the package's normal dependency order.  Requiring
    ;; every discovered feature afterward makes newly unreferenced production
    ;; modules visible without evaluating any module before instrumentation.
    (require 'jaunder)
    (dolist (file files)
      (require (intern (file-name-base file))))))

(defun jaunder-coverage--load-tests (directory suffix)
  "Load each test source in DIRECTORY whose filename ends in SUFFIX."
  (dolist (file (sort (directory-files directory t suffix t) #'string<))
    (load file nil t)))

(defun jaunder-coverage--unexpected-p (stats)
  "Return non-nil when ERT STATS contains an unexpected result."
  (not (zerop (ert-stats-completed-unexpected stats))))

(defun jaunder-coverage--run-populations (root)
  "Run pure then live ERT populations and return whether either failed.

The shared live server starts once after the pure run and its teardown lives in
`unwind-protect', so a pure failure cannot suppress live observations and a live
failure cannot leak the server."
  (let ((test-dir (expand-file-name "test" root))
        failed)
    (add-to-list 'load-path test-dir)
    (jaunder-coverage--load-tests test-dir "-test\\.el\\'")
    (setq failed (jaunder-coverage--unexpected-p (ert-run-tests-batch)))
    (require 'jaunder-integration-helper)
    (jaunder-coverage--load-tests test-dir "-integration\\.el\\'")
    (let ((state (jaunder-test--server-up-retrying)))
      (jaunder-test--set-globals state)
      (unwind-protect
          (setq failed (or (jaunder-coverage--unexpected-p
                            (ert-run-tests-batch :new))
                           failed))
        (jaunder-test--server-down state)))
    failed))

(defun jaunder-coverage--relativize-lcov (path root)
  "Rewrite Undercover's store paths in PATH to consumer-stable source names."
  (let ((known (mapcar #'file-truename (jaunder-coverage--source-files root))))
    (with-temp-buffer
      (insert-file-contents path)
      (goto-char (point-min))
      (while (re-search-forward "^SF:\\(.+\\)$" nil t)
        (let ((source (file-truename (match-string 1))))
          (unless (member source known)
            (error "LCOV contains non-production source %s" source))
          (replace-match (concat "SF:elisp/" (file-name-nondirectory source)) t t)))
      (write-region nil nil path nil 'silent))))

(defun jaunder-coverage--write-summary (lcov-path summary-path)
  "Write a readable, zero-safe line summary from LCOV-PATH to SUMMARY-PATH."
  (let (current rows (relevant 0) (covered 0))
    (cl-labels
     ((finish-record
        ()
        (unless current
          (error "LCOV end_of_record without SF"))
        (push (format "%s: relevant %d, covered %d, missed %d\n"
                      current relevant covered (- relevant covered))
              rows)
        (setq current nil
              relevant 0
              covered 0)))
     (dolist (line (split-string
                    (with-temp-buffer
                      (insert-file-contents lcov-path)
                      (buffer-string))
                    "\n"))
       (cond
        ((string-prefix-p "SF:" line)
         (when current
           (error "LCOV nested SF record"))
         (setq current (substring line 3)))
        ((string-match "\\`DA:[0-9]+,\\([0-9]+\\)\\'" line)
         (unless current
           (error "LCOV DA without SF"))
         (setq relevant (1+ relevant))
         (when (> (string-to-number (match-string 1 line)) 0)
           (setq covered (1+ covered))))
        ((equal line "end_of_record")
         (finish-record))))
     (when current
       (error "LCOV missing end_of_record")))
    (with-temp-file summary-path
      (insert "== Emacs Protocol Client line coverage ==\n")
      (mapc #'insert (nreverse rows)))))

(defun jaunder-coverage--write-reports (output root)
  "Finalize Undercover's combined LCOV and text reports in OUTPUT for ROOT."
  (let ((undercover--report-on-kill nil)
        (undercover--send-report nil)
        (undercover--merge-report nil)
        (undercover--report-file-path (expand-file-name "lcov.info" output)))
    (undercover-report 'lcov))
  (jaunder-coverage--relativize-lcov (expand-file-name "lcov.info" output) root)
  (jaunder-coverage--write-summary
   (expand-file-name "lcov.info" output)
   (expand-file-name "summary.txt" output)))

(defun jaunder-coverage-run (&optional root output)
  "Run the hermetic combined producer and return its controlled outcome.

ROOT defaults to the parent of this scripts directory; OUTPUT is supplied by
`run-coverage.el'.  Unexpected setup/instrumentation and report errors remain
controlled producer outcomes with retained artifacts."
  (let* ((root (or root (file-name-directory
                         (directory-file-name
                          (file-name-directory (or load-file-name buffer-file-name))))))
         (output (or output (getenv "JAUNDER_ELISP_COVERAGE_DIR")))
         (modules [])
         (outcome "success"))
    (unless output
      (error "JAUNDER_ELISP_COVERAGE_DIR is required"))
    (condition-case err
        (progn
          (jaunder-coverage--load-production root)
          (setq modules (jaunder-coverage--module-census root)))
      (error
       (setq outcome "instrumentation-failure")
       (message "coverage instrumentation error: %s" (error-message-string err))))
    ;; A failed census still has a controlled artifact boundary.  In the normal
    ;; path this happens before either ERT population begins.
    (jaunder-coverage--write-controlled-artifacts output outcome modules)
    (when (equal outcome "success")
      (condition-case err
          (when (jaunder-coverage--run-populations root)
            (setq outcome "ert-failure"))
        (error
         (setq outcome "ert-failure")
         (message "coverage test error: %s" (error-message-string err))))
      (condition-case report-error
          (jaunder-coverage--write-reports output root)
        (error
         (setq outcome "invalid-report")
         (message "coverage report error: %s" (error-message-string report-error)))))
    (jaunder-coverage--write-controlled-artifacts output outcome modules)
    outcome))

(provide 'jaunder-coverage)
;;; jaunder-coverage.el ends here
