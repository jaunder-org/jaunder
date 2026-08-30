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

(cl-defstruct (jaunder-entry (:constructor jaunder--make-entry))
              "Structured AtomPub entry mapped from a source buffer.
Holds abstract field values only; wire encoding (namespaces, media types,
`app:draft' nesting) lives in `jaunder--atom-entry->xml'.  `body' is the
body-only content with the metadata header block stripped."
              title categories summary draft content-type body published)

(provide 'jaunder-entry)
;;; jaunder-entry.el ends here
