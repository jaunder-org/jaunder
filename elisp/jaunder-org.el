;;; jaunder-org.el --- Jaunder org-buffer -> entry mapping -*- lexical-binding: t; -*-

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
;; The org format adapter: map an org-mode buffer's metadata header block and
;; body to the format-neutral `jaunder-entry'.  A future non-org format (e.g.
;; markdown) would be a sibling adapter producing the same IR.

;;; Code:

(require 'org)
(require 'jaunder-entry)
(require 'jaunder-buffer)
(require 'jaunder-datetime)

(defconst jaunder--org-media-type "text/org"
  "The atom:content media type for org source.
`jaunder--org->atom' converts an org buffer, so its content is always org;
the media type is knowable from the converter, not from any header field.
Non-org authoring buffers (markdown/html) are separate future converters,
out of scope for v1.")

(defun jaunder--org->atom ()
  "Map the current org buffer to a `jaunder-entry'.
Reads the metadata header block via `org-collect-keywords' and carries the
body-only content with the header block stripped.  Non-mutating.  The
`published' slot is filled by the timezone computation (see
`jaunder--org-date->utc'); `body' still holds local media links, substituted
later by the media unit."
  (let* ((kws (org-collect-keywords
               '("TITLE" "DATE" "KEYWORDS" "DESCRIPTION" "PROPERTY")))
         (props (jaunder--collect-properties kws))
         (raw-title (cadr (assoc "TITLE" kws)))
         (title (and raw-title (not (string= (string-trim raw-title) "")) raw-title))
         (categories (jaunder--split-keywords (cdr (assoc "KEYWORDS" kws))))
         (descriptions (cdr (assoc "DESCRIPTION" kws)))
         (summary (and descriptions (mapconcat #'identity descriptions "\n")))
         (status (cdr (assoc "JAUNDER_STATUS" props)))
         (draft (and status (string= (downcase status) "draft") t))
         (date-raw (cadr (assoc "DATE" kws)))
         (tz (cdr (assoc "JAUNDER_DATE_TZ" props)))
         ;; Drafts carry no publish time; "publish now" (published status, no
         ;; #+DATE) omits it so the server stamps it (see the spec status table).
         (published (and (not draft) date-raw
                         (jaunder--org-date->utc date-raw tz))))
    (jaunder--make-entry
     :title title
     :categories categories
     :summary summary
     :draft draft
     :content-type jaunder--org-media-type
     :body (string-trim-right
            (buffer-substring-no-properties (jaunder--body-start) (point-max)))
     :published published)))

(provide 'jaunder-org)
;;; jaunder-org.el ends here
