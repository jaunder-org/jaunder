;;; jaunder-entry.el --- Jaunder AtomPub entry IR -*- lexical-binding: t; -*-

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
;; The format-neutral intermediate representation: a post mapped to abstract
;; AtomPub fields.  Format mappers (org, and any future markdown) produce a
;; `jaunder-entry'; the atom wire encoder consumes one.

;;; Code:

(require 'cl-lib)
(require 'subr-x)

(cl-defstruct (jaunder-entry (:constructor jaunder--make-entry))
  "Structured AtomPub entry mapped from a source buffer.
Holds abstract field values only; wire encoding (namespaces, media types,
`app:draft' nesting) lives in `jaunder--atom-entry->xml'.  `body' is the
body-only content with the metadata header block stripped."
  title categories summary audiences draft content-type body published)

(defconst jaunder--max-audience-id 9223372036854775807
  "Largest Named audience ID accepted by the AtomPub protocol.")

(defun jaunder--audience-sort-key (audience)
  "Return the canonical sort key for validated AUDIENCE."
  (cond
   ((equal audience "public") '(0 0))
   ((equal audience "subscribers") '(1 0))
   ((equal audience "private") '(2 0))
   (t (list 3 (string-to-number (substring audience (length "named:")))))))

(defun jaunder--canonical-audiences (audiences)
  "Validate and canonically order AUDIENCES, preserving nil as omission.
Accepted values are `public', `subscribers', `private', and canonical positive
`named:ID' values within the signed 64-bit range.  Duplicates are rejected and
`private' must stand alone."
  (when audiences
    (unless (listp audiences)
      (error "jaunder: audiences must be a list"))
    (dolist (audience audiences)
      (unless
          (and
           (stringp audience)
           (or (member audience '("public" "subscribers" "private"))
               (and (string-match-p "\\`named:[1-9][0-9]*\\'" audience)
                    (<= (string-to-number (substring audience (length "named:")))
                        jaunder--max-audience-id))))
        (error "jaunder: invalid audience value %S" audience)))
    (when (/= (length audiences) (length (delete-dups (copy-sequence audiences))))
      (error "jaunder: duplicate audience value"))
    (when (and (member "private" audiences) (/= (length audiences) 1))
      (error "jaunder: private audience must stand alone"))
    (sort (copy-sequence audiences)
          (lambda (left right)
            (let ((left-key (jaunder--audience-sort-key left))
                  (right-key (jaunder--audience-sort-key right)))
              (or (< (car left-key) (car right-key))
                  (and (= (car left-key) (car right-key))
                       (< (cadr left-key) (cadr right-key)))))))))

(defun jaunder--ascii-tag-alphanumeric-p (character)
  "Return non-nil when CHARACTER is an ASCII letter or digit."
  (or (and (<= ?a character) (<= character ?z))
      (and (<= ?A character) (<= character ?Z))
      (and (<= ?0 character) (<= character ?9))))

(defun jaunder--tag-label-defect (label)
  "Return the first Tag grammar defect in LABEL, or nil when valid.
A defect is (KIND . OFFSET), where KIND is `wrong-type', `empty',
`invalid-start', or `invalid-character' and OFFSET is zero-based in the trimmed
LABEL.  The scanner
is the single Tag grammar authority used by both validation and interactive
repair diagnostics."
  (if (not (stringp label))
      '(wrong-type . 0)
    (let ((trimmed (string-trim label)))
      (cond
       ((string-empty-p trimmed) '(empty . 0))
       ((not (jaunder--ascii-tag-alphanumeric-p (aref trimmed 0)))
        '(invalid-start . 0))
       (t
        (let ((offset 1))
          (while (and (< offset (length trimmed))
                      (let ((character (aref trimmed offset)))
                        (or (jaunder--ascii-tag-alphanumeric-p character)
                            (= character ?-))))
            (setq offset (1+ offset)))
          (when (< offset (length trimmed))
            (cons 'invalid-character offset))))))))

(defun jaunder--valid-tag-slug-p (slug)
  "Return non-nil when SLUG is a canonical lowercase Jaunder Tag."
  (and (stringp slug)
       (equal slug (string-trim slug))
       (equal slug (downcase slug))
       (null (jaunder--tag-label-defect slug))))

(defun jaunder--valid-tag-label-p (label)
  "Return non-nil when LABEL satisfies Jaunder's case-preserving Tag boundary."
  (null (jaunder--tag-label-defect label)))

(provide 'jaunder-entry)
;;; jaunder-entry.el ends here
