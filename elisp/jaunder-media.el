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
;; Detect qualifying local-image links in a post's body, upload each distinct
;; file once (the server sha256-dedups), and rewrite the links in the sent body
;; to the harvested server URLs — without ever mutating the authoring buffer.

;;; Code:

(require 'org)
(require 'jaunder-org)
(require 'jaunder-atom)
(require 'jaunder-config)
(require 'jaunder-transport)

(defconst jaunder--media-image-types
  '(("png" . "image/png")
    ("jpg" . "image/jpeg")
    ("jpeg" . "image/jpeg")
    ("gif" . "image/gif")
    ("webp" . "image/webp")
    ("svg" . "image/svg+xml"))
  "Alist of lowercase image extension → MIME type for uploadable media.
Its key set is the qualification predicate: only links whose file extension is a
key are uploaded.  Non-image media types are out of scope for now.")

(defun jaunder--media-content-type (filename)
  "Return the media MIME type for FILENAME by extension, or nil if unqualified.
The extension match is case-insensitive."
  (let ((ext (downcase (or (file-name-extension filename) ""))))
    (cdr (assoc ext jaunder--media-image-types))))

(defun jaunder--media-link-p (link)
  "Return the media MIME type if org-element LINK is a qualifying local-image link.
Qualifies a `file:'- or `attachment:'-type link whose target has an image
extension; nil otherwise.  The single source of truth for \"qualifies\", shared by
detection and substitution so the two stay in lockstep — their positional
one-for-one alignment rides on agreeing here."
  (and (member (org-element-property :type link) '("file" "attachment"))
       (jaunder--media-content-type (org-element-property :path link))))

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
  "Collect qualifying local-image links in the current buffer's body region, in order.
Each `org-element' body link (`jaunder--org-body-links') that qualifies as an
uploadable image (`jaunder--media-link-p') becomes a plist (:raw-link RAW
:content-type MIME :path ABS), its file resolved via `jaunder--org-link-file'.
In document order, one-for-one with the links in the sent body."
  (delq nil
        (mapcar (lambda (link)
                  (let ((mime (jaunder--media-link-p link)))
                    (when mime
                      (list :raw-link (org-element-property :raw-link link)
                            :content-type mime
                            :path (jaunder--org-link-file link)))))
                (jaunder--org-body-links))))

(defun jaunder--substitute-media (body urls)
  "Return BODY with its qualifying media links rewritten to URLS, in order.
Delegates the org rewrite to `jaunder--org-substitute-links', selecting the media
links via `jaunder--media-link-p'."
  (jaunder--org-substitute-links body #'jaunder--media-link-p urls))

(defun jaunder--media-preflight (records)
  "Signal an error if any RECORDS `:path' is not a readable file.
RECORDS is a `jaunder--collect-media-links' list.  Checks every path and, if any
are missing, signals one error listing them all — fail-fast, upload nothing."
  (let ((missing (delq nil
                       (mapcar (lambda (r)
                                 (let ((p (plist-get r :path)))
                                   (unless (file-readable-p p) p)))
                               records))))
    (when missing
      (error "jaunder: media file(s) not found: %s"
             (mapconcat #'identity missing ", ")))))

(defun jaunder--localize-media (body)
  "Upload the current buffer's local images and return BODY with links localized.
Detects qualifying media links in the buffer's body region, pre-flights that all
exist (else errors, uploading nothing), uploads each distinct file once (server
sha256-dedups), and rewrites those links in BODY — the sent body — to the
harvested server URLs, in order.  The authoring buffer is never modified."
  (let ((records (jaunder--collect-media-links)))
    (jaunder--media-preflight records)
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

(provide 'jaunder-media)
;;; jaunder-media.el ends here
