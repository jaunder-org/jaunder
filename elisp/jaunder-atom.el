;;; jaunder-atom.el --- Jaunder entry <-> AtomPub XML wire -*- lexical-binding: t; -*-

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
;; The AtomPub wire format: serialize a `jaunder-entry' to an <entry> XML string
;; and parse a server entry's XML back into harvested fields.  Format-neutral —
;; any source mapper that produces a `jaunder-entry' rides this wire unchanged.

;;; Code:

(require 'cl-lib)
(require 'dom)
(require 'jaunder-entry)

(defconst jaunder--atom-ns "http://www.w3.org/2005/Atom"
  "The Atom namespace URI.")

(defconst jaunder--app-ns "http://www.w3.org/2007/app"
  "The Atom Publishing Protocol namespace URI (`app:control'/`app:draft').")

(defun jaunder--atom-entry->xml (entry)
  "Serialize a `jaunder-entry' ENTRY to a standalone AtomPub <entry> XML string.
Builds a `dom' node and renders it with `dom-print', which escapes text and
attribute values.  Emits only set fields: `<title>'/`<summary>'/`<published>'
are omitted when nil, one `<category term>' per tag, and the
`<app:control><app:draft>yes>' marker (with the `app' namespace) only for a
draft.  All wire knowledge (namespaces, media types, element order) lives
here."
  (let* ((draft (jaunder-entry-draft entry))
         (attrs (append
                 (list (cons 'xmlns jaunder--atom-ns))
                 ;; Declare the app namespace only when it is used.
                 (when draft (list (cons 'xmlns:app jaunder--app-ns)))))
         (children '()))
    (when (jaunder-entry-title entry)
      (push (list 'title nil (jaunder-entry-title entry)) children))
    (when (jaunder-entry-summary entry)
      (push (list 'summary nil (jaunder-entry-summary entry)) children))
    (dolist (term (jaunder-entry-categories entry))
      (push (list 'category (list (cons 'term term))) children))
    (push (list 'content
                (list (cons 'type (jaunder-entry-content-type entry)))
                (or (jaunder-entry-body entry) ""))
          children)
    (when (jaunder-entry-published entry)
      (push (list 'published nil (jaunder-entry-published entry)) children))
    (when draft
      (push (list 'app:control nil (list 'app:draft nil "yes")) children))
    (with-temp-buffer
      ;; `dom-print' escapes unconditionally; the HTML/XML flag would only
      ;; change boolean-attribute handling, which none of these elements use,
      ;; so the single-arg call keeps output identical while staying portable.
      (dom-print (append (list 'entry attrs) (nreverse children)))
      (buffer-string))))

(defun jaunder--atom-direct-elements (node tag)
  "Return NODE's direct child elements named TAG, in document order."
  (cl-remove-if-not (lambda (child) (and (listp child) (eq (car child) tag)))
                    (dom-children node)))

(defun jaunder--harvest-response-fields (xml)
  "Harvest compatible metadata fields from an AtomPub response entry XML.
The existing singular `content-src', `content-type', `slug', and `published'
keys retain their first direct-child values.  The ordered plural `titles',
`categories', `summaries', `content-nodes', `drafts', `published-values',
`edit-uris', and `slugs' keys expose all direct-child values for Member parsing.
`content-nodes' deliberately retains DOM nodes for the later text/XHTML
projection.  `app:draft' values are collected only from direct `app:control'
children and their direct `app:draft' children; edit URIs come only from direct
`link rel=\"edit\"' children.  No Member-required cardinality is enforced here,
so media and publish responses remain valid.
`libxml-parse-xml-region' folds namespaces to local element symbols, so Atom,
app, and Jaunder extension elements are selected by their local names."
  (let* ((dom (with-temp-buffer
                (insert xml)
                (libxml-parse-xml-region (point-min) (point-max))))
         (titles (jaunder--atom-direct-elements dom 'title))
         (categories (jaunder--atom-direct-elements dom 'category))
         (summaries (jaunder--atom-direct-elements dom 'summary))
         (content-nodes (jaunder--atom-direct-elements dom 'content))
         (controls (jaunder--atom-direct-elements dom 'control))
         (published-values (jaunder--atom-direct-elements dom 'published))
         (links (jaunder--atom-direct-elements dom 'link))
         (slugs (jaunder--atom-direct-elements dom 'slug))
         (drafts (apply #'append
                        (mapcar (lambda (control)
                                  (jaunder--atom-direct-elements control 'draft))
                                controls)))
         (edit-links (cl-remove-if-not
                      (lambda (link) (equal (dom-attr link 'rel) "edit"))
                      links))
         (content (car content-nodes))
         (slug (car slugs))
         (published (car published-values)))
    (list (cons 'content-src (dom-attr content 'src))
          (cons 'content-type (dom-attr content 'type))
          (cons 'slug (and slug (dom-text slug)))
          (cons 'published (and published (dom-text published)))
          (cons 'titles (mapcar #'dom-text titles))
          (cons 'categories (mapcar (lambda (category) (dom-attr category 'term))
                                    categories))
          (cons 'summaries (mapcar #'dom-text summaries))
          (cons 'content-nodes content-nodes)
          (cons 'drafts (mapcar #'dom-text drafts))
          (cons 'published-values (mapcar #'dom-text published-values))
          (cons 'edit-uris (mapcar (lambda (link) (dom-attr link 'href))
                                   edit-links))
          (cons 'slugs (mapcar #'dom-text slugs)))))

(provide 'jaunder-atom)
;;; jaunder-atom.el ends here
