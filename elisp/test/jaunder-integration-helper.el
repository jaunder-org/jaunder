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
(require 'cl-lib)

(defconst jaunder-test--connect-timeout 2
  "Per-attempt `plz' connect timeout (seconds) for the readiness polls.  Short so
a hung connect on a loaded VM can't consume the wall-clock poll budget (#628).")

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

(defun jaunder-test--wait (predicate what &optional timeout)
  "Poll PREDICATE until non-nil or TIMEOUT seconds elapse; then error WHAT.
TIMEOUT defaults to $JAUNDER_TEST_READY_TIMEOUT (seconds) or 30.  The budget is
wall-clock, not an iteration count, so a slow per-attempt connect can't starve
the poll count on a loaded CI VM (#628)."
  (let* ((budget (or timeout
                     (let ((env (getenv "JAUNDER_TEST_READY_TIMEOUT")))
                       (if env (string-to-number env) 30))))
         (start (float-time))
         (deadline (+ start budget)))
    (catch 'done
      (while (< (float-time) deadline)
        (let ((v (funcall predicate)))
          (when v (throw 'done v)))
        (sleep-for 0.1))
      (error "jaunder-test: timed out (%.1fs) waiting for %s"
             (- (float-time) start) what))))

(defun jaunder-test--http-reachable-p (url)
  "Return non-nil if a GET of URL yields any HTTP response (any status)."
  (condition-case nil
      (progn (plz 'get url :as 'response :connect-timeout jaunder-test--connect-timeout) t)
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
                    :connect-timeout jaunder-test--connect-timeout)))
    ;; A non-200 status (plz signals on 4xx/5xx) or a connection error both
    ;; mean "not ready yet" — the caller retries.
    (error nil)))

(defun jaunder-test--server-up ()
  "Boot a jaunder server in a fresh tempdir, provision `alice' + an app-password,
serve, and wait through the three readiness gates.  Return a state plist
\(:proc :stderr :tmp :base-url :username :token :authinfo).  Provisioning runs
before `serve' (no concurrent sqlite writer); the caller must pair this with
`jaunder-test--server-down'.  On a partial boot, tears down and re-signals."
  (let* ((bin (jaunder-test--binary))
         (tmp (make-temp-file "jaunder-it-" t))
         (storage (expand-file-name "data" tmp))
         (db (concat "sqlite:" (expand-file-name "jaunder.db" tmp)))
         (rf (expand-file-name "runtime.json" tmp))
         (authinfo (expand-file-name "authinfo" tmp))
         (stderr (generate-new-buffer " *jaunder-server*"))
         (username "alice")
         (proc nil))
    (condition-case err
        (progn
          ;; `init` creates the storage dir itself; don't pre-create it.
          (jaunder-test--run-cli bin "init" "--db" db "--storage-path" storage)
          (jaunder-test--run-cli bin "user-create" "--db" db "--storage-path" storage
                                 "--username" username "--password" "password123")
          (let ((token (string-trim
                        (jaunder-test--run-cli bin "app-password-create"
                                               "--db" db "--storage-path" storage
                                               "--username" username "--label" "ert"))))
            (setq proc (make-process
                        :name "jaunder-server" :buffer stderr :noquery t
                        :command (list bin "serve" "--bind" "127.0.0.1:0"
                                       "--db" db "--storage-path" storage
                                       "--runtime-file" rf "--environment" "dev")))
            (let* ((addr (jaunder-test--wait
                          (lambda () (jaunder-test--read-runtime-file rf)) "runtime.json"))
                   (base-url (format "http://%s:%s" (car addr) (cdr addr))))
              (jaunder-test--wait
               (lambda () (jaunder-test--http-reachable-p (concat base-url "/")))
               "server readiness")
              ;; #560: feeds/AtomPub require `site.base_url' to compose absolute URLs,
              ;; else they 500. Point it at this server's own (dynamic) address so the
              ;; atompub collection the auth-readiness poll hits returns 200, and emitted
              ;; URLs (Location/edit links) resolve back to it.
              (jaunder-test--run-cli bin "site-config" "set"
                                     "--db" db "--storage-path" storage
                                     "site.base_url" base-url)
              ;; Auth readiness: an unauthed GET / can succeed before the
              ;; just-provisioned session is reliably usable by the serving connection,
              ;; so the first authed request can race to a 401. Wait for a real 200.
              (jaunder-test--wait
               (lambda () (jaunder-test--authed-200-p
                           (jaunder--build-url base-url "atompub" username "posts")
                           username token))
               "auth readiness")
              (with-temp-file authinfo
                (insert (format "machine %s login %s password %s\n"
                                (car addr) username token)))
              (list :proc proc :stderr stderr :tmp tmp
                    :base-url base-url :username username
                    :token token :authinfo authinfo))))
      (error
       (when (process-live-p proc) (delete-process proc))
       (when (buffer-live-p stderr) (kill-buffer stderr))
       (ignore-errors (delete-directory tmp t))
       (signal (car err) (cdr err))))))

(defun jaunder-test--server-down (state)
  "Tear down the server + tempdir described by STATE (`jaunder-test--server-up')."
  (let ((proc (plist-get state :proc))
        (stderr (plist-get state :stderr))
        (tmp (plist-get state :tmp)))
    (when (process-live-p proc) (delete-process proc))
    (when (buffer-live-p stderr) (kill-buffer stderr))
    (when (and tmp (file-directory-p tmp)) (delete-directory tmp t))))

(defun jaunder-test--server-up-retrying (&optional attempts)
  "Like `jaunder-test--server-up' but retry up to ATTEMPTS times (default 3) on a
boot/readiness failure, re-signalling the last error if all fail.  The batch
runner boots ONE server for the whole suite (#628), so that single boot is a
point of failure — an occasional slow-VM boot must retry with a fresh server,
not kill all 14 tests.  Each failed attempt has already torn itself down (see
`jaunder-test--server-up'), so a retry starts clean."
  (let ((attempts (or attempts 3))
        (n 0)
        (state nil))
    (while (and (not state) (< n attempts))
      (setq n (1+ n))
      (condition-case err
          (setq state (jaunder-test--server-up))
        (error
         (if (>= n attempts)
             (signal (car err) (cdr err))
           (message "jaunder-test: server-up attempt %d/%d failed (%s); retrying"
                    n attempts (error-message-string err))))))
    state))

(defun jaunder-test--global-bindings (state)
  "Return (SYMBOLS . VALUES) for the harness globals derived from STATE, so the
batch runner (via `set') and the fallback (via `cl-progv') share one derivation."
  (let ((base (plist-get state :base-url))
        (user (plist-get state :username)))
    (cons '(jaunder-test-base-url jaunder-test-username jaunder-test-app-password
                                  jaunder--active-blog auth-source-do-cache auth-sources)
          (list base user (plist-get state :token)
                (list :base-url base :username user)
                nil
                (list (plist-get state :authinfo))))))

(defun jaunder-test--set-globals (state)
  "Permanently bind the harness globals to STATE for the whole batch run (#628)."
  (let ((b (jaunder-test--global-bindings state)))
    (cl-mapc #'set (car b) (cdr b))))

(defmacro jaunder-test--bind-from (state &rest body)
  "Dynamically bind the harness globals from STATE around BODY (fallback path)."
  (declare (indent 1) (debug t))
  (let ((b (make-symbol "bindings")))
    `(let ((,b (jaunder-test--global-bindings ,state)))
       (cl-progv (car ,b) (cdr ,b) ,@body))))

(defmacro jaunder-test--with-live-server (&rest body)
  "Run BODY against a live jaunder server.
If one is already bound — the batch runner set the harness globals for a shared
server (#628) — reuse it and just run BODY.  Otherwise boot a throwaway server
for BODY's dynamic extent and tear it down after, preserving standalone
interactive runs (`M-x ert' on a single test).  See ADR-0035."
  (declare (indent 0) (debug t))
  `(if jaunder-test-base-url
       (progn ,@body)
     (let ((jaunder-test--st (jaunder-test--server-up)))
       (unwind-protect
           (jaunder-test--bind-from jaunder-test--st ,@body)
         (jaunder-test--server-down jaunder-test--st)))))

(provide 'jaunder-integration-helper)
;;; jaunder-integration-helper.el ends here
