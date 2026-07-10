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

(defconst jaunder--header-keyword-re
  "^[ \t]*#\\+[A-Za-z][A-Za-z0-9_-]*:"
  "Regexp matching any org file-keyword line (`#+KEY:').
The metadata header block is the leading run of these; matching *any*
keyword (not just the mapped ones) means an interleaved keyword such as
`#+AUTHOR:' cannot halt stripping and leak a later `#+PROPERTY: JAUNDER_*'
into the sent body.  The trailing colon excludes block markers like
`#+begin_src'.")

(defconst jaunder--blank-line-re "^[ \t]*$"
  "Regexp matching a blank (whitespace-only) line.")

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
  "Return the position after the leading metadata header block in this buffer.
The header block is the leading contiguous run of header-keyword and blank lines.
Shared by `jaunder--strip-header-block' and media detection so both see the same
body region."
  (save-excursion
    (goto-char (point-min))
    (let ((case-fold-search t))
      (while (and (not (eobp))
                  (or (looking-at-p jaunder--blank-line-re)
                      (looking-at-p jaunder--header-keyword-re)))
        (forward-line 1)))
    (point)))

(defun jaunder--strip-header-block (text)
  "Return TEXT with its leading metadata header block removed.
Drops the leading contiguous run of header keyword lines and blank lines
(`jaunder--body-start'), which already positions at the start of content, then
strips only trailing whitespace (the buffer's final newline) — leading
whitespace on the first content line is body, not the header block, and is kept."
  (with-temp-buffer
    (insert text)
    (string-trim-right
     (buffer-substring-no-properties (jaunder--body-start) (point-max)))))

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
          (while (looking-at-p jaunder--header-keyword-re)
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
