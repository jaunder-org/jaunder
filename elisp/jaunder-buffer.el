;;; jaunder-buffer.el --- Jaunder org-buffer read/write + header parsing -*- lexical-binding: t; -*-

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
;; Low-level org-buffer manipulation: locating and parsing the leading metadata
;; header block, reading #+PROPERTY:/#+KEYWORD: values, and writing them back
;; idempotently.  The org-format primitives the higher layers build on.

;;; Code:

(require 'org)
(require 'org-element)

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

(provide 'jaunder-buffer)
;;; jaunder-buffer.el ends here
