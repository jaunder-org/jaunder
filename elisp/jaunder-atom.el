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

(require 'cl-lib) ;; cov:ignore: load-time dependency declaration has no Edebug execution stop
(require 'dom) ;; cov:ignore: load-time dependency declaration has no Edebug execution stop
(require 'xml) ;; cov:ignore: load-time dependency declaration has no Edebug execution stop
(require 'jaunder-entry) ;; cov:ignore: load-time dependency declaration has no Edebug execution stop

(defconst jaunder--atom-ns "http://www.w3.org/2005/Atom" ;; cov:ignore: constant declaration has no Edebug execution stop
  "The Atom namespace URI.")

(defconst jaunder--app-ns "http://www.w3.org/2007/app" ;; cov:ignore: constant declaration has no Edebug execution stop
  "The Atom Publishing Protocol namespace URI (`app:control'/`app:draft').")

(defconst jaunder--atompub-ns "https://jaunder.org/ns/atompub" ;; cov:ignore: constant declaration has no Edebug execution stop
  "The Jaunder AtomPub extension namespace URI.")


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

(defun jaunder--atom-local-name (tag)
  "Return TAG's local XML name as a symbol."
  (intern (car (last (split-string (symbol-name tag) ":")))))

(defun jaunder--atom-namespace-context (node inherited)
  "Return NODE's in-scope namespace bindings over INHERITED.
The result maps prefix symbols (or nil for the default namespace) to namespace
URI strings.  A declaration on NODE shadows a binding inherited from its
parent."
  (let ((bindings (copy-sequence inherited)))
    (dolist (attribute (cadr node) bindings)
      (let ((name (symbol-name (car attribute))))
        (cond
         ((equal name "xmlns")
          (setq bindings (assq-delete-all nil bindings))
          (push (cons nil (cdr attribute)) bindings))
         ((string-prefix-p "xmlns:" name)
          (let ((prefix (intern (substring name (length "xmlns:")))))
            (setq bindings (assq-delete-all prefix bindings))
            (push (cons prefix (cdr attribute)) bindings))))))))

(defun jaunder--atom-element-namespace (node namespaces)
  "Return NODE's resolved namespace URI using in-scope NAMESPACES."
  (let* ((parts (split-string (symbol-name (car node)) ":"))
         (prefix (and (cdr parts) (intern (car parts)))))
    (cdr (assq prefix namespaces))))

(defun jaunder--atom-direct-elements-in-namespace (node tag namespace inherited)
  "Return NODE's direct TAG children resolved to NAMESPACE, in document order.
INHERITED is the namespace context in scope on NODE.  Each child is resolved
against that context plus declarations on the child itself, so documents may
use arbitrary prefixes or redeclare a prefix at any direct-child boundary."
  (cl-remove-if-not
   (lambda (child)
     (and (listp child)
          (eq (jaunder--atom-local-name (car child)) tag)
          (equal (jaunder--atom-element-namespace
                  child (jaunder--atom-namespace-context child inherited))
                 namespace)))
   (dom-children node)))

(defun jaunder--atom-direct-elements (node tag)
  "Return NODE's direct child elements with local name TAG, in document order.
This intentionally ignores namespaces for the XHTML wrapper validator, which
checks that wrapper's namespace separately."
  (cl-remove-if-not
   (lambda (child)
     (and (listp child)
          (eq (jaunder--atom-local-name (car child)) tag)))
   (dom-children node)))


(defun jaunder--harvest-response-fields (xml)
  "Harvest compatible metadata fields from an AtomPub response entry XML.
The existing singular `content-src', `content-type', `slug', and `published'
keys retain their first direct-child values.  The ordered plural `titles',
`categories', `summaries', `content-nodes', `drafts', `published-values',
`edit-uris', and `slugs' keys expose all direct-child values for Member parsing.
`content-nodes' deliberately retains DOM nodes for the later text/XHTML
projection.  Atom metadata is accepted only from the Atom namespace,
`app:control'/`app:draft' only from APP, and `slug' only from the Jaunder
extension namespace.  No Member-required cardinality is enforced here, so media
and publish responses remain valid."
  (let* ((dom (with-temp-buffer
                (insert xml)
                (car (xml-parse-region (point-min) (point-max)))))
         (entry-namespaces (jaunder--atom-namespace-context dom nil))
         (titles (jaunder--atom-direct-elements-in-namespace
                  dom 'title jaunder--atom-ns entry-namespaces))
         (categories (jaunder--atom-direct-elements-in-namespace
                      dom 'category jaunder--atom-ns entry-namespaces))
         (summaries (jaunder--atom-direct-elements-in-namespace
                     dom 'summary jaunder--atom-ns entry-namespaces))
         (content-nodes (jaunder--atom-direct-elements-in-namespace
                         dom 'content jaunder--atom-ns entry-namespaces))
         (controls (jaunder--atom-direct-elements-in-namespace
                    dom 'control jaunder--app-ns entry-namespaces))
         (published-values (jaunder--atom-direct-elements-in-namespace
                            dom 'published jaunder--atom-ns entry-namespaces))
         (links (jaunder--atom-direct-elements-in-namespace
                 dom 'link jaunder--atom-ns entry-namespaces))
         (slugs (jaunder--atom-direct-elements-in-namespace
                 dom 'slug jaunder--atompub-ns entry-namespaces))
         (drafts (apply #'append
                        (mapcar
                         (lambda (control)
                           (jaunder--atom-direct-elements-in-namespace
                            control 'draft jaunder--app-ns
                            (jaunder--atom-namespace-context
                             control entry-namespaces)))
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

(provide 'jaunder-atom) ;; cov:ignore: feature declaration has no Edebug execution stop
;;; jaunder-atom.el ends here
