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
(require 'jaunder-transport)
(require 'jaunder-warn)

(defvar jaunder--service-doc-cache nil
  "Session-scoped alist of BASE-URL -> list of advertised feature tokens.
Populated on the first successful service-doc fetch per base-url; failures are
not cached, so a later publish may retry.  Reset only by restarting Emacs.")

(defconst jaunder--entry-content-type "application/atom+xml;type=entry"
  "AtomPub media type accepted by Jaunder's Posts Collection.")

(defun jaunder--parse-service-features (body)
  "Parse service-doc BODY into its advertised feature tokens.
Returns the list of tokens, `()' when the service document parses but advertises
none, or the symbol `unknown' when BODY is not a parseable AtomPub service
document.  libxml returns nil on a garbage body, and a 2xx HTML/error page from
a proxy parses to another root element; neither is a real probe, so both map to
`unknown' (skip, no cache) rather than a false negative.  The extension element
is matched by local name (libxml folds the `j:' prefix), and its `features'
attribute is split on whitespace."
  (with-temp-buffer
    (insert (or body ""))
    (let ((dom (libxml-parse-xml-region (point-min) (point-max))))
      (if (or (null dom) (not (eq (dom-tag dom) 'service)))
          'unknown
        (let* ((ext (car (dom-by-tag dom 'extension)))
               (features (and ext (dom-attr ext 'features))))
          (if features (split-string features) '()))))))

(defun jaunder--parse-service-tags (body)
  "Return BODY's Posts Collection Tag slugs, or `unknown'.
The Posts Collection is the unique collection accepting
`jaunder--entry-content-type'.  Only its inline category terms are returned;
categories from other collections are ignored.  A valid document with no
categories returns `()'."
  (with-temp-buffer
    (insert (or body ""))
    (let ((dom (libxml-parse-xml-region (point-min) (point-max))))
      (if (or (null dom) (not (eq (dom-tag dom) 'service)))
          'unknown
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
              (if (cl-every
                   (lambda (term)
                     (and (stringp term) (not (string-empty-p term))))
                   terms)
                  terms
                'unknown))))))))

(defun jaunder--fetch-service-tags (base-url)
  "Fetch BASE-URL's Service Document and return Posts Collection Tags.
Returns `unknown' on a transport, HTTP, or parse failure.  Failures never
signal because Tag completion is optional for local Post creation."
  (condition-case nil
      (let* ((response
              (jaunder--http-request
               "GET" (jaunder--build-url base-url "atompub" "service")))
             (status (plist-get response :status)))
        (if (and (integerp status) (<= 200 status 299))
            (jaunder--parse-service-tags (plist-get response :body))
          'unknown))
    (error 'unknown)))

(defun jaunder--fetch-service-features (base-url)
  "Fetch and parse BASE-URL's AtomPub service document.
Returns a list of feature tokens, `()', or the symbol `unknown' on any
transport, non-2xx, or parse failure.  Never signals, so a probe can never
abort a publish."
  (condition-case nil
      (let* ((resp (jaunder--http-request
                    "GET" (jaunder--build-url base-url "atompub" "service")))
             (status (plist-get resp :status)))
        (if (and (integerp status) (<= 200 status 299))
            (jaunder--parse-service-features (plist-get resp :body))
          'unknown))
    (error 'unknown)))

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
