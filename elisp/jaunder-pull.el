;;; jaunder-pull.el --- Deterministic AtomPub Member pull -*- lexical-binding: t; -*-

;; Copyright (C) 2026 Jaunder contributors

;;; Commentary:
;; Validate a complete AtomPub Member Entry and synthesize exact Org bytes.  The
;; filesystem/network pull operation is added separately; this module's mapping
;; seam is pure when given its ETag, captured clock, and zone.

;;; Code:

(require 'cl-lib)
(require 'dom)
(require 'url-parse)
(require 'xml)
(require 'jaunder-atom)
(require 'jaunder-datetime)

(defun jaunder--pull-error (invariant)
  "Signal a pull mapping error naming broken INVARIANT."
  (error "jaunder pull: %s" invariant))

(defun jaunder--pull-exactly-one (fields key description)
  "Return sole FIELDS value under KEY or signal DESCRIPTION."
  (let ((values (cdr (assq key fields))))
    (if (= (length values) 1)
        (car values)
      (jaunder--pull-error (format "Member must have exactly one %s" description)))))

(defun jaunder--pull-at-most-one (fields key description)
  "Return optional sole FIELDS value under KEY or signal DESCRIPTION."
  (let ((values (cdr (assq key fields))))
    (if (<= (length values) 1)
        (car values)
      (jaunder--pull-error (format "Member must have at most one %s" description)))))

(defun jaunder--strong-etag-p (etag)
  "Return non-nil when ETAG is a strong quoted entity tag."
  (and (stringp etag)
       (not (string-prefix-p "W/" etag))
       (string-match-p "\\`\"[^\"\r\n]+\"\\'" etag)))

(defun jaunder--pull-edit-id (uri)
  "Return decimal terminal Post ID from edit URI, or nil."
  (when (stringp uri)
    (let* ((parsed (url-generic-parse-url uri))
           (path (url-filename parsed)))
      (when (and (stringp path) (string-match "/\\([0-9]+\\)\\'" path))
        (match-string 1 path)))))

(defun jaunder--safe-pull-slug-p (slug)
  "Return non-nil when SLUG names one direct child path component."
  (and (stringp slug)
       (not (string-empty-p slug))
       (not (member slug '("." "..")))
       (equal slug (file-name-nondirectory slug))
       (not (string-match-p "[\\\\/]" slug))))

(defun jaunder--pull-content-format (content)
  "Return (FORMAT . KIND) for CONTENT or signal on its wire type.
KIND is `text' or `xhtml'."
  (let* ((raw (dom-attr content 'type))
         (wire (and raw (downcase (string-trim (car (split-string raw ";")))))))
    (pcase wire
      ("text/org" '("org" . text))
      ("text/markdown" '("markdown" . text))
      ((or "html" "text/html") '("html" . text))
      ("xhtml" '("html" . xhtml))
      (_ (jaunder--pull-error "Member content has an unrecognized format")))))

(defun jaunder--serialize-xhtml-node (node)
  "Return canonical XML serialization of one XHTML child NODE."
  (if (stringp node)
      (xml-escape-string node)
    (with-temp-buffer
      (dom-print node)
      (buffer-string))))

(defun jaunder--pull-content-body (content kind)
  "Return native source body from CONTENT projected according to KIND."
  (pcase kind
    ('text (or (dom-text content) ""))
    ('xhtml
     (let ((divs (jaunder--atom-direct-elements content 'div)))
       (unless (= (length divs) 1)
         (jaunder--pull-error "xhtml content must have exactly one div wrapper"))
       (mapconcat #'jaunder--serialize-xhtml-node (dom-children (car divs)) "")))
    (_ (jaunder--pull-error "unknown content projection"))))

(defun jaunder--pull-header-lines (name value)
  "Return repeated Org header NAME lines for LF-delimited VALUE."
  (unless (string-empty-p value)
    (mapcar (lambda (line) (format "#+%s: %s" name line))
            (split-string value "\n" nil))))

(defun jaunder--atom->org (entry-xml etag captured-at zone)
  "Map Member ENTRY-XML to exact Org bytes using ETAG, CAPTURED-AT, and ZONE.
ETAG is stored verbatim.  CAPTURED-AT is an Emacs time value used both for
status classification and the UTC sync stamp.  ZONE is recorded and used to
render the local Org date.  This function performs no network or filesystem I/O."
  (unless (jaunder--strong-etag-p etag)
    (jaunder--pull-error "Member response must carry a strong quoted ETag"))
  (unless (and (stringp zone) (not (string-empty-p zone)))
    (jaunder--pull-error "pull zone must be non-empty"))
  (let* ((fields (jaunder--harvest-response-fields entry-xml))
         (title (jaunder--pull-exactly-one fields 'titles "title"))
         (content (jaunder--pull-exactly-one fields 'content-nodes "content"))
         (edit-uri (jaunder--pull-exactly-one fields 'edit-uris "edit URI"))
         (slug (jaunder--pull-exactly-one fields 'slugs "j:slug"))
         (summary (jaunder--pull-at-most-one fields 'summaries "summary"))
         (draft-value (jaunder--pull-at-most-one fields 'drafts "app:draft"))
         (published (jaunder--pull-at-most-one fields 'published-values "published"))
         (id (jaunder--pull-edit-id edit-uri))
         (format-kind (jaunder--pull-content-format content))
         (format (car format-kind))
         (body (jaunder--pull-content-body content (cdr format-kind)))
         (draft (cond ((null draft-value) nil)
                      ((equal draft-value "yes") t)
                      ((equal draft-value "no") nil)
                      (t (jaunder--pull-error "app:draft must be yes or no"))))
         status date-line date-tz date-utc)
    (unless id
      (jaunder--pull-error "Member edit URI must end in a decimal Post ID"))
    (unless (jaunder--safe-pull-slug-p slug)
      (jaunder--pull-error "Member j:slug must name one safe path component"))
    (dolist (category (cdr (assq 'categories fields)))
      (unless (and (stringp category) (not (string-empty-p category)))
        (jaunder--pull-error "Member category term must be non-empty")))
    (if draft
        (setq status "draft")
      (unless published
        (jaunder--pull-error "non-draft Member must have published"))
      (let ((published-time (condition-case nil
                                (date-to-time published)
                              (error (jaunder--pull-error
                                      "Member published value must be RFC-3339")))))
        (setq status (if (time-less-p captured-at published-time)
                         "scheduled"
                       "published")
              date-line (jaunder--utc->org-date published zone)
              date-tz zone
              date-utc published)))
    (let ((lines (append
                  (jaunder--pull-header-lines "TITLE" title)
                  (when date-line (list (format "#+DATE: %s" date-line)))
                  (let ((categories (cdr (assq 'categories fields))))
                    (when categories
                      (list (format "#+KEYWORDS: %s"
                                    (mapconcat #'identity categories ", ")))))
                  (and summary (jaunder--pull-header-lines "DESCRIPTION" summary))
                  (list (format "#+PROPERTY: JAUNDER_STATUS %s" status))
                  (when date-tz
                    (list (format "#+PROPERTY: JAUNDER_DATE_TZ %s" date-tz)
                          (format "#+PROPERTY: JAUNDER_DATE_UTC %s" date-utc)))
                  (list (format "#+PROPERTY: JAUNDER_FORMAT %s" format)
                        (format "#+PROPERTY: JAUNDER_SLUG %s" slug)
                        (format "#+PROPERTY: JAUNDER_ID %s" id)
                        (format "#+PROPERTY: JAUNDER_SYNCED %s" etag)
                        (format "#+PROPERTY: JAUNDER_SYNCED_AT %s"
                                (format-time-string "%Y-%m-%dT%H:%M:%SZ"
                                                    captured-at t))))))
      (concat (mapconcat #'identity lines "\n") "\n\n" body))))

(provide 'jaunder-pull)
;;; jaunder-pull.el ends here
