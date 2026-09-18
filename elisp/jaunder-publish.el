;;; jaunder-publish.el --- Jaunder publish/new-post commands -*- lexical-binding: t; -*-

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
;; The user-facing commands and their orchestration: `jaunder-new-post',
;; `jaunder-publish', and `jaunder-save-draft', plus the transient new-Post
;; input lifecycle (`C-c C-c' completes and `C-c C-k' abandons) and the ID-first
;; safe-to-resume write-back that ties the buffer, the mapper, the wire, media,
;; and transport together (ADR-0047).

;;; Code:

;; `plz' is required directly, not left to arrive via `jaunder-transport': the
;; retry below dispatches on the `plz-error' condition, and `condition-case'
;; silently never matches a condition symbol that no `define-error' has run for.
(require 'plz)

(require 'jaunder-entry)
(require 'jaunder-config)
(require 'jaunder-datetime)
(require 'jaunder-atom)
(require 'jaunder-org)
(require 'jaunder-transport)
(require 'jaunder-service)
(require 'jaunder-media)

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

(defun jaunder--rename-to-slug (slug)
  "Rename the current buffer's file and buffer to SLUG.org in its directory.
A no-op when already so named; on collision appends `-N'.  Returns the path."
  (let* ((old (or (buffer-file-name)
                  (error "jaunder: buffer is not visiting a file")))
         (dir (file-name-directory old))
         (target (expand-file-name (concat slug ".org") dir)))
    (if (string= old target)
        old
      (let ((final target) (n 1))
        (while (file-exists-p final)
          (setq final (expand-file-name (format "%s-%d.org" slug n) dir)
                n (1+ n)))
        (rename-file old final)
        ;; ALONG-WITH-FILE=t: the file is already moved, so don't re-save it;
        ;; NO-QUERY=t: never prompt (publish is automated).
        (set-visited-file-name final t t)
        final))))

