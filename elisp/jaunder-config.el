;;; jaunder-config.el --- Jaunder blog config + request context -*- lexical-binding: t; -*-

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
;; User configuration (`jaunder-blogs') and blog resolution: which blog governs
;; a file (longest-prefix match), and the per-request active-blog context that
;; the transport reads through validating accessors.

;;; Code:

(require 'url-parse)

(defgroup jaunder nil
  "Emacs blogging front-end for Jaunder over AtomPub."
  :group 'comm
  :prefix "jaunder-")

(defcustom jaunder-blogs nil
  "Alist mapping a local directory to a Jaunder blog.
Each element is (DIRECTORY . PLIST), where PLIST carries :base-url and
:username (strings) and an optional :format (accepted for forward
compatibility but not used in v1 — org is the only converter)."
  :type '(alist :key-type directory
                :value-type (plist :key-type symbol :value-type string))
  :group 'jaunder)

(defun jaunder--blog-entry-for (file-or-dir)
  "Return the `jaunder-blogs' entry (DIRECTORY . PLIST) governing FILE-OR-DIR, or nil.
Longest-prefix match: the entry whose DIRECTORY is the longest prefix of
FILE-OR-DIR's expanded directory, so a nested blog root wins over its parent.
Both `jaunder--resolve-blog' (which blog to publish to) and `jaunder-new-post'
\(where to create a draft) share this one matcher."
  (let ((dir (file-name-as-directory
              (expand-file-name (if (file-directory-p file-or-dir)
                                    file-or-dir
                                  (file-name-directory file-or-dir)))))
        (best nil) (best-len -1))
    (dolist (entry jaunder-blogs)
      (let ((root (file-name-as-directory (expand-file-name (car entry)))))
        (when (and (string-prefix-p root dir) (> (length root) best-len))
          (setq best entry best-len (length root)))))
    best))

(defvar jaunder--active-blog nil
  "Plist (:base-url :username) of the blog for the in-flight request.
Dynamically bound by `jaunder--with-blog' for the extent of one request; it is
internal request context, not user configuration.  The transport reads it only
through `jaunder--active-base-url' / `jaunder--active-username', which fail
loudly when it is unset.")

(defun jaunder--active-base-url ()
  "Return the in-flight blog's base URL.
Errors when there is no active blog — i.e. a transport call made outside
`jaunder--with-blog' — rather than silently issuing a half-configured request."
  (or (plist-get jaunder--active-blog :base-url)
      (error "jaunder: no active blog (call within `jaunder--with-blog')")))

(defun jaunder--active-username ()
  "Return the in-flight blog's username.
Errors when there is no active blog — i.e. a transport call made outside
`jaunder--with-blog' — rather than silently issuing a half-configured request."
  (or (plist-get jaunder--active-blog :username)
      (error "jaunder: no active blog (call within `jaunder--with-blog')")))

(defun jaunder--resolve-blog (file-or-dir)
  "Return the blog plist (:base-url :username) governing FILE-OR-DIR.
Longest-prefix match against `jaunder-blogs' (`jaunder--blog-entry-for').
Errors when no blog matches, when the entry's :base-url is not an absolute URL,
or when it lacks a non-empty :username — a request is never issued
half-configured.  The returned :base-url is normalized (trailing slashes
stripped), so downstream URL joining can treat it as a clean prefix."
  (let ((best (cdr (jaunder--blog-entry-for file-or-dir))))
    (unless best
      (error "jaunder: no blog configured for %s (see `jaunder-blogs')" file-or-dir))
    (let* ((base-url (plist-get best :base-url))
           (username (plist-get best :username))
           (parsed (and (stringp base-url) (url-generic-parse-url base-url))))
      (unless (and parsed (url-type parsed)
                   (url-host parsed) (not (string= (url-host parsed) "")))
        (error "jaunder: blog for %s has a malformed :base-url: %S"
               file-or-dir base-url))
      (when (or (null username) (string= username ""))
        (error "jaunder: blog for %s has no :username" file-or-dir))
      (list :base-url (replace-regexp-in-string "/+\\'" "" base-url)
            :username username))))

(defmacro jaunder--with-blog (file &rest body)
  "Resolve the blog for FILE and run BODY with `jaunder--active-blog' bound.
Signals when FILE resolves to no configured, fully-specified blog."
  (declare (indent 1) (debug t))
  `(let ((jaunder--active-blog (jaunder--resolve-blog ,file)))
     ,@body))

(provide 'jaunder-config)
;;; jaunder-config.el ends here
