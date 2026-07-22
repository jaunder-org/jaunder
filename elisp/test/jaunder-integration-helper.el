;;; jaunder-integration-helper.el --- live-server harness for jaunder ERT -*- lexical-binding: t; -*-

;;; Commentary:
;; Boots a real jaunder server in a tempdir, provisions a user + app password
;; (before serving, so there is no concurrent sqlite writer), and runs a body
;; against it.  See ADR-0035.  Not loaded by the pure suite (run-tests.el globs
;; -test.el only).

;;; Code:

(require 'jaunder)
(require 'json)
(require 'plz)
(require 'auth-source)
(require 'subr-x)

(defvar jaunder-test-app-password nil
  "Raw app-password token, bound inside `jaunder-test--with-live-server'.")

(defvar jaunder-test-base-url nil
  "Live server base URL, bound inside `jaunder-test--with-live-server'.")

(defvar jaunder-test-username nil
  "Provisioned username, bound inside `jaunder-test--with-live-server'.")

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
  "Return non-nil if a GET of URL yields any HTTP response (any status)."
  (condition-case nil
      (progn (plz 'get url :as 'response :connect-timeout 5) t)
    ;; An HTTP error status still means the server answered = reachable.
    (plz-http-error t)
    ;; A curl/connection error means it is not up yet.
    (error nil)))

(defun jaunder-test--authed-200-p (url user password)
  "Return non-nil if an authenticated GET of URL returns HTTP 200.
Uses `plz' (curl) — the same transport as `jaunder--http-request' — so the
readiness gate never rides `url.el', whose load-bearing auth-header handling
is exactly what this suite exists to avoid (ADR-0038)."
  (condition-case nil
      (eq 200 (plz-response-status
               (plz 'get url
                    :headers (list (jaunder--basic-auth-header user password))
                    :as 'response
                    :connect-timeout 5)))
    ;; A non-200 status (plz signals on 4xx/5xx) or a connection error both
    ;; mean "not ready yet" — the caller retries.
    (error nil)))

(defmacro jaunder-test--with-live-server (&rest body)
  "Boot a jaunder server in a tempdir, provision creds, then run BODY.
Bound in BODY: `jaunder-test-base-url', `jaunder-test-username',
`jaunder-test-app-password', `jaunder--active-blog' (so a low-level transport
call in BODY has request context), and `auth-sources' (a temp netrc)."
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
                    (base-url (format "http://%s:%s" (car addr) (cdr addr)))
                    (username "alice")
                    (jaunder-test-base-url base-url)
                    (jaunder-test-username username)
                    (jaunder-test-app-password token)
                    (jaunder--active-blog (list :base-url base-url :username username))
                    (authinfo (expand-file-name "authinfo" tmp))
                    (auth-source-do-cache nil)
                    (auth-sources (list authinfo)))
               (jaunder-test--wait
                (lambda () (jaunder-test--http-reachable-p (concat base-url "/")))
                "server readiness")
               ;; #560: feeds/AtomPub now require `site.base_url` to compose absolute
               ;; URLs, else they 500. Point it at this server's own (dynamic) address
               ;; so the atompub collection the auth-readiness poll below hits returns
               ;; 200, and emitted URLs (Location/edit links) resolve back to it.
               (jaunder-test--run-cli bin "site-config" "set"
                                      "--db" db "--storage-path" storage
                                      "site.base_url" base-url)
               ;; Auth readiness: an unauthed GET / can succeed before the
               ;; just-provisioned session is reliably usable by the serving
               ;; connection, so the first authed request can race to a 401.
               ;; Wait until an authed request actually returns 200.
               (jaunder-test--wait
                (lambda ()
                  (jaunder-test--authed-200-p
                   (jaunder--build-url base-url "atompub" username "posts")
                   username jaunder-test-app-password))
                "auth readiness")
               (with-temp-file authinfo
                 (insert (format "machine %s login %s password %s\n"
                                 (car addr) username jaunder-test-app-password)))
               ,@body)))
       (when (process-live-p proc) (delete-process proc))
       (when (buffer-live-p stderr) (kill-buffer stderr))
       (delete-directory tmp t))))

(provide 'jaunder-integration-helper)
;;; jaunder-integration-helper.el ends here
