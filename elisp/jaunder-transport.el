;;; jaunder-transport.el --- Jaunder HTTP transport + auth -*- lexical-binding: t; -*-

;; Copyright (C) 2026 Jaunder contributors

;; This program is free software: you can redistribute it and/or modify
;; it under the terms of the GNU General Public License as published by
;; the Free Software Foundation, either version 3 of the License, or
;; (at your option) any later version.
;;
;; This program is distributed in the hope that it will be useful,
;; but WITHOUT ANY WARRANTY; without even the implied warranty of
;; MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
;; GNU General Public License for more details.
;;
;; You should have received a copy of the GNU General Public License
;; along with this program.  If not, see <https://www.gnu.org/licenses/>.

;;; Commentary:
;; The network core: URL building, HTTP Basic auth (app password via
;; auth-source), and authenticated requests over `plz' (curl).  Reads the active
;; blog's base URL and username through `jaunder-config'.

;;; Code:

(require 'auth-source)
(require 'seq)
(require 'url-parse)
(require 'plz)
(require 'jaunder-config)

(defun jaunder--build-url (base &rest segments)
  "Join BASE and path SEGMENTS into a URL with single-slash separators.
Callers pass clean, non-empty path tokens; BASE is a normalized base URL (see
`jaunder--resolve-blog', which validates it and strips its trailing slash).
Signals an error when BASE is nil or empty — a broken invariant, not user input
to be massaged."
  (when (or (null base) (string= base ""))
    (error "jaunder--build-url: BASE must be non-empty"))
  (mapconcat #'identity (cons base segments) "/"))

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

(defun jaunder--plz-response->plist (response)
  "Convert a `plz-response' RESPONSE to a (:status :headers :body) plist.
Header names are downcased strings so `jaunder--response-header' can look
them up case-insensitively."
  (list :status (plz-response-status response)
        :headers (mapcar (lambda (h)
                           (cons (downcase (format "%s" (car h))) (cdr h)))
                         (plz-response-headers response))
        :body (or (plz-response-body response) "")))

(defun jaunder--response-header (response name)
  "Return the value of header NAME (case-insensitive) in RESPONSE, or nil."
  (cdr (assoc (downcase name) (plist-get response :headers))))

(defun jaunder--auth-secret ()
  "Retrieve the app password for the active blog's user via auth-source.
Thin I/O wrapper over `auth-source-search' using `jaunder--auth-source-spec'."
  (let* ((match (car (apply #'auth-source-search
                            (jaunder--auth-source-spec (jaunder--active-base-url)
                                                       (jaunder--active-username)))))
         (secret (and match (plist-get match :secret))))
    (cond ((functionp secret) (funcall secret))
          (secret secret)
          (t (error "jaunder: no auth-source entry for %s@%s"
                    (jaunder--active-username) (jaunder--active-base-url))))))

(defun jaunder--curl-header-value (value)
  "Escape VALUE so `plz' transmits the header intact through curl's config file.
plz writes each header as `--header \"NAME: VALUE\"' into a curl `--config' file
without escaping VALUE (plz 0.9.1, plz.el:503).  A raw double quote — as in a
strong `ETag' echoed back via `If-Match' — closes the config-file string early,
truncating the header to an empty value that curl then drops, so the precondition
never reaches the server.  Backslash-escaping `\\' and `\"' lets curl's config
parser rebuild the literal value."
  (replace-regexp-in-string "[\\\"]" "\\\\\\&" value))

(defun jaunder--http-request (method url &optional body content-type extra-headers)
  "Make an authenticated METHOD request to URL via `plz', returning a plist.
METHOD is an HTTP verb string; URL an absolute URL.  BODY is a request body: a
string, or the `plz' file form `(file PATH)' to upload a file's raw bytes.
CONTENT-TYPE and EXTRA-HEADERS (an alist of extra (NAME . VALUE) headers) apply to
write requests.  Basic-auth credentials come from `jaunder--auth-secret' for the
active blog's user.  Returns the `jaunder--plz-response->plist' plist; HTTP error
statuses (4xx/5xx) are reported in :status, not signalled.  A transport-level
failure re-signals.

`plz' drives the `curl' binary, so request construction does not depend on
the finicky dynamic-variable handling that made `url.el' occasionally drop
the auth header under load (ADR-0038)."
  (let ((headers (mapcar
                  (lambda (h) (cons (car h) (jaunder--curl-header-value (cdr h))))
                  (append
                   (list (jaunder--basic-auth-header (jaunder--active-username)
                                                     (jaunder--auth-secret)))
                   (when content-type (list (cons "Content-Type" content-type)))
                   extra-headers)))
        (verb (intern (downcase method))))
    (condition-case err
        (jaunder--plz-response->plist
         (plz verb url :headers headers :body body :as 'response))
      (plz-error
       (let* ((pe (seq-find #'plz-error-p (cdr err)))
              (resp (and pe (plz-error-response pe))))
         (if resp
             (jaunder--plz-response->plist resp)
           (signal (car err) (cdr err))))))))

(provide 'jaunder-transport)
;;; jaunder-transport.el ends here
