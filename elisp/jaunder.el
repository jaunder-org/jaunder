;;; jaunder.el --- Jaunder blogging client (AtomPub) -*- lexical-binding: t; -*-

;; Author: Jaunder contributors
;; Version: 0.1.0
;; Package-Requires: ((emacs "29.1"))
;; Keywords: hypermedia, comm, outlines
;; URL: https://jaunder.org

;;; Commentary:
;; Shared plumbing for the Jaunder Emacs blogging front-end over AtomPub.
;; This is the Infra-unit skeleton (issue #73): pure helpers plus seams that
;; units C (#74, authoring/publish) and D (#75, management/reconcile) extend.

;;; Code:

(require 'cl-lib)
(require 'org)
(require 'org-attach)
(require 'dom)
(require 'url-parse)
(require 'url-util)
(require 'auth-source)
(require 'seq)
(require 'plz)

(defgroup jaunder nil
  "Emacs blogging front-end for Jaunder over AtomPub."
  :group 'comm
  :prefix "jaunder-")

(defcustom jaunder-base-url nil
  "Base URL of the Jaunder instance, e.g. \"https://blog.example.com\"."
  :type '(choice (const :tag "Unset" nil) string)
  :group 'jaunder)

(defcustom jaunder-username nil
  "Username used for AtomPub authentication."
  :type '(choice (const :tag "Unset" nil) string)
  :group 'jaunder)

(defcustom jaunder-blogs nil
  "Alist mapping a local directory to a Jaunder blog.
Each element is (DIRECTORY . PLIST), where PLIST carries :base-url and
:username (strings) and an optional :format (accepted for forward
compatibility but not used in v1 — org is the only converter)."
  :type '(alist :key-type directory
                :value-type (plist :key-type symbol :value-type string))
  :group 'jaunder)

;;; Pure helpers

(defun jaunder--build-url (base &rest segments)
  "Join BASE and path SEGMENTS into a normalized URL.
Trailing slashes on BASE and surrounding slashes on each segment are
collapsed to single separators; nil or empty segments are dropped.
Signals an error when BASE is nil or empty."
  (when (or (null base) (string= base ""))
    (error "jaunder--build-url: BASE must be non-empty"))
  (let ((head (replace-regexp-in-string "/+\\'" "" base))
        (tail (delq nil
                    (mapcar (lambda (s)
                              (when (and s (not (string= s "")))
                                (let ((stripped (replace-regexp-in-string "\\`/+\\|/+\\'" "" s)))
                                  ;; An all-slash segment (e.g. "/") strips to ""; drop it
                                  ;; rather than relying on `delq' matching interned "".
                                  (unless (string= stripped "") stripped))))
                            segments))))
    (mapconcat #'identity (cons head tail) "/")))

(defun jaunder--basic-auth-header (user password)
  "Return the HTTP Basic Authorization header cons for USER and PASSWORD.
The value is \"Basic <base64(user:password)>\" with no line breaks.
Credentials are UTF-8-encoded before base64 (RFC 7617) so non-ASCII
usernames/passwords are handled rather than raising."
  (cons "Authorization"
        (concat "Basic "
                (base64-encode-string
                 (encode-coding-string (concat user ":" password) 'utf-8) t))))

(defun jaunder--auth-source-spec (base-url user)
  "Return the `auth-source-search' plist for BASE-URL and USER.
:host is the URL host of BASE-URL (port excluded); at most one match."
  (list :host (url-host (url-generic-parse-url base-url))
        :user user
        :max 1))

;;; Seams — implemented by later units; calling them now is a programmer error.

(defun jaunder--plz-response->plist (response)
  "Convert a `plz-response' RESPONSE to a (:status :headers :body) plist.
Header names are downcased strings so `jaunder--response-header' can look
them up case-insensitively."
  (list :status (plz-response-status response)
        :headers (mapcar (lambda (h)
                           (cons (downcase (format "%s" (car h))) (cdr h)))
                         (plz-response-headers response))
        :body (or (plz-response-body response) "")))

(defun jaunder--response-header (response name)
  "Return the value of header NAME (case-insensitive) in RESPONSE, or nil."
  (cdr (assoc (downcase name) (plist-get response :headers))))

(defun jaunder--auth-secret ()
  "Retrieve the app password for `jaunder-username' via auth-source.
Thin I/O wrapper over `auth-source-search' using `jaunder--auth-source-spec'."
  (let* ((match (car (apply #'auth-source-search
                            (jaunder--auth-source-spec jaunder-base-url
                                                       jaunder-username))))
         (secret (and match (plist-get match :secret))))
    (cond ((functionp secret) (funcall secret))
          (secret secret)
          (t (error "jaunder: no auth-source entry for %s@%s"
                    jaunder-username jaunder-base-url)))))

(defun jaunder--http-request (method url &optional body content-type extra-headers)
  "Make an authenticated METHOD request to URL via `plz', returning a plist.
METHOD is an HTTP verb string; URL an absolute URL.  BODY is a request body: a
string, or the `plz' file form `(file PATH)' to upload a file's raw bytes.
CONTENT-TYPE and EXTRA-HEADERS (an alist of extra (NAME . VALUE) headers) apply to
write requests.  Basic-auth credentials come from `jaunder--auth-secret' for
`jaunder-username'.  Returns the `jaunder--plz-response->plist' plist; HTTP error
statuses (4xx/5xx) are reported in :status, not signalled.  A transport-level
failure re-signals.

`plz' drives the `curl' binary, so request construction does not depend on
the finicky dynamic-variable handling that made `url.el' occasionally drop
the auth header under load (ADR-0038)."
  (let ((headers (append
                  (list (jaunder--basic-auth-header jaunder-username
                                                    (jaunder--auth-secret)))
                  (when content-type (list (cons "Content-Type" content-type)))
                  extra-headers))
        (verb (intern (downcase method))))
    (condition-case err
        (jaunder--plz-response->plist
         (plz verb url :headers headers :body body :as 'response))
      (plz-error
       (let* ((pe (seq-find #'plz-error-p (cdr err)))
              (resp (and pe (plz-error-response pe))))
         (if resp
             (jaunder--plz-response->plist resp)
           (signal (car err) (cdr err))))))))

;;; org -> atom mapping (unit C, issue #74/#160)

(cl-defstruct (jaunder-entry (:constructor jaunder--make-entry))
              "Structured AtomPub entry mapped from an org buffer (issue #160).
Holds abstract field values only; wire encoding (namespaces, media types,
`app:draft' nesting) lives in `jaunder--atom-entry->xml'.  `body' is the
body-only content with the metadata header block stripped."
              title categories summary draft content-type body published)

(defconst jaunder--header-keyword-re
  "^[ \t]*#\\+[A-Za-z][A-Za-z0-9_-]*:"
  "Regexp matching any org file-keyword line (`#+KEY:').
The metadata header block is the leading run of these; matching *any*
keyword (not just the mapped ones) means an interleaved keyword such as
`#+AUTHOR:' cannot halt stripping and leak a later `#+PROPERTY: JAUNDER_*'
into the sent body.  The trailing colon excludes block markers like
`#+begin_src' (issue #160).")

(defconst jaunder--blank-line-re "^[ \t]*$"
  "Regexp matching a blank (whitespace-only) line.")

(defconst jaunder--org-media-type "text/org"
  "The atom:content media type for org source.
`jaunder--org->atom' converts an org buffer, so its content is always org;
the media type is knowable from the converter, not from any header field.
Non-org authoring buffers (markdown/html) are separate future converters,
out of scope for v1 (issue #160).")

(defun jaunder--collect-properties (keywords)
  "Return an alist of file-level #+PROPERTY: KEY/VALUE pairs from KEYWORDS.
KEYWORDS is the result of `org-collect-keywords'; each PROPERTY entry is a
\"KEY VALUE\" string split on the first run of whitespace."
  (delq nil
        (mapcar (lambda (line)
                  (when (string-match "\\`\\([^ \t]+\\)[ \t]+\\(.*\\)\\'" line)
                    (cons (match-string 1 line) (match-string 2 line))))
                (cdr (assoc "PROPERTY" keywords)))))

(defun jaunder--split-keywords (values)
  "Split each #+KEYWORDS: string in VALUES on commas and flatten.
Whitespace is trimmed and empty terms dropped."
  (let (out)
    (dolist (line values (nreverse out))
      (dolist (term (split-string line "," t "[ \t]+"))
        (unless (string= term "") (push term out))))))

(defun jaunder--body-start ()
  "Return the position after the leading metadata header block in this buffer.
The header block is the leading contiguous run of header-keyword and blank lines.
Shared by `jaunder--strip-header-block' and media detection so both see the same
body region (#161)."
  (save-excursion
    (goto-char (point-min))
    (let ((case-fold-search t))
      (while (and (not (eobp))
                  (or (looking-at-p jaunder--blank-line-re)
                      (looking-at-p jaunder--header-keyword-re)))
        (forward-line 1)))
    (point)))

(defun jaunder--strip-header-block (text)
  "Return TEXT with its leading metadata header block removed.
Drops the leading contiguous run of header keyword lines and blank lines
(`jaunder--body-start'), then trims surrounding whitespace from the remaining body."
  (with-temp-buffer
    (insert text)
    (string-trim (buffer-substring-no-properties (jaunder--body-start) (point-max)))))

(defun jaunder--offset->seconds (offset)
  "Parse a numeric UTC OFFSET string (\"±HHMM\" or \"±HH:MM\") to integer seconds.
Returns nil when OFFSET is not a numeric offset.  Used only for the
JAUNDER_DATE_TZ fallback: `encode-time' silently misreads an offset *string*
as UTC, so a numeric offset must be handed to it as integer seconds (#160)."
  (when (and offset
             (string-match
              "\\`\\([+-]\\)\\([0-9]\\{2\\}\\):?\\([0-9]\\{2\\}\\)\\'" offset))
    (let ((sign (if (string= (match-string 1 offset) "-") -1 1))
          (hours (string-to-number (match-string 2 offset)))
          (mins (string-to-number (match-string 3 offset))))
      (* sign (+ (* hours 3600) (* mins 60))))))

(defun jaunder--resolve-zone (tz)
  "Resolve a JAUNDER_DATE_TZ string TZ to an `encode-time' ZONE value.
An IANA name is preferred and returned as-is (DST-correct); a numeric offset
is parsed to integer seconds (the fallback — see `jaunder--offset->seconds').
nil or empty TZ falls back to the local zone.  A typo'd IANA name is silently
treated as UTC by `encode-time'; time zones are hard and we do our best (#160)."
  (cond
   ((or (null tz) (string= (string-trim tz) "")) nil)
   ((jaunder--offset->seconds tz))
   (t tz)))

(defun jaunder--org-date->utc (date-raw tz)
  "Interpret org timestamp DATE-RAW in zone TZ; return RFC-3339 UTC, or nil.
DATE-RAW is a raw #+DATE value (e.g. \"[2026-07-01 Wed 09:00]\"); TZ is a
JAUNDER_DATE_TZ string (IANA name preferred, numeric offset as fallback).
Returns nil when DATE-RAW does not parse to a time."
  (let ((decoded (ignore-errors (org-parse-time-string date-raw))))
    (when decoded
      (setf (nth 8 decoded) (jaunder--resolve-zone tz))
      (format-time-string "%Y-%m-%dT%H:%M:%SZ" (encode-time decoded) t))))

(defun jaunder--utc->org-date (utc tz)
  "Render an org inactive timestamp for UTC interpreted in zone TZ (C4 / #162).
UTC is an RFC-3339 UTC string (e.g. \"2026-07-01T13:00:00Z\"); TZ a
JAUNDER_DATE_TZ string.  Inverse of `jaunder--org-date->utc' at org's minute
resolution: a server UTC carrying non-zero seconds is truncated to the minute
\(org timestamps have no seconds field)."
  (format-time-string "[%Y-%m-%d %a %H:%M]"
                      (date-to-time utc)
                      (jaunder--resolve-zone tz)))

(defun jaunder--current-zone-name ()
  "Return the machine's current IANA zone name, else a numeric offset string (C4 / #162).
Prefers a `TZ' IANA name, then the /etc/localtime symlink target; falls back to
the current numeric UTC offset (IANA preferred, offset caveat).  The TZ branch
trusts a non-empty, non-`:'-prefixed value as an IANA name; a POSIX-style TZ
\(e.g. \"EST5EDT\") is passed through as-is."
  (or (let ((tz (getenv "TZ")))
        (and tz (not (string= tz "")) (not (string-prefix-p ":" tz)) tz))
      (let ((link (ignore-errors (file-symlink-p "/etc/localtime"))))
        (and link (string-match "zoneinfo/\\(.+\\)\\'" link)
             (match-string 1 link)))
      (format-time-string "%z")))

(defun jaunder--ensure-date-tz ()
  "Ensure the buffer records a JAUNDER_DATE_TZ; return the effective zone string (C4 / #162).
When unset, captures the machine's current zone (`jaunder--current-zone-name')
so #+DATE: is interpreted in a recorded zone, not one silently re-inferred on a
later machine.  Idempotent: an existing value is preserved verbatim."
  (or (jaunder--buffer-property "JAUNDER_DATE_TZ")
      (let ((zone (jaunder--current-zone-name)))
        (jaunder--set-property "JAUNDER_DATE_TZ" zone)
        zone)))

(defun jaunder--org->atom ()
  "Map the current org buffer to a `jaunder-entry' (issue #160).
Reads the metadata header block via `org-collect-keywords' and carries the
body-only content with the header block stripped.  Non-mutating.  The
`published' slot is filled by the timezone computation (see
`jaunder--org-date->utc'); `body' still holds local media links, substituted
later by the media unit (#161)."
  (let* ((kws (org-collect-keywords
               '("TITLE" "DATE" "KEYWORDS" "DESCRIPTION" "PROPERTY")))
         (props (jaunder--collect-properties kws))
         (raw-title (cadr (assoc "TITLE" kws)))
         (title (and raw-title (not (string= (string-trim raw-title) "")) raw-title))
         (categories (jaunder--split-keywords (cdr (assoc "KEYWORDS" kws))))
         (descriptions (cdr (assoc "DESCRIPTION" kws)))
         (summary (and descriptions (mapconcat #'identity descriptions "\n")))
         (status (cdr (assoc "JAUNDER_STATUS" props)))
         (draft (and status (string= (downcase status) "draft") t))
         (date-raw (cadr (assoc "DATE" kws)))
         (tz (cdr (assoc "JAUNDER_DATE_TZ" props)))
         ;; Drafts carry no publish time; "publish now" (published status, no
         ;; #+DATE) omits it so the server stamps it (see the spec status table).
         (published (and (not draft) date-raw
                         (jaunder--org-date->utc date-raw tz))))
    (jaunder--make-entry
     :title title
     :categories categories
     :summary summary
     :draft draft
     :content-type jaunder--org-media-type
     :body (jaunder--strip-header-block (buffer-string))
     :published published)))

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
here (issue #160)."
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

;;; media upload + content-addressed link substitution (unit C, issue #161)

(defun jaunder--atom-entry-fields (xml)
  "Parse AtomPub entry XML into an alist of harvested fields.
Returns `content-src'/`content-type' from `<content>', `slug' from `<j:slug>',
and `published' from `<published>'.  The shared entry-parse primitive: C3 uses
the content subset, C4 the slug/published subset, Unit D extends it further.
`libxml-parse-xml-region' folds the default namespace, so `<content>' and
`<published>' are `content'/`published'; the `j:'-prefixed slug is matched by
local name via `dom-by-tag' on the `slug' symbol."
  (let* ((dom (with-temp-buffer
                (insert xml)
                (libxml-parse-xml-region (point-min) (point-max))))
         (content (car (dom-by-tag dom 'content)))
         (slug (car (dom-by-tag dom 'slug)))
         (published (car (dom-by-tag dom 'published))))
    (list (cons 'content-src (dom-attr content 'src))
          (cons 'content-type (dom-attr content 'type))
          (cons 'slug (and slug (dom-text slug)))
          (cons 'published (and published (dom-text published))))))

(defconst jaunder--media-image-types
  '(("png" . "image/png")
    ("jpg" . "image/jpeg")
    ("jpeg" . "image/jpeg")
    ("gif" . "image/gif")
    ("webp" . "image/webp")
    ("svg" . "image/svg+xml"))
  "Alist of lowercase image extension → MIME type for uploadable media (#161).
Its key set is the qualification predicate: only links whose file extension is a
key are uploaded.  Non-image media is #25.")

(defun jaunder--media-content-type (filename)
  "Return the media MIME type for FILENAME by extension, or nil if unqualified.
The extension match is case-insensitive."
  (let ((ext (downcase (or (file-name-extension filename) ""))))
    (cdr (assoc ext jaunder--media-image-types))))

(defun jaunder--media-link-p (link)
  "Return the media MIME type if org-element LINK is a qualifying local-image link.
Qualifies a `file:'- or `attachment:'-type link whose target has an image
extension; nil otherwise.  The single source of truth for \"qualifies\", shared by
detection and substitution so the two stay in lockstep — their positional
one-for-one alignment rides on agreeing here (#161)."
  (and (member (org-element-property :type link) '("file" "attachment"))
       (jaunder--media-content-type (org-element-property :path link))))

(defun jaunder--upload-media (path content-type)
  "Upload the file at PATH as CONTENT-TYPE to the media collection; return its URL.
POSTs the raw bytes to `/atompub/{user}/media' with the filename in a `Slug'
header (the server sha256-dedups: 201 new / 200 re-upload), then harvests the
server-assigned binary URL from the response entry's `<content src>' via
`jaunder--atom-entry-fields'.  Signals an error on any non-2xx status (#161)."
  (let* ((url (jaunder--build-url jaunder-base-url "atompub" jaunder-username "media"))
         (resp (jaunder--http-request
                "POST" url (list 'file path) content-type
                (list (cons "Slug" (file-name-nondirectory path)))))
         (status (plist-get resp :status)))
    (unless (memq status '(200 201))
      (error "jaunder: media upload of %s failed (HTTP %s)" path status))
    (cdr (assq 'content-src
               (jaunder--atom-entry-fields (plist-get resp :body))))))

(defun jaunder--collect-media-links ()
  "Collect qualifying local-image links in the current buffer's body region, in order.
Walks `org-element' `link' objects after the header block (`jaunder--body-start'),
keeping `file:'-type links whose extension is an image and `attachment:' links
(both via `jaunder--media-content-type').  Returns an ordered list of plists
\(:raw-link RAW :path ABS :content-type MIME).  `file:' paths resolve against
`default-directory'; `attachment:' paths via `org-attach-expand' at the link's
heading.  Restricting to the body region keeps this list aligned one-for-one and
in order with the links in the C2 sent body (#161)."
  (save-restriction
    (narrow-to-region (jaunder--body-start) (point-max))
    (let ((tree (org-element-parse-buffer)))
      (delq nil
            (org-element-map tree 'link
                             (lambda (link)
                               (let ((mime (jaunder--media-link-p link)))
                                 (when mime
                                   (let ((raw (org-element-property :raw-link link))
                                         (path (org-element-property :path link)))
                                     (list :raw-link raw
                                           :content-type mime
                                           :path (if (string= (org-element-property :type link)
                                                              "attachment")
                                                     (save-excursion
                                                       (goto-char (org-element-property :begin link))
                                                       (org-attach-expand path))
                                                   (expand-file-name path))))))))))))

(defun jaunder--substitute-media (body urls)
  "Return BODY with its qualifying media links rewritten to URLS, in order.
URLS has one entry per qualifying link in document order.  Each link's whole inner
target is replaced with its URL, brackets and any `[…][description]' preserved
\(result stays `[[URL]]' / `[[URL][desc]]').  Rewrites right-to-left (#161)."
  (with-temp-buffer
    (insert body)
    (org-mode)
    (let* ((tree (org-element-parse-buffer))
           (links (delq nil
                        (org-element-map tree 'link
                                         (lambda (link)
                                           (when (jaunder--media-link-p link) link)))))
           (pairs (cl-mapcar #'cons links urls)))
      (dolist (pair (nreverse pairs))
        (let* ((link (car pair))
               (url (cdr pair))
               (beg (org-element-property :begin link))
               (end (- (org-element-property :end link)
                       (or (org-element-property :post-blank link) 0)))
               (cb (org-element-property :contents-begin link))
               (ce (org-element-property :contents-end link))
               (desc (and cb ce (buffer-substring-no-properties cb ce))))
          (delete-region beg end)
          (goto-char beg)
          (insert (if desc (format "[[%s][%s]]" url desc) (format "[[%s]]" url))))))
    (buffer-substring-no-properties (point-min) (point-max))))

(defun jaunder--media-preflight (records)
  "Signal an error if any RECORDS `:path' is not a readable file.
RECORDS is a `jaunder--collect-media-links' list.  Checks every path and, if any
are missing, signals one error listing them all — fail-fast, upload nothing (#161)."
  (let ((missing (delq nil
                       (mapcar (lambda (r)
                                 (let ((p (plist-get r :path)))
                                   (unless (file-readable-p p) p)))
                               records))))
    (when missing
      (error "jaunder: media file(s) not found: %s"
             (mapconcat #'identity missing ", ")))))

(defun jaunder--localize-media (body)
  "Upload the current buffer's local images and return BODY with links localized.
Detects qualifying media links in the buffer's body region, pre-flights that all
exist (else errors, uploading nothing), uploads each distinct file once (server
sha256-dedups), and rewrites those links in BODY — the C2 sent body — to the
harvested server URLs, in order.  The authoring buffer is never modified (#161)."
  (let ((records (jaunder--collect-media-links)))
    (jaunder--media-preflight records)
    (let ((cache (make-hash-table :test 'equal)))
      (dolist (r records)
        (let ((path (plist-get r :path)))
          (unless (gethash path cache)
            (puthash path
                     (jaunder--upload-media path (plist-get r :content-type))
                     cache))))
      (jaunder--substitute-media
       body
       (mapcar (lambda (r) (gethash (plist-get r :path) cache)) records)))))

;;; buffer read/write helpers (unit C4, issue #162)

(defun jaunder--set-keyword-line (line-re new-line)
  "Replace the first LINE-RE match in the leading header block with NEW-LINE.
When absent, insert NEW-LINE after the last contiguous header-keyword line
\(before any blank line or the body).  Header block only; the body is never
touched."
  (save-excursion
    (goto-char (point-min))
    (let ((case-fold-search t)
          (limit (jaunder--body-start)))
      (if (re-search-forward line-re limit t)
          (progn (beginning-of-line)
                 (delete-region (point) (line-end-position))
                 (insert new-line))
        (goto-char (point-min))
        (let ((insert-at (point-min)))
          (while (looking-at-p jaunder--header-keyword-re)
            (forward-line 1)
            (setq insert-at (point)))
          (goto-char insert-at)
          (insert new-line "\n"))))))

(defun jaunder--set-property (key value)
  "Set the file-level #+PROPERTY: KEY to VALUE (idempotent replace or insert)."
  (jaunder--set-keyword-line
   (format "^[ \t]*#\\+PROPERTY:[ \t]+%s\\(?:[ \t].*\\)?$" (regexp-quote key))
   (format "#+PROPERTY: %s %s" key value)))

(defun jaunder--set-keyword (keyword value)
  "Set the file-level #+KEYWORD: to VALUE (idempotent replace or insert)."
  (jaunder--set-keyword-line
   (format "^[ \t]*#\\+%s:.*$" (regexp-quote keyword))
   (format "#+%s: %s" keyword value)))

(defun jaunder--buffer-property (key)
  "Return the #+PROPERTY: KEY value in the current buffer, or nil."
  (cdr (assoc key (jaunder--collect-properties
                   (org-collect-keywords '("PROPERTY"))))))

(defun jaunder--buffer-keyword (key)
  "Return the #+KEY: value in the current buffer, or nil."
  (cadr (assoc key (org-collect-keywords (list key)))))

;;; multi-blog config + resolution (unit C4, issue #162)

(defun jaunder--resolve-blog (file-or-dir)
  "Return the active-blog plist (:base-url :username) for FILE-OR-DIR.
Longest-prefix match against `jaunder-blogs'; else the single-blog globals;
else an error naming the directory."
  (let* ((dir (file-name-as-directory
               (expand-file-name (if (file-directory-p file-or-dir)
                                     file-or-dir
                                   (file-name-directory file-or-dir)))))
         (best nil) (best-len -1))
    (dolist (entry jaunder-blogs)
      (let ((root (file-name-as-directory (expand-file-name (car entry)))))
        (when (and (string-prefix-p root dir) (> (length root) best-len))
          (setq best (cdr entry) best-len (length root)))))
    (cond
     (best (list :base-url (plist-get best :base-url)
                 :username (plist-get best :username)))
     ((and jaunder-base-url jaunder-username)
      (list :base-url jaunder-base-url :username jaunder-username))
     (t (error "jaunder: no blog configured for %s" dir)))))

(defmacro jaunder--with-blog (file &rest body)
  "Resolve the blog for FILE and run BODY with the transport specials bound."
  (declare (indent 1) (debug t))
  (let ((blog (make-symbol "blog")))
    `(let* ((,blog (jaunder--resolve-blog ,file))
            (jaunder-base-url (plist-get ,blog :base-url))
            (jaunder-username (plist-get ,blog :username)))
       ,@body)))

;;; publish validation + Location->id + force-draft (unit C4, issue #162)

(defun jaunder--validate-publish (entry status date-raw tz)
  "Signal an error if ENTRY is not publishable; return nil otherwise.
Requires a non-empty body; a `scheduled' STATUS requires a future #+DATE:
\(DATE-RAW interpreted in TZ)."
  (when (string= (string-trim (or (jaunder-entry-body entry) "")) "")
    (error "jaunder: refusing to publish an empty body"))
  (when (and status (string= (downcase status) "scheduled"))
    (let ((utc (and date-raw (jaunder--org-date->utc date-raw tz))))
      (unless (and utc (time-less-p (current-time) (date-to-time utc)))
        (error "jaunder: a scheduled post needs a future #+DATE:"))))
  nil)

(defun jaunder--location->id (location)
  "Return the trailing numeric post id from a create `Location' URL, or nil."
  (when (and location (string-match "/\\([0-9]+\\)/?\\'" location))
    (match-string 1 location)))

(defun jaunder--force-draft (entry)
  "Mark ENTRY a server-side draft in place: set `draft', clear `published'.
Clearing `published' keeps `jaunder--atom-entry->xml' from emitting a
`<published>' on a draft (it emits one whenever the slot is set)."
  (setf (jaunder-entry-draft entry) t
        (jaunder-entry-published entry) nil)
  entry)

(defun jaunder--atom->org (&rest _args)
  "Atom->Org mapping seam.  Implemented by units C/D (issues #74/#75)."
  (error "jaunder: atom->org mapping not yet implemented (units C/D, issues #74/#75)"))

(provide 'jaunder)
;;; jaunder.el ends here
