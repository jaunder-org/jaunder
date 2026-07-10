;;; jaunder-publish.el --- Jaunder publish/new-post commands -*- lexical-binding: t; -*-

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
;; The user-facing commands and their orchestration: `jaunder-new-post',
;; `jaunder-publish', and `jaunder-save-draft', plus the ID-first safe-to-resume
;; write-back that ties the buffer, the mapper, the wire, media, and transport
;; together (ADR-0047).

;;; Code:

(require 'jaunder-entry)
(require 'jaunder-config)
(require 'jaunder-buffer)
(require 'jaunder-datetime)
(require 'jaunder-atom)
(require 'jaunder-org)
(require 'jaunder-transport)
(require 'jaunder-media)

(defun jaunder--validate-publish (entry status date-raw tz)
  "Signal an error if ENTRY is not publishable; return nil otherwise.
Requires a non-empty body; a `scheduled' STATUS requires a future #+DATE:
\(DATE-RAW interpreted in TZ)."
  (when (string= (string-trim (or (jaunder-entry-body entry) "")) "")
    (error "jaunder: refusing to publish an empty body"))
  (when (and status (string= (downcase status) "scheduled"))
    (let ((utc (and date-raw (jaunder--org-date->utc date-raw tz))))
      (unless (and utc (time-less-p (current-time) (date-to-time utc)))
        (error "jaunder: a scheduled post needs a future #+DATE:"))))
  nil)

(defun jaunder--location->id (location)
  "Return the trailing numeric post id from a create `Location' URL, or nil."
  (when (and location (string-match "/\\([0-9]+\\)/?\\'" location))
    (match-string 1 location)))

(defun jaunder--force-draft (entry)
  "Mark ENTRY a server-side draft in place: set `draft', clear `published'.
Clearing `published' keeps `jaunder--atom-entry->xml' from emitting a
`<published>' on a draft (it emits one whenever the slot is set)."
  (setf (jaunder-entry-draft entry) t
        (jaunder-entry-published entry) nil)
  entry)

(defun jaunder--rename-to-slug (slug)
  "Rename the current buffer's file and buffer to SLUG.org in its directory.
A no-op when already so named; on collision appends `-N'.  Returns the path."
  (let* ((old (or (buffer-file-name)
                  (error "jaunder: buffer is not visiting a file")))
         (dir (file-name-directory old))
         (target (expand-file-name (concat slug ".org") dir)))
    (if (string= old target)
        old
      (let ((final target) (n 1))
        (while (file-exists-p final)
          (setq final (expand-file-name (format "%s-%d.org" slug n) dir)
                n (1+ n)))
        (rename-file old final)
        ;; ALONG-WITH-FILE=t: the file is already moved, so don't re-save it;
        ;; NO-QUERY=t: never prompt (publish is automated).
        (set-visited-file-name final t t)
        final))))

(defun jaunder--write-back (response created)
  "Persist server-assigned values from RESPONSE into the current buffer.
RESPONSE is a `jaunder--http-request' plist.  CREATED non-nil (a POST) writes
JAUNDER_ID from the `Location' header; an update leaves it unchanged.  Writes
JAUNDER_ID first, then JAUNDER_SLUG, JAUNDER_SYNCED (ETag, verbatim),
JAUNDER_SYNCED_AT (now), and the resolved publish time.  Saves the buffer and
returns the slug.

Precondition for the publish-now `#+DATE:' render: the buffer's JAUNDER_DATE_TZ
must already be recorded (the command calls `jaunder--ensure-date-tz' before the
send); absent it, the render falls back to the local zone via
`jaunder--resolve-zone'."
  (let* ((fields (jaunder--harvest-response-fields (plist-get response :body)))
         (slug (cdr (assq 'slug fields)))
         (published (cdr (assq 'published fields)))
         (etag (jaunder--response-header response "ETag"))
         (now (format-time-string "%Y-%m-%dT%H:%M:%SZ" nil t)))
    (when created
      (let ((id (jaunder--location->id
                 (jaunder--response-header response "Location"))))
        (when id (jaunder--set-property "JAUNDER_ID" id))))
    (when slug (jaunder--set-property "JAUNDER_SLUG" slug))
    (when etag (jaunder--set-property "JAUNDER_SYNCED" etag))
    (jaunder--set-property "JAUNDER_SYNCED_AT" now)
    (when published
      ;; published→UTC (drop the offset): the canonical value the server stamped.
      (let ((utc (format-time-string "%Y-%m-%dT%H:%M:%SZ"
                                     (date-to-time published) t))
            (tz (jaunder--buffer-property "JAUNDER_DATE_TZ")))
        (jaunder--set-property "JAUNDER_DATE_UTC" utc)
        ;; "publish now": no author #+DATE: — render it from the server time.
        (unless (jaunder--buffer-keyword "DATE")
          (jaunder--set-keyword "DATE" (jaunder--utc->org-date utc tz)))))
    (save-buffer)
    slug))

(defun jaunder--new-post-in (dir now-string)
  "Create and save a timestamped draft in DIR stamped NOW-STRING; return its path.
Inserts the minimal org template (empty TITLE, DATE now, empty KEYWORDS and
DESCRIPTION, JAUNDER_STATUS draft) and leaves point in the body."
  (let* ((path (expand-file-name (format "draft-%s.org" now-string) dir))
         (buf (find-file-noselect path)))
    (with-current-buffer buf
      (insert "#+TITLE: \n"
              (format "#+DATE: %s\n" (format-time-string "[%Y-%m-%d %a %H:%M]"))
              "#+KEYWORDS: \n"
              "#+DESCRIPTION: \n"
              "#+PROPERTY: JAUNDER_STATUS draft\n\n")
      (save-buffer))
    path))

(defun jaunder-new-post ()
  "Create a new Jaunder draft in the blog whose directory contains `default-directory'.
When no blog matches, prompt to choose one from `jaunder-blogs'.  Inserts the
minimal template and visits the file."
  (interactive)
  (let* ((dir (or (car (jaunder--blog-entry-for default-directory))
                  (if jaunder-blogs
                      (completing-read "Blog directory: " (mapcar #'car jaunder-blogs) nil t)
                    default-directory)))
         (path (jaunder--new-post-in dir (format-time-string "%Y%m%dT%H%M%S"))))
    (switch-to-buffer (find-file-noselect path))
    (goto-char (point-max))))

(defconst jaunder--entry-content-type "application/atom+xml;type=entry"
  "Request Content-Type for an AtomPub <entry> POST/PUT.")

(defun jaunder-publish (&optional force-draft)
  "Publish the current buffer's org post over AtomPub.
Resolves the blog from the buffer's file, records the machine zone when unset,
maps + validates, uploads media (sent body only), sends (POST create / PUT with
If-Match on update), writes back server values (ID first), and renames the temp
file to <slug>.org.  With FORCE-DRAFT (see `jaunder-save-draft') pushes an
`app:draft' regardless of JAUNDER_STATUS.  A non-2xx status leaves the on-disk
file pristine."
  (interactive)
  (let ((file (or (buffer-file-name)
                  (error "jaunder: buffer is not visiting a file"))))
    (jaunder--with-blog file
                        (let* ((status (jaunder--buffer-property "JAUNDER_STATUS"))
                               (date-raw (jaunder--buffer-keyword "DATE"))
                               (tz (jaunder--buffer-property "JAUNDER_DATE_TZ"))
                               (id (jaunder--buffer-property "JAUNDER_ID"))
                               (synced (jaunder--buffer-property "JAUNDER_SYNCED"))
                               (entry (jaunder--org->atom)))
                          (when force-draft (jaunder--force-draft entry))
                          ;; Validate BEFORE any buffer write, so a rejected publish leaves the
                          ;; on-disk file pristine.
                          (jaunder--validate-publish entry status date-raw tz)
                          ;; Record the machine zone (idempotent) so #+DATE: is interpreted in a
                          ;; recorded zone on later machines.  A first-publish's org->atom above
                          ;; already used the local zone, which equals the captured name.
                          (jaunder--ensure-date-tz)
                          (setf (jaunder-entry-body entry)
                                (jaunder--localize-media (jaunder-entry-body entry)))
                          (let* ((xml (jaunder--atom-entry->xml entry))
                                 (resp (if id
                                           (jaunder--http-request
                                            "PUT"
                                            (jaunder--build-url (jaunder--active-base-url) "atompub"
                                                                (jaunder--active-username) "posts" id)
                                            xml jaunder--entry-content-type
                                            (when synced (list (cons "If-Match" synced))))
                                         (jaunder--http-request
                                          "POST"
                                          (jaunder--build-url (jaunder--active-base-url) "atompub"
                                                              (jaunder--active-username) "posts")
                                          xml jaunder--entry-content-type)))
                                 (code (plist-get resp :status)))
                            (unless (memq code '(200 201))
                              (error "jaunder: publish failed (HTTP %s)" code))
                            (let ((slug (jaunder--write-back resp (null id))))
                              (when slug (jaunder--rename-to-slug slug))
                              (message "jaunder: published %s" (or slug ""))))))))

(defun jaunder-save-draft ()
  "Publish the current buffer as a server-side draft (forces `app:draft')."
  (interactive)
  (jaunder-publish t))

(defun jaunder--atom->org (&rest _args)
  "Atom->Org mapping seam; not yet implemented."
  (error "jaunder: atom->org mapping not yet implemented"))

(provide 'jaunder-publish)
;;; jaunder-publish.el ends here
