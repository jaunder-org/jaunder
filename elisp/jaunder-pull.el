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
(require 'jaunder-config)
(require 'jaunder-reconcile)
(require 'jaunder-pull-media)

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


(defun jaunder--pull-edit-id (uri)
  "Return decimal terminal Post ID from edit URI, or nil."
  (when (stringp uri)
    (let* ((parsed (url-generic-parse-url uri))
           (path (url-filename parsed)))
      (when (and (stringp path) (string-match "/\\([0-9]+\\)\\'" path))
        (match-string 1 path)))))

(defconst jaunder--pull-rfc-3339-offset-regexp ;; cov:ignore: computed concat initializer has no Edebug execution stop
  (concat "\\`\\([0-9]\\{4\\}\\)-\\([0-9]\\{2\\}\\)-\\([0-9]\\{2\\}\\)"
          "T\\([0-9]\\{2\\}\\):\\([0-9]\\{2\\}\\):\\([0-9]\\{2\\}\\)"
          "\\(?:\\.[0-9]+\\)?\\(?:Z\\|[+-]\\([0-9]\\{2\\}\\):\\([0-9]\\{2\\}\\)\\)\\'")
  "RFC-3339 timestamp shape with captured calendar and numeric-offset fields.")

(defconst jaunder--pull-xhtml-ns "http://www.w3.org/1999/xhtml"
  "Namespace required on an Atom XHTML content wrapper.")

(defun jaunder--pull-control-character-p (character)
  "Return non-nil when CHARACTER is a Unicode control character."
  (eq (get-char-code-property character 'general-category) 'Cc))

(defun jaunder--pull-rfc-3339-components-valid-p (published)
  "Return non-nil when matched RFC-3339 PUBLISHED fields are semantically valid.
This rejects values `date-to-time' would normalize, including impossible
Gregorian dates and out-of-range numeric offsets."
  (let* ((year (string-to-number (match-string 1 published)))
         (month (string-to-number (match-string 2 published)))
         (day (string-to-number (match-string 3 published)))
         (hour (string-to-number (match-string 4 published)))
         (minute (string-to-number (match-string 5 published)))
         (second (string-to-number (match-string 6 published)))
         (offset-hour (match-string 7 published))
         (offset-minute (match-string 8 published))
         (days-in-month
          (and (<= 1 month 12)
               (aref [0 31 28 31 30 31 30 31 31 30 31 30 31] month))))
    (when (and (= month 2)
               (or (= 0 (% year 400))
                   (and (= 0 (% year 4))
                        (/= 0 (% year 100)))))
      (setq days-in-month 29))
    (and days-in-month
         (<= 1 day days-in-month)
         (<= 0 hour 23)
         (<= 0 minute 59)
         (<= 0 second 59)
         (or (null offset-hour)
             (and (<= (string-to-number offset-hour) 23)
                  (<= (string-to-number offset-minute) 59))))))

(defun jaunder--pull-rfc-3339-time (published)
  "Parse semantically valid offset-qualified RFC-3339 PUBLISHED text.
The original text remains the wire value; the parsed time is used only for
status and local-date projection."
  (unless (and (stringp published)
               (string-match jaunder--pull-rfc-3339-offset-regexp published)
               (jaunder--pull-rfc-3339-components-valid-p published))
    (jaunder--pull-error "Member published value must be offset-qualified RFC-3339"))
  (condition-case nil
      (date-to-time published)
    (error (jaunder--pull-error
            "Member published value must be RFC-3339"))))

(defun jaunder--safe-pull-slug-p (slug)
  "Return non-nil when SLUG names one safe direct-child path component."
  (and (stringp slug)
       (not (string-empty-p slug))
       (not (member slug '("." "..")))
       (equal slug (file-name-nondirectory slug))
       (not (string-match-p "[\\\\/]" slug))
       (not (cl-some #'jaunder--pull-control-character-p slug))))

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

(defun jaunder--pull-xhtml-wrapper (content)
  "Return CONTENT's sole XHTML div after validating its direct children."
  (let* ((children (dom-children content))
         (divs (jaunder--atom-direct-elements content 'div)))
    (unless (and (= (length divs) 1)
                 (cl-every
                  (lambda (child)
                    (or (eq child (car divs))
                        (and (stringp child)
                             (string-match-p "\\`[ \t\r\n]*\\'" child))))
                  children))
      (jaunder--pull-error
       "xhtml content must contain one div and only surrounding XML whitespace"))
    (let ((div (car divs)))
      (unless (equal (dom-attr div 'xmlns) jaunder--pull-xhtml-ns)
        (jaunder--pull-error "xhtml div wrapper must use the XHTML namespace"))
      div)))

(defun jaunder--pull-content-body (content kind)
  "Return native source body from CONTENT projected according to KIND."
  (pcase kind
    ('text (or (dom-text content) ""))
    ('xhtml
     (mapconcat #'jaunder--serialize-xhtml-node
                (dom-children (jaunder--pull-xhtml-wrapper content)) ""))
    (_ (jaunder--pull-error "unknown content projection"))))

(defun jaunder--pull-header-lines (name value)
  "Return repeated Org header NAME lines for LF-delimited VALUE."
  (unless (string-empty-p value)
    (mapcar (lambda (line) (format "#+%s: %s" name line))
            (split-string value "\n" nil))))

(cl-defstruct (jaunder-pulled-member (:constructor jaunder--make-pulled-member))
              "Validated Member data shared by rendering and pull localization."
              org-prefix org format body)

(defun jaunder--parse-pulled-member (entry-xml etag captured-at zone)
  "Parse Member ENTRY-XML once into exact Org bytes and native source fields.
ETAG, CAPTURED-AT, and ZONE have the same validation and projection semantics
as `jaunder--atom->org'.  This function performs no network or filesystem I/O."
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
      (let ((published-time (jaunder--pull-rfc-3339-time published)))
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
      (let ((org-prefix (concat (mapconcat #'identity lines "\n") "\n\n")))
        (jaunder--make-pulled-member
         :org-prefix org-prefix
         :org (concat org-prefix body)
         :format format
         :body body)))))

(defun jaunder--atom->org (entry-xml etag captured-at zone)
  "Map Member ENTRY-XML to exact Org bytes using ETAG, CAPTURED-AT, and ZONE."
  (jaunder-pulled-member-org
   (jaunder--parse-pulled-member entry-xml etag captured-at zone)))

(defun jaunder--render-pulled-member (member body)
  "Render MEMBER's exact Org header bytes with replacement native BODY."
  (unless (stringp body)
    (jaunder--pull-error "localized Member body must be a string"))
  (concat (jaunder-pulled-member-org-prefix member) body))


(cl-defstruct (jaunder-pull-result (:constructor jaunder--make-pull-result))
              "Outcome of one D3-facing server-only pull."
              status path)

(defun jaunder--pull-destination (root slug)
  "Return exact direct-child Org destination under ROOT for SLUG."
  (unless (jaunder--safe-pull-slug-p slug)
    (jaunder--pull-error "Member j:slug must name one safe path component"))
  (let* ((directory (file-name-as-directory (expand-file-name root)))
         (path (expand-file-name (concat slug ".org") directory)))
    (unless (equal (file-name-directory path) directory)
      (jaunder--pull-error "pull destination must be directly under the root")) ;; cov:ignore: a validated safe leaf passed to expand-file-name cannot escape its just-derived parent
    path))

(defun jaunder--pull-destination-exists-p (path)
  "Return non-nil when PATH already has any filesystem directory entry."
  (or (file-exists-p path) (file-symlink-p path)))

(defun jaunder--pull-response-identity (entry-xml)
  "Return (ID . SLUG) from complete response ENTRY-XML."
  (let* ((fields (jaunder--harvest-response-fields entry-xml))
         (edit-uri (jaunder--pull-exactly-one fields 'edit-uris "edit URI"))
         (slug (jaunder--pull-exactly-one fields 'slugs "j:slug"))
         (id (jaunder--pull-edit-id edit-uri)))
    (unless id
      (jaunder--pull-error "Member edit URI must end in a decimal Post ID"))
    (cons id slug)))

(defun jaunder--pull-member-instance-id (response)
  "Return RESPONSE's sole canonical Jaunder instance UUID."
  (let ((instances (jaunder--pull-media-header-values response "x-jaunder-instance")))
    (unless (and (= (length instances) 1)
                 (string-match-p jaunder--pull-media-instance-id-regexp
                                 (car instances)))
      (jaunder--pull-error
       "Member response must carry exactly one canonical X-Jaunder-Instance UUID"))
    (car instances)))

(defun jaunder--install-pulled-bytes (path bytes)
  "Install BYTES at PATH without overwrite; return a pull result.
Writes a complete same-directory temporary file, then claims PATH by hard-link
creation, which is atomic and fails if another directory entry won the race."
  (if (jaunder--pull-destination-exists-p path)
      (jaunder--make-pull-result :status 'blocked :path path)
    (let ((temporary nil))
      (unwind-protect
          (progn
            (setq temporary
                  (make-temp-file
                   (expand-file-name ".jaunder-pull-" (file-name-directory path))))
            (let ((coding-system-for-write 'utf-8-unix))
              (write-region bytes nil temporary nil 'silent))
            (condition-case err
                (progn
                  (add-name-to-file temporary path)
                  (jaunder--make-pull-result :status 'pulled :path path))
              (file-already-exists
               (jaunder--make-pull-result :status 'blocked :path path))
              (file-error
               (if (jaunder--pull-destination-exists-p path)
                   (jaunder--make-pull-result :status 'blocked :path path)
                 (signal (car err) (cdr err))))))
        (when (and temporary (file-exists-p temporary))
          (delete-file temporary))))))

(defun jaunder--pull-member (root member)
  "Pull D1 inventory MEMBER into ROOT, returning `jaunder-pull-result'.
An existing destination blocks before any Member or media I/O.  A complete
localized Post is installed only after every Local Media Copy verifies."
  (unless (jaunder-inventory-member-p member)
    (jaunder--pull-error "pull input must be a D1 inventory Member"))
  (let* ((slug (jaunder-inventory-member-slug member))
         (path (jaunder--pull-destination root slug)))
    (if (jaunder--pull-destination-exists-p path)
        (jaunder--make-pull-result :status 'blocked :path path)
      (jaunder--with-blog root
                          (let ((response
                                 (jaunder--http-request
                                  "GET" (jaunder-inventory-member-edit-uri member))))
                            (unless (and (integerp (plist-get response :status))
                                         (<= 200 (plist-get response :status) 299))
                              (jaunder--pull-error "Member GET returned non-2xx status"))
                            (let* ((entry-xml (plist-get response :body))
                                   (identity (jaunder--pull-response-identity entry-xml))
                                   (instance-id (jaunder--pull-member-instance-id response)))
                              (unless (and (equal (car identity) (jaunder-inventory-member-id member))
                                           (equal (cdr identity) slug))
                                (jaunder--pull-error "Member response identity changed since inventory"))
                              (let* ((captured-at (current-time))
                                     (zone (jaunder--current-zone-name))
                                     (etag (jaunder--response-header response "ETag"))
                                     (pulled-member
                                      (jaunder--parse-pulled-member entry-xml etag captured-at zone))
                                     (plan
                                      (jaunder--pull-media-plan
                                       (jaunder-pulled-member-format pulled-member)
                                       (jaunder-pulled-member-body pulled-member)
                                       (jaunder--active-base-url))))
                                ;; A Post is the final durable claim: verified copies can safely
                                ;; survive a late failure and make the next reconcile retry cheaper.
                                (jaunder--pull-media-materialize root instance-id plan)
                                (jaunder--install-pulled-bytes
                                 path
                                 (jaunder--render-pulled-member
                                  pulled-member (jaunder--pull-media-apply-plan plan))))))))))
(provide 'jaunder-pull)
;;; jaunder-pull.el ends here