(defun jaunder--write-back (response created &optional create-intent-matches conditional-update)
  "Persist server-assigned values from RESPONSE into the current buffer.
RESPONSE is a `jaunder--http-request' plist.  CREATED non-nil (a POST) writes
JAUNDER_ID from the `Location' header; an update leaves it unchanged.  A create
whose current Entry differs from its persisted intent recovers identity but does
not claim synchronization.  A changed recovery records an explicit local-ahead
marker, which a later successful conditional PUT clears.  Writes JAUNDER_ID
first, then JAUNDER_SLUG, JAUNDER_SYNCED (ETag, verbatim), JAUNDER_SYNCED_AT
(the original attempt time for changed recovery, otherwise now), and the
resolved publish time.  Saves the buffer and returns the slug.

Precondition for the publish-now `#+DATE:' render: the buffer's JAUNDER_DATE_TZ
must already be recorded (the command calls `jaunder--ensure-date-tz' before the
send); absent it, the render falls back to the local zone via
`jaunder--resolve-zone'."
  (let* ((fields (jaunder--harvest-response-fields (plist-get response :body)))
         (slug (cdr (assq 'slug fields)))
         (published (cdr (assq 'published fields)))
         (etag (jaunder--response-header response "ETag"))
         (now (format-time-string "%Y-%m-%dT%H:%M:%SZ" nil t))
         (synced-at (if (eq create-intent-matches 'changed)
                        (or (jaunder--buffer-property "JAUNDER_CREATE_ATTEMPT_AT") now)
                      now)))
    (when created
      (let ((id (jaunder--location->id
                 (jaunder--response-header response "Location"))))
        (when id
          (jaunder--set-property "JAUNDER_ID" id)
          ;; The ID must reach disk before clearing the recovery intent.
          (save-buffer))))
    (when slug (jaunder--set-property "JAUNDER_SLUG" slug))
    ;; A replay's ETag is the remote baseline even when the local Entry changed
    ;; after its recorded create attempt.  The original attempt time leaves that
    ;; changed file visibly local-ahead rather than marker-unclassifiable.
    (when etag (jaunder--set-property "JAUNDER_SYNCED" etag))
    (jaunder--set-property "JAUNDER_SYNCED_AT" synced-at)
    (when (eq create-intent-matches 'changed)
      (jaunder--set-property "JAUNDER_LOCAL_AHEAD" "true"))
    (when conditional-update
      (jaunder--remove-property "JAUNDER_LOCAL_AHEAD"))
    (when published
      ;; published→UTC (drop the offset): the canonical value the server stamped.
      (let ((utc (format-time-string "%Y-%m-%dT%H:%M:%SZ"
                                     (date-to-time published) t))
            (tz (jaunder--buffer-property "JAUNDER_DATE_TZ")))
        (jaunder--set-property "JAUNDER_DATE_UTC" utc)
        ;; "publish now": no author #+DATE: — render it from the server time.
        (unless (jaunder--buffer-keyword "DATE")
          (jaunder--set-keyword "DATE" (jaunder--utc->org-date utc tz)))))
    (save-buffer)
    (when (jaunder--buffer-property "JAUNDER_ID")
      (jaunder--remove-property "JAUNDER_CREATE_KEY")
      (jaunder--remove-property "JAUNDER_CREATE_DIGEST")
      (jaunder--remove-property "JAUNDER_CREATE_ATTEMPT_AT")
      (save-buffer))
    slug))

(defun jaunder--new-post-in (dir now-string)
  "Create and save a timestamped draft in DIR stamped NOW-STRING; return its path.
Inserts the minimal org template (empty TITLE, DATE now, empty KEYWORDS and
DESCRIPTION, JAUNDER_STATUS draft, and the current JAUNDER_DATE_TZ) and leaves
point in the body."
  (let* ((path (expand-file-name (format "draft-%s.org" now-string) dir))
         (buf (find-file-noselect path)))
    (with-current-buffer buf
      (insert "#+TITLE: \n"
              (format "#+DATE: %s\n" (format-time-string "[%Y-%m-%d %a %H:%M]"))
              "#+KEYWORDS: \n"
              "#+DESCRIPTION: \n"
              "#+PROPERTY: JAUNDER_STATUS draft\n\n")
      ;; Capture the interpretation zone before editing so a failed first
      ;; publish has no reason to mutate the author's input.
      (jaunder--ensure-date-tz)
      (save-buffer))
    path))

(defun jaunder--select-new-post-blog ()
  "Return the configured blog entry selected for ordinary Post creation.
Uses the longest matching root, prompts among configured blogs when none
matches, and returns (`default-directory' . nil) when no blogs are configured."
  (let ((entry (jaunder--blog-entry-for default-directory)))
    (cond
     (entry entry)
     (jaunder-blogs
      (let ((selected
             (completing-read
              "Blog directory: " (mapcar #'car jaunder-blogs) nil t)))
        (or (assoc selected jaunder-blogs)
            (error "jaunder: selected blog is no longer configured"))))
     (t (cons default-directory nil)))))

(defun jaunder--select-minimal-new-post-blog ()
  "Return the unambiguous blog entry for prompt-free Post creation.
Uses the longest matching configured root.  An unconfigured client uses
`default-directory'; a nonempty configuration with no matching root is an
error rather than a hidden blog-choice prompt."
  (or (jaunder--blog-entry-for default-directory)
      (if jaunder-blogs
          (user-error
           "jaunder: no configured blog contains %s" default-directory)
        (cons default-directory nil))))

(defun jaunder--new-post-tag-candidates (entry)
  "Return the Posts Collection Tag candidates for configured blog ENTRY.
Failures emit one message and return nil so local Post creation remains
available without server completion."
  (if (null (cdr entry))
      (progn
        (message "jaunder: Tag completion unavailable; no blog is configured")
        nil)
    (condition-case err
        (let ((tags
               (jaunder--call-with-blog
                (car entry)
                (lambda ()
                  (jaunder--fetch-service-tags
                   (jaunder--active-base-url))))))
          (if (eq tags 'unknown)
              (progn
                (message
                 "jaunder: Tag completion unavailable; using free-text entry")
                nil)
            tags))
      (error
       (message "jaunder: Tag completion unavailable: %s"
                (error-message-string err))
       nil))))


(defun jaunder--invalid-tag-repair (answer defect)
  "Return (PROMPT . CURSOR) for invalid Tag ANSWER and its DEFECT.
DEFECT comes from `jaunder--tag-label-defect', the shared grammar authority.
PROMPT explains that first violation.  CURSOR is its zero-based position in the
original, untrimmed ANSWER so the next prompt can preserve and repair exactly
what the user entered."
  (let* ((trimmed (string-trim answer))
         (leading-whitespace
          (- (length answer) (length (string-trim-left answer))))
         (offset (cdr defect)))
    (pcase (car defect)
      ('invalid-start
       (cons
        (concat
         "Tag must start with an ASCII letter or digit; remaining characters "
         "may be ASCII letters, digits, or hyphens; edit: ")
        leading-whitespace))
      ('invalid-character
       (cons
        (format
         (concat
          "Tag character %S at position %d is not allowed; subsequent "
          "characters allow only ASCII letters, digits, or hyphens; edit: ")
         (char-to-string (aref trimmed offset))
         (1+ offset))
        (+ leading-whitespace offset)))
      (_ (error "jaunder: unsupported Tag defect %S" (car defect))))))

(defun jaunder--read-new-post-tags (candidates)
  "Prompt for Tags using CANDIDATES until empty input; return accepted labels.
New valid labels are allowed.  Invalid labels re-prompt, and duplicate
canonical slugs are omitted while preserving first-entry order."
  (let (labels seen done retry-input retry-prompt)
    (while (not done)
      (let* ((answer
              (completing-read
               (or retry-prompt "Tag (empty to finish): ")
               candidates nil nil retry-input))
             (label (string-trim answer))
             (slug (downcase label))
             (defect (jaunder--tag-label-defect answer)))
        (setq retry-input nil retry-prompt nil)
        (cond
         ((string-empty-p label) (setq done t))
         (defect
          (let ((repair (jaunder--invalid-tag-repair answer defect)))
            (setq retry-input (cons answer (cdr repair))
                  retry-prompt (car repair)))
          (message
           "jaunder: Tag must match [a-z0-9][a-z0-9-]* (case preserved)"))
         ((member slug seen))
         (t
          (push slug seen)
          (push label labels)))))
    (nreverse labels)))

(defun jaunder--read-new-post-schedule ()
  "Prompt until the Org date reader returns a future instant.
Invalid or non-future answers re-prompt.  `quit' remains uncaught so cancelling
the command before file creation has no filesystem side effect."
  (let (scheduled-date)
    (while (null scheduled-date)
      (condition-case err
          (let* ((candidate
                  (org-read-date nil t nil "Scheduled date: "))
                 (rendered
                  (format-time-string "[%Y-%m-%d %a %H:%M]" candidate))
                 (persisted (org-time-string-to-time rendered)))
            (if (time-less-p (current-time) persisted)
                (setq scheduled-date rendered)
              (message "jaunder: scheduled date must be in the future")))
        (error
         (message "jaunder: invalid scheduled date: %s"
                  (error-message-string err)))))
    scheduled-date))

(defun jaunder--write-new-post-metadata (path title tags status scheduled-date)
  "Write TITLE, TAGS, STATUS, and SCHEDULED-DATE into the Post at PATH."
  (with-current-buffer (find-file-noselect path)
    (jaunder--set-keyword "TITLE" title)
    (jaunder--set-keyword "KEYWORDS" (mapconcat #'identity tags ", "))
    (jaunder--set-property "JAUNDER_STATUS" status)
    (when scheduled-date
      (jaunder--set-keyword "DATE" scheduled-date))
    (save-buffer)))

(defvar-keymap jaunder-new-post-mode-map ;; cov:ignore: defvar-keymap expands to synthetic bookkeeping with no instrumentable source form
  :doc "Keymap for a Post being entered by `jaunder-new-post'."
  "C-c C-c" #'jaunder-new-post-complete
  "C-c C-k" #'jaunder-new-post-cancel)

(define-minor-mode jaunder-new-post-mode ;; cov:ignore: define-minor-mode expands to synthetic bookkeeping with no instrumentable source form
  "Treat the current buffer as transient new-Post input."
  :lighter nil
  :keymap jaunder-new-post-mode-map)

(defun jaunder-new-post-complete ()
  "Publish the new Post and close its input buffer on success."
  (interactive)
  (jaunder-publish)
  (kill-current-buffer))

(defun jaunder-new-post-cancel ()
  "Delete the local new Post and close its input buffer."
  (interactive)
  (let ((path (or (buffer-file-name)
                  (error "jaunder: new Post buffer is not visiting a file"))))
    (when (file-exists-p path)
      (delete-file path))
    (set-buffer-modified-p nil)
    (kill-current-buffer)))

(defun jaunder-new-post (&optional prefix)
  "Create an Org Post and visit its body.
Ordinary invocation resolves the target blog and collects title, Tags, and
status before creating the file.  With PREFIX, preserve minimal-template
creation without prompts; an unmatched nonempty `jaunder-blogs' is then an
error rather than an implicit target choice."
  (interactive "P")
  (if prefix
      (let* ((entry (jaunder--select-minimal-new-post-blog))
             (path
              (jaunder--new-post-in
               (car entry) (format-time-string "%Y%m%dT%H%M%S"))))
        (switch-to-buffer (find-file-noselect path))
        (jaunder-new-post-mode 1)
        (goto-char (point-max)))
    (let* ((entry (jaunder--select-new-post-blog))
           (dir (car entry))
           (title (read-string "Title: "))
           (tags
            (jaunder--read-new-post-tags
             (jaunder--new-post-tag-candidates entry)))
           (status
            (completing-read
             "Status: " '("draft" "published" "scheduled") nil t nil nil "draft"))
           (scheduled-date
            (when (equal status "scheduled")
              (jaunder--read-new-post-schedule)))
           (path
            (jaunder--new-post-in dir (format-time-string "%Y%m%dT%H%M%S"))))
      (jaunder--write-new-post-metadata
       path title tags status scheduled-date)
      (switch-to-buffer (find-file-noselect path))
      (jaunder-new-post-mode 1)
      (goto-char (point-max)))))


(defun jaunder--idempotency-key ()
  "Return a fresh opaque idempotency key.
Self-contained (no `org-id' dependency): an md5 of local entropy."
  (md5 (format "%s%s%s" (random) (float-time) (emacs-pid))))

(defun jaunder--create-with-retry (url xml &optional key)
  "POST XML to URL as a create, retrying transient failures with one key.
Sends a stable `Idempotency-Key' header so the server dedups a retried create.
Retries a signalled transport error or a 5xx status, up to 3 attempts total
(backoff ~1s then ~2s); a 4xx or 2xx returns immediately.  Returns the
`jaunder--http-request' response plist.  The ephemeral key lives only for this
call unless KEY is supplied by durable create recovery."
  (let ((key (or key (jaunder--idempotency-key)))
        (delays '(1 2))
        (attempt 0)
        resp)
    (while (null resp)
      (setq attempt (1+ attempt))
      (let ((r (condition-case err
                   (jaunder--http-request "POST" url xml jaunder--entry-content-type
                                          (list (cons "Idempotency-Key" key)))
                 (plz-error (if (< attempt 3) 'retry (signal (car err) (cdr err)))))))
        (cond
         ((eq r 'retry) (sleep-for (pop delays)))
         ((and (integerp (plist-get r :status))
               (<= 500 (plist-get r :status) 599)
               (< attempt 3))
          (sleep-for (pop delays)))
         (t (setq resp r)))))
    resp))

(defun jaunder--create-intent (xml)
  "Persist and return the durable create intent for exact sent XML.
The digest distinguishes a later local edit from the Entry that may already
have committed after a response-less request."
  (let ((key (or (jaunder--buffer-property "JAUNDER_CREATE_KEY")
                 (jaunder--idempotency-key))))
    (unless (jaunder--buffer-property "JAUNDER_CREATE_KEY")
      (jaunder--set-property "JAUNDER_CREATE_KEY" key)
      (jaunder--set-property "JAUNDER_CREATE_DIGEST" (secure-hash 'sha256 xml))
      (jaunder--set-property "JAUNDER_CREATE_ATTEMPT_AT"
                             (format-time-string "%Y-%m-%dT%H:%M:%SZ" nil t))
      (save-buffer))
    (list :key key
          :matches (if (equal (jaunder--buffer-property "JAUNDER_CREATE_DIGEST")
                              (secure-hash 'sha256 xml))
                       'matched
                     'changed))))

(defun jaunder-publish (&optional force-draft)
  "Publish the current buffer's org post over AtomPub.
Resolves the blog from the buffer's file, records the machine zone when unset,
maps + validates, uploads media (sent body only), sends (POST create / PUT with
If-Match on update), writes back server values (ID first), and renames the temp
file to <slug>.org.  With FORCE-DRAFT (see `jaunder-save-draft') pushes an
`app:draft' regardless of JAUNDER_STATUS.  A non-2xx create leaves any
previously authored content intact but retains its durable create intent for
safe retry."
  (interactive)
  (let ((file (or (buffer-file-name)
                  (error "jaunder: buffer is not visiting a file"))))
    (jaunder--call-with-blog
     file
     (lambda ()
       (let* ((status (jaunder--buffer-property "JAUNDER_STATUS"))
              (date-raw (jaunder--buffer-keyword "DATE"))
              (tz (jaunder--buffer-property "JAUNDER_DATE_TZ"))
              (id (jaunder--buffer-property "JAUNDER_ID"))
              (synced (jaunder--buffer-property "JAUNDER_SYNCED"))
              (entry (jaunder--org->atom)))
         (when force-draft (jaunder--force-draft entry))
         ;; Validate BEFORE any buffer write, so a rejected publish leaves the
         ;; on-disk file pristine.
         (jaunder--validate-publish entry status date-raw tz)
         ;; Record the machine zone (idempotent) so #+DATE: is interpreted in a
         ;; recorded zone on later machines.  A first-publish's org->atom above
         ;; already used the local zone, which equals the captured name.
         (jaunder--ensure-date-tz)
         ;; Authoring-hygiene warning: `tz' is the zone recorded
         ;; *before* the capture above, so a difference means the
         ;; author moved machines since recording it.
         (jaunder--warn-zone-mismatch tz)
         ;; Warn (once per session per blog) if the server won't
         ;; honour the per-entry text/org content type.
         (jaunder--warn-missing-format-media-type
          (jaunder--active-base-url))
         (setf (jaunder-entry-body entry)
               (jaunder--localize-media (jaunder-entry-body entry)))
         (let* ((xml (jaunder--atom-entry->xml entry))
                (intent (unless id (jaunder--create-intent xml)))
                (resp (if id
                          (jaunder--http-request
                           "PUT"
                           (jaunder--member-url id)
                           xml jaunder--entry-content-type
                           (when synced (list (cons "If-Match" synced))))
                        (jaunder--create-with-retry
                         (jaunder--build-url (jaunder--active-base-url) "atompub"
                                             (jaunder--active-username) "posts")
                         xml (plist-get intent :key))))
                (code (plist-get resp :status)))
           (unless (memq code '(200 201))
             (error "jaunder: publish failed (HTTP %s)" code))
           (let ((slug (jaunder--write-back resp (null id)
                                            (plist-get intent :matches)
                                            (and id synced))))
             (when slug (jaunder--rename-to-slug slug))
             (message "jaunder: published %s" (or slug "")))))))))

(defun jaunder-save-draft ()
  "Publish the current buffer as a server-side draft (forces `app:draft')."
  (interactive)
  (jaunder-publish t))


(provide 'jaunder-publish)
;;; jaunder-publish.el ends here
