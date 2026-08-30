;;; jaunder-media.el --- Jaunder media upload + link substitution -*- lexical-binding: t; -*-

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
;; Detect qualifying local-file links in a post's body, upload each distinct
;; file once (the server sha256-dedups), and rewrite the links in the sent body
;; to the harvested server URLs — without ever mutating the authoring buffer.

;;; Code:

(require 'jaunder-org) ;; cov:ignore: load-time dependency declaration has no Edebug execution stop
(require 'jaunder-atom) ;; cov:ignore: load-time dependency declaration has no Edebug execution stop
(require 'jaunder-config) ;; cov:ignore: load-time dependency declaration has no Edebug execution stop
(require 'jaunder-transport) ;; cov:ignore: load-time dependency declaration has no Edebug execution stop
(require 'jaunder-warn) ;; cov:ignore: load-time dependency declaration has no Edebug execution stop

(defconst jaunder--media-types ;; cov:ignore: constant declaration has no Edebug execution stop
  '(("jpg" . "image/jpeg")
    ("jpeg" . "image/jpeg")
    ("png" . "image/png")
    ("gif" . "image/gif")
    ("webp" . "image/webp")
    ("svg" . "image/svg+xml")
    ("mp3" . "audio/mpeg")
    ("ogg" . "audio/ogg")
    ("oga" . "audio/ogg")
    ("flac" . "audio/flac")
    ("wav" . "audio/wav")
    ("mp4" . "video/mp4")
    ("webm" . "video/webm")
    ("pdf" . "application/pdf"))
  "Alist of lowercase media extension → MIME type.")

(defun jaunder--media-content-type (filename)
  "Return the deterministic media MIME type for FILENAME.
The extension match is case-insensitive.  Unknown or extensionless names use
`application/octet-stream'."
  (let ((ext (downcase (or (file-name-extension filename) ""))))
    (or (cdr (assoc ext jaunder--media-types))
        "application/octet-stream")))

(defun jaunder--media-link-p (link-record)
  "Return non-nil when LINK-RECORD identifies a local-path link.
LINK-RECORD is a `jaunder--org-link->record' plist.  Only `file:' and
`attachment:' records qualify.  This type-only predicate is shared by collection
and substitution so their positional one-for-one alignment stays in lockstep."
  (not (null (member (plist-get link-record :type) '("file" "attachment")))))

(defun jaunder--upload-media (path content-type)
  "Upload the file at PATH as CONTENT-TYPE to the media collection; return its URL.
POSTs the raw bytes to `/atompub/{user}/media' with the filename in a `Slug'
header (the server sha256-dedups: 201 new / 200 re-upload), then harvests the
server-assigned binary URL from the response entry's `<content src>' via
`jaunder--harvest-response-fields'.  Signals an error on any non-2xx status."
  (let* ((url (jaunder--build-url (jaunder--active-base-url) "atompub"
                                  (jaunder--active-username) "media"))
         (resp (jaunder--http-request
                "POST" url (list 'file path) content-type
                (list (cons "Slug" (file-name-nondirectory path)))))
         (status (plist-get resp :status)))
    (unless (memq status '(200 201))
      (error "jaunder: media upload of %s failed (HTTP %s)" path status))
    (cdr (assq 'content-src
               (jaunder--harvest-response-fields (plist-get resp :body))))))

(defun jaunder--collect-media-links ()
  "Collect qualifying local-file links in the buffer's body region, in order.
Each selected record from `jaunder--org-body-links' becomes a plist (:raw-link
RAW :content-type MIME :path ABS).  MIME is selected from the resolved absolute
`:file', not the raw Org target.  Records remain in document order, one-for-one
with the links in the sent body."
  (delq nil
        (mapcar (lambda (rec)
                  (when (jaunder--media-link-p rec)
                    (let ((file (plist-get rec :file)))
                      (list :raw-link (plist-get rec :raw-link)
                            :content-type (jaunder--media-content-type file)
                            :path file))))
                (jaunder--org-body-links))))

(defun jaunder--substitute-media (body urls)
  "Return BODY with its qualifying media links rewritten to URLS, in order.
Delegates the org rewrite to `jaunder--org-substitute-links', selecting the
media links via `jaunder--media-link-p'."
  (jaunder--org-substitute-links body #'jaunder--media-link-p urls))

(defun jaunder--media-preflight (records)
  "Signal one error if any RECORDS `:path' cannot be uploaded.
Every resolved path must exist, be readable, and be a regular file.  All failing
paths are reported before any upload begins."
  (let ((failing
         (delq nil
               (mapcar (lambda (record)
                         (let ((path (plist-get record :path)))
                           (unless (and (file-exists-p path)
                                        (file-readable-p path)
                                        (file-regular-p path))
                             path)))
                       records))))
    (when failing
      (error "jaunder: media file(s) missing, unreadable, or not regular: %s"
             (mapconcat #'identity failing ", ")))))

(defun jaunder--git-toplevel (dir)
  "Return the git work-tree toplevel containing DIR, or nil.
Best-effort: nil when DIR is nil, git is unavailable, or DIR is not inside a
work tree — so the untracked-media check simply skips rather than erroring.
`call-process' itself signals when DIR is unenterable (e.g. a remote TRAMP
`default-directory'), so the invocation is wrapped to skip on any signal too."
  (when (and dir (executable-find "git"))
    (ignore-errors
      (let ((default-directory dir))
        (with-temp-buffer
          (when (zerop (call-process "git" nil (list t nil) nil
                                     "rev-parse" "--show-toplevel"))
            (string-trim (buffer-string))))))))

(defun jaunder--git-tracked-p (toplevel path)
  "Return non-nil when PATH is tracked by git in the TOPLEVEL work tree.
Untracked, gitignored, and outside-the-tree paths all report nil — `ls-files
--error-unmatch' exits non-zero for each."
  (let ((default-directory toplevel))
    (zerop (call-process "git" nil nil nil
                         "ls-files" "--error-unmatch" "--" path))))

(defun jaunder--warn-untracked-media (records)
  "Warn once per distinct untracked media `:path' in RECORDS.
Anchored on the git repository containing the current buffer's file; skips
entirely when that buffer is not in a work tree or git is unavailable.  A soft
authoring-hygiene nudge (a fresh clone would lack local preview) that never
blocks the publish.  Gated by `jaunder-warn-untracked-media'."
  (when jaunder-warn-untracked-media
    (let ((toplevel (jaunder--git-toplevel
                     (and buffer-file-name
                          (file-name-directory buffer-file-name)))))
      (when toplevel
        (let (seen)
          (dolist (r records)
            (let ((path (plist-get r :path)))
              (when (and path (not (member path seen)))
                (push path seen)
                (unless (jaunder--git-tracked-p toplevel path)
                  (jaunder--warn
                   "referenced media %s is not tracked by git in this document's repository; a fresh clone will lack local preview"
                   path))))))))))

(defun jaunder--localize-media (body)
  "Upload the current buffer's local files and return BODY with links localized.
Detect qualifying links in the buffer's body region, preflight every target,
warn about untracked media, upload each distinct resolved file once, and rewrite
those links in BODY to harvested server URLs in document order.  The authoring
buffer is never modified."
  (let ((records (jaunder--collect-media-links)))
    (jaunder--media-preflight records)
    (jaunder--warn-untracked-media records)
    (let ((cache (make-hash-table :test 'equal)))
      (dolist (r records)
        (let ((path (plist-get r :path)))
          (unless (gethash path cache)
            (puthash path
                     (jaunder--upload-media path (plist-get r :content-type))
                     cache))))
      (jaunder--substitute-media
       body
       (mapcar (lambda (r) (gethash (plist-get r :path) cache)) records)))))

(provide 'jaunder-media) ;; cov:ignore: feature declaration has no Edebug execution stop
;;; jaunder-media.el ends here
