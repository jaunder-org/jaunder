;;; jaunder-integration-helper.el --- live-server harness for jaunder ERT -*- lexical-binding: t; -*-

;;; Commentary:
;; Boots a real jaunder server in a tempdir, provisions a user + app password
;; (before serving, so there is no concurrent sqlite writer), and runs a body
;; against it.  See ADR-0035.  Not loaded by the pure suite (run-tests.el globs
;; -test.el only).

;;; Code:

(require 'jaunder)
(require 'json)
(require 'url)
(require 'auth-source)
(require 'subr-x)

(defvar jaunder-test-app-password nil
  "Raw app-password token, bound inside `jaunder-test--with-live-server'.")

(defun jaunder-test--binary ()
  "Locate the jaunder binary or signal an error (never silently skip)."
  (or (getenv "JAUNDER_TEST_BINARY")
      (executable-find "jaunder")
      (error "jaunder-test: set JAUNDER_TEST_BINARY or put `jaunder' on PATH")))

(defun jaunder-test--run-cli (bin &rest args)
  "Run BIN with ARGS synchronously; return clean stdout.
stderr is captured separately (so it never pollutes stdout, e.g. the minted
token) and surfaced only on a non-zero exit."
  (let ((errfile (make-temp-file "jaunder-cli-err")))
    (unwind-protect
        (with-temp-buffer
          (let ((code (apply #'call-process bin nil (list t errfile) nil args)))
            (unless (eq code 0)
              (error "jaunder-test: %s %S exited %s: %s%s" bin args code
                     (buffer-string)
                     (with-temp-buffer
                       (insert-file-contents errfile)
                       (buffer-string))))
            (buffer-string)))
      (delete-file errfile))))

(defun jaunder-test--read-runtime-file (path)
  "Return (IP . PORT) from runtime file PATH, or nil if absent/unparseable."
  (when (and (file-exists-p path)
             (> (file-attribute-size (file-attributes path)) 0))
    (ignore-errors
      (let* ((json-object-type 'alist)
             (data (json-read-file path)))
        (cons (alist-get 'ip data) (alist-get 'port data))))))

(defun jaunder-test--wait (predicate what)
  "Poll PREDICATE up to 100 times every 0.1s; return its value or error WHAT."
  (or (catch 'done
        (dotimes (_ 100)
          (let ((v (funcall predicate)))
            (when v (throw 'done v)))
          (sleep-for 0.1))
        nil)
      (error "jaunder-test: timed out waiting for %s" what)))

(defun jaunder-test--http-reachable-p (url)
  "Return non-nil if a GET of URL yields any HTTP response."
  (ignore-errors
    (let ((buf (url-retrieve-synchronously url t t 5)))
      (when buf (kill-buffer buf) t))))

(defmacro jaunder-test--with-live-server (&rest body)
  "Boot a jaunder server in a tempdir, provision creds, then run BODY.
Bound in BODY: `jaunder-base-url', `jaunder-username',
`jaunder-test-app-password', and `auth-sources' (a temp netrc with the token)."
  (declare (indent 0) (debug t))
  `(let* ((bin (jaunder-test--binary))
          (tmp (make-temp-file "jaunder-it-" t))
          (storage (expand-file-name "data" tmp))
          (db (concat "sqlite:" (expand-file-name "jaunder.db" tmp)))
          (rf (expand-file-name "runtime.json" tmp))
          (stderr (generate-new-buffer " *jaunder-server*"))
          (proc nil))
     (unwind-protect
         (progn
           ;; `init` creates the storage dir itself; don't pre-create it.
           ;; Provision before serving — no concurrent sqlite writer.
           (jaunder-test--run-cli bin "init" "--db" db "--storage-path" storage)
           (jaunder-test--run-cli bin "user-create" "--db" db "--storage-path" storage
                                  "--username" "alice" "--password" "password123")
           (let ((token (string-trim
                         (jaunder-test--run-cli bin "app-password-create"
                                                "--db" db "--storage-path" storage
                                                "--username" "alice" "--label" "ert"))))
             (setq proc (make-process
                         :name "jaunder-server" :buffer stderr :noquery t
                         :command (list bin "serve"
                                        "--bind" "127.0.0.1:0"
                                        "--db" db "--storage-path" storage
                                        "--runtime-file" rf
                                        "--environment" "dev")))
             (let* ((addr (jaunder-test--wait
                           (lambda () (jaunder-test--read-runtime-file rf)) "runtime.json"))
                    (jaunder-base-url (format "http://%s:%s" (car addr) (cdr addr)))
                    (jaunder-username "alice")
                    (jaunder-test-app-password token)
                    (authinfo (expand-file-name "authinfo" tmp))
                    (auth-source-do-cache nil)
                    (auth-sources (list authinfo)))
               (jaunder-test--wait
                (lambda () (jaunder-test--http-reachable-p (concat jaunder-base-url "/")))
                "server readiness")
               (with-temp-file authinfo
                 (insert (format "machine %s login %s password %s\n"
                                 (car addr) jaunder-username jaunder-test-app-password)))
               ,@body)))
       (when (process-live-p proc) (delete-process proc))
       (when (buffer-live-p stderr) (kill-buffer stderr))
       (delete-directory tmp t))))

(provide 'jaunder-integration-helper)
;;; jaunder-integration-helper.el ends here
