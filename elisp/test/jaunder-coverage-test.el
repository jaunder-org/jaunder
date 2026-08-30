;;; jaunder-coverage-test.el --- coverage producer contracts -*- lexical-binding: t; -*-

;;; Commentary:
;; Focused contract tests for the coverage producer's repository-local census and
;; controlled artifact boundary.  They deliberately use temporary flat source
;; trees so test helpers and runners can never enter the production population.

;;; Code:

(require 'ert)
(require 'json)

(let ((scripts-dir (expand-file-name "../scripts"
                                     (file-name-directory load-file-name))))
  (load (expand-file-name "jaunder-coverage.el" scripts-dir) nil t))

(defun jaunder-coverage-test--with-tree (files body)
  "Create FILES in a temporary root and call BODY with that root."
  (let ((root (make-temp-file "jaunder-coverage-" t)))
    (unwind-protect
        (progn
          (dolist (file files)
            (let ((path (expand-file-name (car file) root)))
              (make-directory (file-name-directory path) t)
              (with-temp-file path
                (insert (cdr file)))))
          (funcall body root))
      (delete-directory root t))))

(ert-deftest jaunder-coverage-census-is-flat-production-source-only ()
  "The census includes only flat production modules, never test infrastructure."
  (jaunder-coverage-test--with-tree
   '(("jaunder-alpha.el" . "(defun alpha () 1)\n")
     ("jaunder-beta.el" . "(provide 'jaunder-beta)\n")
     ("test/jaunder-alpha-test.el" . "(ert-deftest ignored () t)\n")
     ("scripts/run-coverage.el" . "(message \"ignored\")\n")
     ("vendor/undercover.el" . "(provide 'undercover)\n")
     ("generated.elc" . "bytecode"))
   (lambda (root)
     (should (equal (jaunder-coverage--source-files root)
                    (list (expand-file-name "jaunder-alpha.el" root)
                          (expand-file-name "jaunder-beta.el" root)))))))

(ert-deftest jaunder-coverage-census-keeps-macro-struct-and-declaration-forms ()
  "Every top-level form remains visible even when Edebug yields no stop points."
  (jaunder-coverage-test--with-tree
   '(("jaunder-fixture.el" .
      "(defmacro jaunder--with-blog (file &rest body)\n  (declare (indent 1) (debug t))\n  `(progn ,file ,@body))\n\n(cl-defstruct fixture value)\n(declare-function fixture-call \"fixture\")\n(provide 'jaunder-fixture)\n"))
   (lambda (root)
     (let* ((file (expand-file-name "jaunder-fixture.el" root))
            (forms (jaunder-coverage--read-forms file))
            (census (jaunder-coverage--census-file file)))
       (should (equal (mapcar (lambda (form) (plist-get form :kind)) forms)
                      '("defmacro" "cl-defstruct" "declare-function" "provide")))
       (should (equal (mapcar (lambda (form) (alist-get 'kind form)) census)
                      '("defmacro" "cl-defstruct" "declare-function" "provide")))
       (mapc (lambda (form)
               (should (equal (alist-get 'points form)
                              (vector `((line . ,(alist-get 'start_line form))
                                        (kind . "synthetic"))))))
             census)))))

(ert-deftest jaunder-coverage-controlled-outcomes-preserve-fixed-artifacts ()
  "Every controlled outcome writes the full artifact set for the consumer."
  (let ((output (make-temp-file "jaunder-coverage-output-" t))
        (modules (vector '((path . "elisp/client.el") (forms . [])))))
    (unwind-protect
        (dolist (outcome '("success" "ert-failure" "instrumentation-failure" "invalid-report"))
          (jaunder-coverage--write-controlled-artifacts output outcome modules)
          (dolist (name '("lcov.info" "summary.txt" "status.json"))
            (should (file-exists-p (expand-file-name name output))))
          (let* ((json-object-type 'alist)
                 (status (json-read-file (expand-file-name "status.json" output))))
            (should (equal (alist-get 'schema status) "elisp-coverage-v1"))
            (should (equal (alist-get 'outcome status) outcome))
            (should (equal (alist-get 'modules status) modules))))
      (delete-directory output t))))



(ert-deftest jaunder-coverage-missing-output-is-uncontrolled ()
  "A wrapper/configuration failure must escape instead of becoming status JSON."
  (let ((process-environment (copy-sequence process-environment)))
    (setenv "JAUNDER_ELISP_COVERAGE_DIR" nil)
    (should-error (jaunder-coverage-run default-directory nil)
                  :type 'error)))
(ert-deftest jaunder-coverage-summary-keeps-zero-point-modules ()
  "A census-visible zero-point module must not make readable reporting overflow."
  (let ((lcov (make-temp-file "jaunder-coverage-" nil ".info"
                              "SF:elisp/jaunder-empty.el\nend_of_record\n"))
        (summary (make-temp-file "jaunder-coverage-" nil ".txt")))
    (unwind-protect
        (progn
          (jaunder-coverage--write-summary lcov summary)
          (with-temp-buffer
            (insert-file-contents summary)
            (should (search-forward
                     "elisp/jaunder-empty.el: relevant 0, covered 0, missed 0"
                     nil t))))
      (delete-file lcov)
      (delete-file summary))))
(ert-deftest jaunder-coverage-runs-both-populations-and-always-tears-down ()
  "A pure failure remains diagnostic while the live population still tears down."
  (let* ((root (file-name-directory (locate-library "jaunder")))
         (events nil)
         (runs 0))
    (add-to-list 'load-path (expand-file-name "test" root))
    (require 'jaunder-integration-helper)
    (cl-letf (((symbol-function 'jaunder-coverage--load-tests)
               (lambda (_directory suffix) (push suffix events)))
              ((symbol-function 'ert-run-tests-batch)
               (lambda (&optional selector)
                 (setq runs (1+ runs))
                 (push (list "ert" selector) events)
                 runs))
              ((symbol-function 'jaunder-coverage--unexpected-p)
               (lambda (stats) (= stats 1)))
              ((symbol-function 'jaunder-test--server-up-retrying)
               (lambda () (push "up" events) 'state))
              ((symbol-function 'jaunder-test--set-globals)
               (lambda (_state) (push "bind" events)))
              ((symbol-function 'jaunder-test--server-down)
               (lambda (_state) (push "down" events))))
             (should (jaunder-coverage--run-populations root)))
    (should (equal (nreverse events)
                   '("-test\\.el\\'" ("ert" nil) "-integration\\.el\\'"
                     "up" "bind" ("ert" :new) "down")))))


(ert-deftest jaunder-coverage-installs-undercover-before-loading-production ()
  "Undercover's handlers and files precede every production `require'."
  (jaunder-coverage-test--with-tree
   '(("jaunder.el" . "(provide 'jaunder)\n")
     ("jaunder-alpha.el" . "(provide 'jaunder-alpha)\n"))
   (lambda (root)
     (let ((events nil))
       (cl-letf (((symbol-function 'undercover--set-edebug-handlers)
                  (lambda () (push "handlers" events)))
                 ((symbol-function 'undercover--edebug-files)
                  (lambda (_files) (push "files" events)))
                 ((symbol-function 'require)
                  (lambda (feature &rest _args)
                    (push (list "require" feature) events))))
                (jaunder-coverage--load-production root))
       (should (equal (nreverse events)
                      '("handlers" "files" ("require" jaunder)
                        ("require" jaunder-alpha) ("require" jaunder))))))))
;;; jaunder-coverage-test.el ends here
