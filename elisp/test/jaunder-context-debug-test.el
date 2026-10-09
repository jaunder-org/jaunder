;;; jaunder-context-debug-test.el --- Configuration and credential timing -*- lexical-binding: t; -*-

;;; Commentary:
;; Time the actual configuration and credential owners without changing their
;; resolved account context, lookup count, secret identity, or native conditions.

;;; Code:

(require 'ert)
(require 'cl-lib)
(require 'rx)
(require 'jaunder)
(load (expand-file-name "jaunder-debug-boundary-fixture.el"
                        (file-name-directory (or load-file-name buffer-file-name))) nil t)

(ert-deftest jaunder-context-debug-resolution-off-on-selection-and-errors ()
  "Keep longest-prefix selection and malformed/unconfigured errors private."
  (jaunder-debug-boundary--with-session
   (let ((jaunder-blogs '(("/private-root/" :base-url "https://private-parent/" :username "private-user")
                          ("/private-root/nested/" :base-url "https://private-child///" :username "private-child-user")))
         snapshots)
     (dolist (enabled '(nil t))
       (setq jaunder-debug enabled)
       (push (list (jaunder--resolve-blog "/private-root/nested/post.org")
                   (should-error (jaunder--resolve-blog "/unconfigured-private/post.org"))
                   (let ((jaunder-blogs '(("/private-root/" :base-url "private-malformed" :username "private-user"))))
                     (should-error (jaunder--resolve-blog "/private-root/post.org"))))
             snapshots)
       (unless enabled
         (should (zerop jaunder--debug-id-counter))
         (should-not (get-buffer jaunder--debug-buffer-name))))
     (should (equal (car snapshots) (cadr snapshots)))
     (should (equal (caar snapshots) '(:base-url "https://private-child" :username "private-child-user")))
     (let ((text (jaunder-debug-boundary--text)))
       (should (= 6 (jaunder-debug-boundary--label-count "config.resolve" text)))
       (should-not (string-match-p (rx (or "private" "https://")) text))))))

(ert-deftest jaunder-context-debug-auth-off-on-opaque-secret-and-lookup-count ()
  "Return the same secret object and call each actual credential seam once."
  (jaunder-debug-boundary--with-session
   (let ((jaunder--active-blog '(:base-url "https://private-host:8443" :username "private-user"))
         (secret (copy-sequence "private-password"))
         snapshots)
     (dolist (enabled '(nil t))
       (setq jaunder-debug enabled)
       (let ((searches 0) (evaluations 0) spec)
         (cl-letf (((symbol-function 'auth-source-search)
                    (lambda (&rest args)
                      (setq searches (1+ searches) spec args)
                      (list (list :secret (lambda () (setq evaluations (1+ evaluations)) secret))))))
           (should (eq (jaunder--auth-secret) secret)))
         (push (list searches evaluations spec) snapshots))
       (unless enabled
         (should (zerop jaunder--debug-id-counter))
         (should-not (get-buffer jaunder--debug-buffer-name))))
     (should (equal (car snapshots) (cadr snapshots)))
     (should (equal (car snapshots) '(1 1 (:host "private-host" :user "private-user" :max 1))))
     (let ((text (jaunder-debug-boundary--text)))
       (should (= 2 (jaunder-debug-boundary--label-count "auth.lookup" text)))
       (should-not (string-match-p (rx (or "private" "https://")) text))))))

(ert-deftest jaunder-context-debug-auth-exact-error-and-quit-data ()
  "Diagnostic classification must not replace native credential conditions."
  (jaunder-debug-boundary--with-session
   (let ((jaunder--active-blog '(:base-url "https://private-host" :username "private-user")))
     (dolist (kind '(error quit))
       (dolist (enabled '(nil t))
         (setq jaunder-debug enabled)
         (let ((calls 0) (data '("private-condition" 17)))
           (cl-letf (((symbol-function 'auth-source-search)
                      (lambda (&rest _) (setq calls (1+ calls)) (signal kind data))))
             (should (equal (condition-case condition (jaunder--auth-secret)
                              (error condition) (quit condition))
                            (cons kind data))))
           (should (= calls 1))
           (should-not jaunder--debug-operation-stack))))
     (let ((text (jaunder-debug-boundary--text)))
       (should (= 4 (jaunder-debug-boundary--label-count "auth.lookup" text)))
       (should (string-match-p (rx " outcome=error" line-end) text))
       (should (string-match-p (rx " outcome=cancelled" line-end) text))
       (should-not (string-match-p (rx (or "private" "https://")) text))))))

(ert-deftest jaunder-context-debug-disabled-does-no-diagnostic-work ()
  "Disabled configuration and credential owners do not allocate or time spans."
  (jaunder-debug-boundary--with-session
   (let ((jaunder-blogs '(("/private-root/" :base-url "https://private-host" :username "private-user")))
         (jaunder--active-blog '(:base-url "https://private-host" :username "private-user"))
         (clock (symbol-function 'float-time))
         (formatter (symbol-function 'format-time-string))
         (clocks 0) (formats 0) (begins 0))
     (cl-letf (((symbol-function 'float-time)
                (lambda (&rest args) (setq clocks (1+ clocks)) (apply clock args)))
               ((symbol-function 'format-time-string)
                (lambda (&rest args) (setq formats (1+ formats)) (apply formatter args)))
               ((symbol-function 'jaunder--debug-begin)
                (lambda (&rest _) (setq begins (1+ begins))))
               ((symbol-function 'auth-source-search)
                (lambda (&rest _) '((:secret "private-password")))))
       (jaunder--resolve-blog "/private-root/post.org")
       (jaunder--auth-secret))
     (should (equal (list clocks formats begins jaunder--debug-id-counter) '(0 0 0 0)))
     (should-not (get-buffer jaunder--debug-buffer-name)))))

(provide 'jaunder-context-debug-test)
;;; jaunder-context-debug-test.el ends here
