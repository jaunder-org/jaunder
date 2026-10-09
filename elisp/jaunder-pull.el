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
(require 'jaunder-org)
(require 'jaunder-datetime)
(require 'jaunder-config)
(require 'jaunder-reconcile)
(require 'jaunder-pull-media)
(require 'jaunder-post-link)
(require 'jaunder-debug)

(defvar jaunder--pull-link-inventory nil
  "Optional inventory evidence supplied by a reconciliation pull staging run.")

(defvar jaunder--pull-original-proof nil
  "Optional matched-local Media evidence supplied by reconciliation staging.")

(define-error 'jaunder-pull-stage-identity-changed
              "Member response identity changed since inventory" 'error)

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

(defconst jaunder--pull-rfc-3339-offset-regexp
  "\\`\\([0-9]\\{4\\}\\)-\\([0-9]\\{2\\}\\)-\\([0-9]\\{2\\}\\)T\\([0-9]\\{2\\}\\):\\([0-9]\\{2\\}\\):\\([0-9]\\{2\\}\\)\\(?:\\.[0-9]+\\)?\\(?:Z\\|[+-]\\([0-9]\\{2\\}\\):\\([0-9]\\{2\\}\\)\\)\\'"
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
    ('text (or (dom-inner-text content) ""))
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
  org-prefix org format body audience-omitted)

(defun jaunder--parse-pulled-member (entry-xml etag captured-at zone &optional audience-capable)
  "Parse Member ENTRY-XML once into exact Org bytes and native source fields.
ETAG, CAPTURED-AT, and ZONE have the same validation and projection semantics
as `jaunder--atom->org'.  AUDIENCE-CAPABLE is the current service verdict.
This function performs no network or filesystem I/O."
  (jaunder--with-debug-operation "member.parse" ()
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
                                        (audiences
                                         (condition-case err
                                             (jaunder--synchronized-response-audiences fields audience-capable)
                                           (error (jaunder--pull-error (error-message-string err)))))
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
                                   (when (jaunder--title-has-line-separator-p title)
                                     (jaunder--pull-error "Member title must be one line"))
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
                                                 (mapcar (lambda (audience)
                                                           (format "#+PROPERTY: JAUNDER_AUDIENCE %s" audience))
                                                         audiences)
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
                                       (jaunder--debug-fields format format)
                                       (let ((member (jaunder--make-pulled-member
                                                      :org-prefix org-prefix :format format :body body
                                                      :audience-omitted (null audiences))))
                                         (setf (jaunder-pulled-member-org member)
                                               (jaunder--render-pulled-member member body))
                                         member))))))

(defun jaunder--atom->org (entry-xml etag captured-at zone &optional audience-capable)
  "Map Member ENTRY-XML to Org using ETAG, CAPTURED-AT, ZONE, and service evidence."
  (jaunder-pulled-member-org
   (jaunder--parse-pulled-member entry-xml etag captured-at zone audience-capable)))

(defun jaunder--render-pulled-member (member body)
  "Render MEMBER's exact Org header bytes with replacement native BODY."
  (jaunder--with-debug-operation "org.serialize" (format "org")
                                 (unless (stringp body)
                                   (jaunder--pull-error "localized Member body must be a string"))
                                 (concat (jaunder-pulled-member-org-prefix member) body)))


(cl-defstruct (jaunder-pull-result (:constructor jaunder--make-pull-result))
  "Outcome of one D3-facing server-only pull.
Successful results retain server-confirmed metadata for reconciliation."
  status path id slug etag synced-at http-status local-effect)

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
  (jaunder--with-debug-operation "member.identity" ()
                                 (let* ((fields (jaunder--harvest-response-fields entry-xml))
                                        (edit-uri (jaunder--pull-exactly-one fields 'edit-uris "edit URI"))
                                        (slug (jaunder--pull-exactly-one fields 'slugs "j:slug"))
                                        (id (jaunder--pull-edit-id edit-uri)))
                                   (unless id
                                     (jaunder--pull-error "Member edit URI must end in a decimal Post ID"))
                                   (cons id slug))))

(defun jaunder--pull-member-instance-id (response)
  "Return RESPONSE's sole canonical Jaunder instance UUID."
  (let ((instances (jaunder--pull-media-header-values response "x-jaunder-instance")))
    (unless (and (= (length instances) 1)
                 (string-match-p jaunder--pull-media-instance-id-regexp
                                 (car instances)))
      (jaunder--pull-error
       "Member response must carry exactly one canonical X-Jaunder-Instance UUID"))
    (car instances)))

(defun jaunder--pull-write-checkpoint (path bytes &optional staged-synced-at)
  "Write BYTES to temporary PATH, returning its installed sync timestamp.
When STAGED-SYNCED-AT is supplied, renew that canonical header and align the
file mtime with the same checkpoint.  Without it, write exact BYTES."
  (let* ((checkpoint (and staged-synced-at (current-time)))
         (synced-at (and checkpoint
                         (format-time-string "%Y-%m-%dT%H:%M:%SZ" checkpoint t))))
    (with-temp-buffer
      (insert bytes)
      (when staged-synced-at
        (goto-char (point-min))
        (let ((header-end (search-forward "\n\n" nil t)))
          (unless header-end
            (jaunder--pull-error "staged Post has no header boundary"))
          (goto-char (point-min))
          (unless (re-search-forward
                   (concat "^#\\+PROPERTY: JAUNDER_SYNCED_AT "
                           (regexp-quote staged-synced-at) "$") header-end t)
            (jaunder--pull-error "staged Post sync timestamp does not match its header"))
          (replace-match (concat "#+PROPERTY: JAUNDER_SYNCED_AT " synced-at) t t)))
      (let ((coding-system-for-write 'utf-8-unix))
        (write-region (point-min) (point-max) path nil 'silent)))
    ;; Disk writes may themselves be slow.  Match the private temporary file to
    ;; its checkpoint before installation, without backdating an authored edit.
    (when checkpoint (set-file-times path checkpoint))
    synced-at))

(defun jaunder--install-pulled-bytes (path bytes &optional staged-synced-at)
  "Install BYTES at PATH without overwrite; return a pull result.
STAGED-SYNCED-AT renews the sync checkpoint immediately before installation.
Writes a complete same-directory temporary file, then claims PATH by hard-link
creation, which is atomic and fails if another directory entry won the race."
  (if (jaunder--pull-destination-exists-p path)
      (jaunder--make-pull-result :status 'blocked :path path)
    (let ((temporary nil)
          synced-at)
      (unwind-protect
          (progn
            (setq temporary
                  (make-temp-file
                   (expand-file-name ".jaunder-pull-" (file-name-directory path))))
            (setq synced-at (jaunder--pull-write-checkpoint
                             temporary bytes staged-synced-at))
            (condition-case err
                (progn
                  (add-name-to-file temporary path)
                  (jaunder--make-pull-result :status 'pulled :path path :synced-at synced-at))
              (file-already-exists
               (jaunder--make-pull-result :status 'blocked :path path))
              (file-error
               (if (jaunder--pull-destination-exists-p path)
                   (jaunder--make-pull-result :status 'blocked :path path)
                 (signal (car err) (cdr err))))))
        (when (and temporary (file-exists-p temporary))
          (delete-file temporary))))))

(defun jaunder--pull-stage-member (root member &optional original-proof)
  "Fetch, verify, and localize MEMBER, returning staged replacement data.
When ORIGINAL-PROOF is non-nil, retain its verified Media stage for the matched
consumer's final original-file revalidation.  Ordinary server-only callers
finalize Local Media Copies here.  The caller owns the final destination safety
check and installation.  `jaunder--pull-link-inventory' supplies the shared
reconciliation snapshot; standalone server-only pulls acquire equivalent
complete evidence themselves."
  (jaunder--with-debug-operation "pull.stage" ()
				 (unless (jaunder-inventory-member-p member)
				   (jaunder--pull-error "pull input must be a D1 inventory Member"))
				 (let* ((audience-capable
					 (jaunder--require-synchronization-audience-evidence
					  (jaunder--active-base-url) nil))
					(response (jaunder--http-request "GET" (jaunder-inventory-member-edit-uri member))))
				   (unless (and (integerp (plist-get response :status))
						(<= 200 (plist-get response :status) 299))
				     (jaunder--pull-error "Member GET returned non-2xx status"))
				   (let* ((entry-xml (plist-get response :body))
					  (identity (jaunder--pull-response-identity entry-xml))
					  (instance-id (jaunder--pull-member-instance-id response))
					  (etag (jaunder--response-header response "ETag")))
				     (unless (and (equal (car identity) (jaunder-inventory-member-id member))
						  (equal (cdr identity) (jaunder-inventory-member-slug member)))
				       (signal 'jaunder-pull-stage-identity-changed
					       (list (list :post-id (car identity) :slug (cdr identity)
							   :etag etag :http-status (plist-get response :status)
							   :detail "Member response identity changed since inventory"))))
				     (let* ((captured-at (current-time))
					    (pulled-member
					     (jaunder--parse-pulled-member entry-xml etag captured-at
									   (jaunder--current-zone-name)
									   audience-capable))
					    (source-body (jaunder-pulled-member-body pulled-member))
					    (inventory (and (equal (jaunder-pulled-member-format pulled-member) "org")
							    (let ((case-fold-search t))
							      (string-match-p "\\[\\[https?:" source-body))
							    (or jaunder--pull-link-inventory
								(jaunder--inventory-for-root root))))
					    (evidence (and inventory
							   (jaunder--inventory-post-link-evidence inventory)))
					    (members (car evidence))
					    (locals (cadr evidence))
					    (body (if inventory
						      (jaunder--reverse-pulled-post-links source-body root members locals)
						    source-body))
					    (plan (jaunder--pull-media-plan
						   (jaunder-pulled-member-format pulled-member) body
						   (jaunder--active-base-url))))
				       ;; The Post remains the final claim.  Matched consumers retain this
				       ;; stage so the original can be proved again at their install boundary.
				       (jaunder--reconcile-pull-progress "acquiring Local Media Copies")
				       (let* ((original-proof (or original-proof jaunder--pull-original-proof))
					      ;; Keep the established server-only materialization seam intact;
					      ;; only matched consumers need transient reuse evidence.
					      (media-staged (and original-proof
								 (jaunder--pull-media-stage
								  root instance-id plan original-proof))))
					 (unless media-staged
					   (jaunder--pull-media-materialize root instance-id plan))
					 (let ((localized-body
						(jaunder--pull-media-apply-plan
						 (if media-staged
						     (jaunder-pull-media-staged-plan media-staged)
						   plan))))
					   (list :etag etag :id (car identity) :slug (cdr identity)
						 :audience-omitted (jaunder-pulled-member-audience-omitted pulled-member)
						 :synced-at (format-time-string "%Y-%m-%dT%H:%M:%SZ" captured-at t)
						 :pulled-member pulled-member :original-proof original-proof
						 :media-staged media-staged
						 :bytes (jaunder--render-pulled-member pulled-member localized-body)))))))))

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
      (jaunder--call-with-blog
       root
       (lambda ()
         (let* ((staged (jaunder--pull-stage-member root member))
                (result (progn
                          (jaunder--reconcile-pull-progress "installing local Post")
                          (jaunder--install-pulled-bytes
                           path (plist-get staged :bytes) (plist-get staged :synced-at)))))
           (when (eq (jaunder-pull-result-status result) 'pulled)
             (setf (jaunder-pull-result-id result) (plist-get staged :id)
                   (jaunder-pull-result-slug result) (plist-get staged :slug)
                   (jaunder-pull-result-etag result) (plist-get staged :etag)
                   (jaunder-pull-result-http-status result) 200
                   (jaunder-pull-result-local-effect result) 'created))
           result))))))
(provide 'jaunder-pull)
;;; jaunder-pull.el ends here
