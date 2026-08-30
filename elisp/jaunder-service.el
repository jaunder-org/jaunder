;;; jaunder-service.el --- Jaunder service-document capability probe -*- lexical-binding: t; -*-

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
;; Read the AtomPub Service Document for capabilities used by the Emacs Protocol
;; Client.  Publish probes and caches the `format-media-type' extension feature.
;; New Post creation separately reads the Posts Collection's inline categories
;; for best-effort Tag completion without caching the server's catalog.

;;; Code:

(require 'cl-lib)
(require 'dom)
(require 'subr-x)
(require 'jaunder-config)
(require 'jaunder-entry)
(require 'jaunder-transport)
(require 'jaunder-warn)

(defvar jaunder--service-doc-cache nil
  "Session-scoped alist of BASE-URL -> list of advertised feature tokens.
Populated on the first successful service-doc fetch per base-url; failures are
not cached, so a later publish may retry.  Reset only by restarting Emacs.")

(defconst jaunder--entry-content-type "application/atom+xml;type=entry"
  "AtomPub media type accepted by Jaunder's Posts Collection.")

(defun jaunder--parse-service-document (body)
  "Parse BODY into an AtomPub Service Document DOM, or return `unknown'."
  (condition-case nil
      (with-temp-buffer
        (insert (or body ""))
        (let ((dom (libxml-parse-xml-region (point-min) (point-max))))
          (if (and dom (eq (dom-tag dom) 'service))
              dom
            'unknown)))
    (error 'unknown)))

(defun jaunder--service-features (dom)
  "Return the extension feature tokens advertised by service DOM."
  (let* ((extension (car (dom-by-tag dom 'extension)))
         (features (and extension (dom-attr extension 'features))))
    (if features (split-string features) '())))

(defun jaunder--parse-service-features (body)
  "Parse service-doc BODY into its advertised feature tokens.
Returns the list of tokens, `()' when the service document parses but advertises
none, or the symbol `unknown' when BODY is not an AtomPub Service Document."
  (let ((dom (jaunder--parse-service-document body)))
    (if (eq dom 'unknown)
        'unknown
      (jaunder--service-features dom))))

(defun jaunder--service-tags (dom)
  "Return DOM's Posts Collection Tag slugs, or `unknown'.
The Posts Collection is the unique collection accepting
`jaunder--entry-content-type'.  Only valid canonical Tag slugs from its inline
categories are returned; categories from other Collections are ignored."
  (let ((posts
         (cl-remove-if-not
          (lambda (collection)
            (cl-some
             (lambda (accept)
               (equal (string-trim (dom-text accept))
                      jaunder--entry-content-type))
             (dom-by-tag collection 'accept)))
          (dom-by-tag dom 'collection))))
    (if (/= (length posts) 1)
        'unknown
      (let ((terms
             (mapcar (lambda (category) (dom-attr category 'term))
                     (dom-by-tag (car posts) 'category))))
        (if (cl-every #'jaunder--valid-tag-slug-p terms)
            terms
          'unknown)))))


(defun jaunder--fetch-service-document (base-url)
  "Fetch BASE-URL's AtomPub Service Document DOM, or return `unknown'.
Transport errors, non-2xx responses, and invalid documents never signal."
  (condition-case nil
      (let* ((response
              (jaunder--http-request
               "GET" (jaunder--build-url base-url "atompub" "service")))
             (status (plist-get response :status)))
        (if (and (integerp status) (<= 200 status 299))
            (jaunder--parse-service-document (plist-get response :body))
          'unknown))
    (error 'unknown)))

(defun jaunder--fetch-service-tags (base-url)
  "Fetch BASE-URL's Posts Collection Tags, or return `unknown'."
  (let ((dom (jaunder--fetch-service-document base-url)))
    (if (eq dom 'unknown)
        'unknown
      (jaunder--service-tags dom))))

(defun jaunder--fetch-service-features (base-url)
  "Fetch BASE-URL's extension feature tokens, or return `unknown'."
  (let ((dom (jaunder--fetch-service-document base-url)))
    (if (eq dom 'unknown)
        'unknown
      (jaunder--service-features dom))))

(defun jaunder--warn-missing-format-media-type (base-url)
  "Warn once per session per BASE-URL when format-media-type is unadvertised.
Fetches and caches the capability on the first call per base-url; a cache hit
does nothing (no fetch, no warning), so it warns at most once per blog per
session.  A fetch or parse failure is neither cached nor warned on.  Gated by
`jaunder-warn-missing-format-media-type'."
  (when (and jaunder-warn-missing-format-media-type
             (not (assoc base-url jaunder--service-doc-cache)))
    (let ((features (jaunder--fetch-service-features base-url)))
      (unless (eq features 'unknown)
        (push (cons base-url features) jaunder--service-doc-cache)
        (unless (member "format-media-type" features)
          (jaunder--warn
           "server at %s does not advertise the format-media-type feature; it may store this post's org source verbatim instead of rendering it"
           base-url))))))

(provide 'jaunder-service)
;;; jaunder-service.el ends here
