;;; jaunder.el --- Jaunder blogging client (AtomPub) -*- lexical-binding: t; -*-

;; Author: Jaunder contributors
;; Version: 0.1.0
;; Package-Requires: ((emacs "27.1"))
;; Keywords: hypermedia, comm, outlines
;; URL: https://jaunder.org

;;; Commentary:
;; Shared plumbing for the Jaunder Emacs blogging front-end over AtomPub.
;; This is the Infra-unit skeleton (issue #73): pure helpers plus seams that
;; units C (#74, authoring/publish) and D (#75, management/reconcile) extend.

;;; Code:

(require 'url-parse)
(require 'auth-source)

(defgroup jaunder nil
  "Emacs blogging front-end for Jaunder over AtomPub."
  :group 'comm
  :prefix "jaunder-")

(defcustom jaunder-base-url nil
  "Base URL of the Jaunder instance, e.g. \"https://blog.example.com\"."
  :type '(choice (const :tag "Unset" nil) string)
  :group 'jaunder)

(defcustom jaunder-username nil
  "Username used for AtomPub authentication."
  :type '(choice (const :tag "Unset" nil) string)
  :group 'jaunder)

;;; Pure helpers

(defun jaunder--build-url (base &rest segments)
  "Join BASE and path SEGMENTS into a normalized URL.
Trailing slashes on BASE and surrounding slashes on each segment are
collapsed to single separators; nil or empty segments are dropped.
Signals an error when BASE is nil or empty."
  (when (or (null base) (string= base ""))
    (error "jaunder--build-url: BASE must be non-empty"))
  (let ((head (replace-regexp-in-string "/+\\'" "" base))
        (tail (delq nil
                    (mapcar (lambda (s)
                              (when (and s (not (string= s "")))
                                (let ((stripped (replace-regexp-in-string "\\`/+\\|/+\\'" "" s)))
                                  ;; An all-slash segment (e.g. "/") strips to ""; drop it
                                  ;; rather than relying on `delq' matching interned "".
                                  (unless (string= stripped "") stripped))))
                            segments))))
    (mapconcat #'identity (cons head tail) "/")))

(defun jaunder--basic-auth-header (user password)
  "Return the HTTP Basic Authorization header cons for USER and PASSWORD.
The value is \"Basic <base64(user:password)>\" with no line breaks.
Credentials are UTF-8-encoded before base64 (RFC 7617) so non-ASCII
usernames/passwords are handled rather than raising."
  (cons "Authorization"
        (concat "Basic "
                (base64-encode-string
                 (encode-coding-string (concat user ":" password) 'utf-8) t))))

(defun jaunder--auth-source-spec (base-url user)
  "Return the `auth-source-search' plist for BASE-URL and USER.
:host is the URL host of BASE-URL (port excluded); at most one match."
  (list :host (url-host (url-generic-parse-url base-url))
        :user user
        :max 1))

;;; Seams — implemented by later units; calling them now is a programmer error.

(defun jaunder--http-request (&rest _args)
  "HTTP transport seam.  Implemented by unit C (issue #74)."
  (error "jaunder: HTTP layer not yet implemented (unit C, issue #74)"))

(defun jaunder--auth-secret ()
  "Retrieve the app password for `jaunder-username' via auth-source.
Thin I/O wrapper over `auth-source-search' using `jaunder--auth-source-spec'."
  (let* ((match (car (apply #'auth-source-search
                            (jaunder--auth-source-spec jaunder-base-url
                                                       jaunder-username))))
         (secret (and match (plist-get match :secret))))
    (cond ((functionp secret) (funcall secret))
          (secret secret)
          (t (error "jaunder: no auth-source entry for %s@%s"
                    jaunder-username jaunder-base-url)))))

(defun jaunder--org->atom (&rest _args)
  "Org->Atom mapping seam.  Implemented by unit C (issue #74)."
  (error "jaunder: org->atom mapping not yet implemented (unit C, issue #74)"))

(defun jaunder--atom->org (&rest _args)
  "Atom->Org mapping seam.  Implemented by units C/D (issues #74/#75)."
  (error "jaunder: atom->org mapping not yet implemented (units C/D, issues #74/#75)"))

(provide 'jaunder)
;;; jaunder.el ends here
