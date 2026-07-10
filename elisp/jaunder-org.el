;;; jaunder-org.el --- Jaunder org document interface -*- lexical-binding: t; -*-

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
;; The org interface: read an org buffer's metadata header and body through org's
;; own machinery (`org-collect-keywords', `org-element'), write file keywords back
;; idempotently, map the buffer to the format-neutral `jaunder-entry', and capture
;; the machine time zone.  A future non-org format (e.g. markdown) would be a
;; sibling adapter producing the same IR.
;;
;; Writing file keywords stays a targeted line edit: org exposes no setter for
;; `#+KEY:'/`#+PROPERTY:' lines (`org-set-property' writes a property drawer).

;;; Code:

(require 'cl-lib)
(require 'org)
(require 'org-element)
(require 'org-attach)
(require 'jaunder-entry)
(require 'jaunder-datetime)

(defconst jaunder--org-media-type "text/org"
  "The atom:content media type for org source.
`jaunder--org->atom' converts an org buffer, so its content is always org;
the media type is knowable from the converter, not from any header field.
Non-org authoring buffers (markdown/html) are separate future converters,
out of scope for v1.")

(defun jaunder--collect-properties (keywords)
  "Return an alist of file-level #+PROPERTY: KEY/VALUE pairs from KEYWORDS.
KEYWORDS is the result of `org-collect-keywords'; each PROPERTY entry is a
\"KEY VALUE\" string split on the first run of whitespace."
  (delq nil
        (mapcar (lambda (line)
                  (when (string-match "\\`\\([^ \t]+\\)[ \t]+\\(.*\\)\\'" line)
                    (cons (match-string 1 line) (match-string 2 line))))
                (cdr (assoc "PROPERTY" keywords)))))

(defun jaunder--split-keywords (values)
  "Split each #+KEYWORDS: string in VALUES on commas and flatten.
Whitespace is trimmed and empty terms dropped."
  (let (out)
    (dolist (line values (nreverse out))
      (dolist (term (split-string line "," t "[ \t]+"))
        (unless (string= term "") (push term out))))))

(defun jaunder--body-start ()
  "Return the buffer position where content begins, after the metadata header.
The header block is the leading run of org file keywords (`#+KEY:' lines); this
returns the start of the first non-keyword top-level element via `org-element',
so \"keyword\" is org's own notion, not a regexp — an interleaved keyword such as
`#+AUTHOR:' is skipped too, so a later `#+PROPERTY: JAUNDER_*' cannot leak into
the body.  A buffer that opens on a headline has no header block, so content
starts at its beginning.  Shared by media detection and the org->atom body
extraction."
  (let ((tree (org-element-parse-buffer 'element))
        (pos (point-max)))
    (dolist (top (org-element-contents tree))
      (pcase (org-element-type top)
        ('headline
         (setq pos (min pos (org-element-property :begin top))))
        ('section
         (dolist (child (org-element-contents top))
           (unless (eq (org-element-type child) 'keyword)
             (setq pos (min pos (org-element-property :begin child))))))))
    pos))

(defun jaunder--set-keyword-line (line-re new-line)
  "Replace the first LINE-RE match in the leading header block with NEW-LINE.
When absent, insert NEW-LINE after the last contiguous header-keyword line
\(before any blank line or the body).  Header block only; the body is never
touched."
  (save-excursion
    (goto-char (point-min))
    (let ((case-fold-search t)
          (limit (jaunder--body-start)))
      (if (re-search-forward line-re limit t)
          (progn (beginning-of-line)
                 (delete-region (point) (line-end-position))
                 (insert new-line))
        (goto-char (point-min))
        (let ((insert-at (point-min)))
          (while (looking-at-p org-keyword-regexp)
            (forward-line 1)
            (setq insert-at (point)))
          (goto-char insert-at)
          (insert new-line "\n"))))))

(defun jaunder--set-property (key value)
  "Set the file-level #+PROPERTY: KEY to VALUE (idempotent replace or insert)."
  (jaunder--set-keyword-line
   (format "^[ \t]*#\\+PROPERTY:[ \t]+%s\\(?:[ \t].*\\)?$" (regexp-quote key))
   (format "#+PROPERTY: %s %s" key value)))

(defun jaunder--set-keyword (keyword value)
  "Set the file-level #+KEYWORD: to VALUE (idempotent replace or insert)."
  (jaunder--set-keyword-line
   (format "^[ \t]*#\\+%s:.*$" (regexp-quote keyword))
   (format "#+%s: %s" keyword value)))

(defun jaunder--buffer-property (key)
  "Return the #+PROPERTY: KEY value in the current buffer, or nil."
  (cdr (assoc key (jaunder--collect-properties
                   (org-collect-keywords '("PROPERTY"))))))

(defun jaunder--buffer-keyword (key)
  "Return the #+KEY: value in the current buffer, or nil."
  (cadr (assoc key (org-collect-keywords (list key)))))

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

(defun jaunder--ensure-date-tz ()
  "Ensure the buffer records a JAUNDER_DATE_TZ; return the effective zone string.
When unset, captures the machine's current zone (`jaunder--current-zone-name')
so #+DATE: is interpreted in a recorded zone, not one silently re-inferred on a
later machine.  Idempotent: an existing value is preserved verbatim."
  (or (jaunder--buffer-property "JAUNDER_DATE_TZ")
      (let ((zone (jaunder--current-zone-name)))
        (jaunder--set-property "JAUNDER_DATE_TZ" zone)
        zone)))

;;; Org link primitives (media-agnostic)

(defun jaunder--org-body-links ()
  "Return the `org-element' link objects in the current buffer's body region.
Walks links after the metadata header block (`jaunder--body-start'), in document
order — the org traversal media detection builds on."
  (save-restriction
    (narrow-to-region (jaunder--body-start) (point-max))
    (org-element-map (org-element-parse-buffer) 'link #'identity)))

(defun jaunder--org-link-file (link)
  "Resolve `org-element' LINK's local target to an absolute file path.
A `file:' path resolves against `default-directory'; an `attachment:' path via
`org-attach-expand' at the link's heading."
  (let ((path (org-element-property :path link)))
    (if (string= (org-element-property :type link) "attachment")
        (save-excursion
          (goto-char (org-element-property :begin link))
          (org-attach-expand path))
      (expand-file-name path))))

(defun jaunder--org-substitute-links (body predicate urls)
  "Return BODY with each org link satisfying PREDICATE rewritten to a URL in URLS.
URLS has one entry per satisfying link, in document order.  Each link's whole
inner target is replaced, brackets and any `[…][description]' preserved (result
stays `[[URL]]' / `[[URL][desc]]').  Rewrites right-to-left."
  (with-temp-buffer
    (insert body)
    (org-mode)
    (let* ((links (delq nil
                        (org-element-map (org-element-parse-buffer) 'link
                                         (lambda (link)
                                           (when (funcall predicate link) link)))))
           (pairs (cl-mapcar #'cons links urls)))
      (dolist (pair (nreverse pairs))
        (let* ((link (car pair))
               (url (cdr pair))
               (beg (org-element-property :begin link))
               (end (- (org-element-property :end link)
                       (or (org-element-property :post-blank link) 0)))
               (cb (org-element-property :contents-begin link))
               (ce (org-element-property :contents-end link))
               (desc (and cb ce (buffer-substring-no-properties cb ce))))
          (delete-region beg end)
          (goto-char beg)
          (insert (if desc (format "[[%s][%s]]" url desc) (format "[[%s]]" url))))))
    (buffer-substring-no-properties (point-min) (point-max))))

(provide 'jaunder-org)
;;; jaunder-org.el ends here
