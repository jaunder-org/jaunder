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
;; Probe the AtomPub service document for advertised capabilities and warn at
;; publish when the server does not advertise the `format-media-type' extension
;; feature — a vanilla server would store the post's org source verbatim instead
;; of rendering it.  The capability is fetched at most once per base-url per
;; session (cached), and the check is best-effort: any failure skips silently.

;;; Code:

(require 'dom)
(require 'jaunder-config)
(require 'jaunder-transport)
(require 'jaunder-warn)

(defvar jaunder--service-doc-cache nil
  "Session-scoped alist of BASE-URL -> list of advertised feature tokens.
Populated on the first successful service-doc fetch per base-url; failures are
not cached, so a later publish may retry.  Reset only by restarting Emacs.")

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
