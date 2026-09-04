;;; jaunder-publish-test.el --- ERT suite for jaunder-publish -*- lexical-binding: t; -*-

;;; Commentary:
;; Unit tests for the corresponding jaunder package module.

;;; Code:

(require 'ert)
(require 'jaunder)

(defun jaunder-test--response (status headers body)
  "Build a `jaunder--http-request'-shaped plist for tests."
  (list :status status
        :headers (mapcar (lambda (h) (cons (downcase (car h)) (cdr h))) headers)
        :body body))

;;; Publish-time warnings (shared idiom) ----------------------------------

(defmacro jaunder-test--capturing-warnings (&rest body)
  "Run BODY with `display-warning' captured; return the list of (TYPE MSG LEVEL).
Lets the warning tests assert on emitted warnings without touching the real
`*Warnings*' buffer."
  (declare (indent 0))
  `(let (jaunder-test--warnings)
     (cl-letf (((symbol-function 'display-warning)
                (lambda (type message &optional level &rest _)
                  (push (list type message level) jaunder-test--warnings))))
              ,@body)
     (nreverse jaunder-test--warnings)))

;;; publish validation + Location->id + force-draft

(ert-deftest jaunder-validate-publish-rejects-empty-body ()
  (let ((e (jaunder--make-entry :body "   \n")))
    (should-error (jaunder--validate-publish e "published" nil nil))))

(ert-deftest jaunder-validate-publish-scheduled-needs-future ()
  (let ((e (jaunder--make-entry :body "x")))
    (should-error (jaunder--validate-publish e "scheduled" "[2000-01-01 Sat 00:00]" nil))
    ;; A far-future date passes.
    (should-not (jaunder--validate-publish e "scheduled" "[2999-01-01 Tue 00:00]" nil))))

(ert-deftest jaunder-location->id-extracts-numeric-tail ()
  (should (equal (jaunder--location->id "https://x/atompub/alice/posts/42") "42"))
  (should (equal (jaunder--location->id "https://x/atompub/alice/posts/42/") "42"))
  (should (null (jaunder--location->id nil))))

(ert-deftest jaunder-force-draft-sets-draft-and-clears-published ()
  ;; A dated, non-draft entry forced to draft must not carry <published>:
  ;; the serializer emits <published> whenever the slot is set, independent of
  ;; the draft flag, so force-draft has to nil it (spec invariant).
  (let ((e (jaunder--make-entry :body "x" :draft nil :content-type "text/org"
                                :published "2026-07-01T13:00:00Z")))
    (jaunder--force-draft e)
    (should (jaunder-entry-draft e))
    (should (null (jaunder-entry-published e)))
    ;; And the wire entry indeed omits <published>.
    (should-not (string-match-p "<published>" (jaunder--atom-entry->xml e)))))

;;; rename temp draft to <slug>.org

(ert-deftest jaunder-rename-to-slug-renames-and-handles-collision ()
  (let ((dir (make-temp-file "jaunder-rn-" t)))
    (unwind-protect
        (let ((tmp (expand-file-name "draft-20260101T000000.org" dir)))
          (with-temp-file tmp (insert "x"))
          (let ((buf (find-file-noselect tmp)))
            (unwind-protect
                (with-current-buffer buf
                  (let ((p (jaunder--rename-to-slug "my-post")))
                    (should (equal (file-name-nondirectory p) "my-post.org"))
                    (should (equal (buffer-file-name) p))
                    (should (file-exists-p p))
                    (should-not (file-exists-p tmp))
                    ;; Idempotent: renaming to the same slug is a no-op.
                    (should (equal (jaunder--rename-to-slug "my-post") p))))
              (kill-buffer buf)))
          ;; Collision: a second post with the same slug gets -1.
          (let ((tmp2 (expand-file-name "draft-20260101T000001.org" dir)))
            (with-temp-file tmp2 (insert "y"))
            (let ((buf2 (find-file-noselect tmp2)))
              (unwind-protect
                  (with-current-buffer buf2
                    (should (equal (file-name-nondirectory
                                    (jaunder--rename-to-slug "my-post"))
                                   "my-post-1.org")))
                (kill-buffer buf2)))))
      (delete-directory dir t))))

(ert-deftest jaunder-write-back-create-writes-id-first ()
  (with-temp-buffer
    (org-mode)
    (insert "#+TITLE: T\n#+PROPERTY: JAUNDER_STATUS published\n\nBody.\n")
    (set-visited-file-name (make-temp-file "jaunder-wb-" nil ".org") nil t)
    (unwind-protect
        (let ((resp (jaunder-test--response
                     201
                     '(("Location" . "https://x/atompub/alice/posts/42")
                       ("ETag" . "\"abc\""))
                     (concat "<entry xmlns=\"http://www.w3.org/2005/Atom\""
                             " xmlns:j=\"https://jaunder.org/ns/atompub\">"
                             "<content type=\"text/org\">Body</content>"
                             "<published>2026-07-01T13:00:00+00:00</published>"
                             "<j:slug>my-post</j:slug></entry>"))))
          (should (equal (jaunder--write-back resp t) "my-post"))
          (should (equal (jaunder--buffer-property "JAUNDER_ID") "42"))
          (should (equal (jaunder--buffer-property "JAUNDER_SLUG") "my-post"))
          (should (equal (jaunder--buffer-property "JAUNDER_SYNCED") "\"abc\""))
          ;; The server's <published> offset is dropped to the canonical UTC
          ;; instant (tz-independent, so deterministic across machines).
          (should (equal (jaunder--buffer-property "JAUNDER_DATE_UTC")
                         "2026-07-01T13:00:00Z"))
          ;; publish-now (no author #+DATE:) → #+DATE: rendered from server time.
          (should (jaunder--buffer-keyword "DATE")))
      (when (buffer-file-name) (delete-file (buffer-file-name))))))

(ert-deftest jaunder-write-back-update-keeps-id ()
  (with-temp-buffer
    (org-mode)
    (insert "#+TITLE: T\n#+PROPERTY: JAUNDER_ID 7\n#+DATE: [2026-07-01 Wed 09:00]\n\nBody.\n")
    (set-visited-file-name (make-temp-file "jaunder-wb-" nil ".org") nil t)
    (unwind-protect
        (let ((resp (jaunder-test--response
                     200 '(("ETag" . "\"z\""))
                     (concat "<entry xmlns=\"http://www.w3.org/2005/Atom\""
                             " xmlns:j=\"https://jaunder.org/ns/atompub\">"
                             "<content type=\"text/org\">Body</content>"
                             "<j:slug>my-post</j:slug></entry>"))))
          (jaunder--write-back resp nil)     ; created = nil (update)
          (should (equal (jaunder--buffer-property "JAUNDER_ID") "7"))  ; unchanged
          (should (equal (jaunder--buffer-property "JAUNDER_SYNCED") "\"z\"")))
      (when (buffer-file-name) (delete-file (buffer-file-name))))))

(ert-deftest jaunder-new-post-writes-timestamped-draft ()
  (let ((dir (make-temp-file "jaunder-np-" t)))
    (unwind-protect
        (let ((path (jaunder--new-post-in dir "20260703T101500")))
          (should (equal (file-name-nondirectory path) "draft-20260703T101500.org"))
          (should (file-exists-p path))
          (let ((buf (find-file-noselect path)))
            (unwind-protect
                (with-current-buffer buf
                  (should (equal (jaunder--buffer-property "JAUNDER_STATUS") "draft"))
                  (should (jaunder--buffer-keyword "TITLE"))   ; present (may be empty)
                  (should (jaunder--buffer-keyword "DATE")))
              (kill-buffer buf))))
      (delete-directory dir t))))

;;; Cross-cutting shared-idiom tests

(ert-deftest jaunder-publish-warnings-are-independent ()
  ;; AC-S2: suppressing one warning leaves the other two firing.
  (let ((jaunder--service-doc-cache nil))
    (cl-letf (((symbol-function 'jaunder--current-zone-name)
               (lambda () "Europe/London"))
              ((symbol-function 'jaunder--git-toplevel) (lambda (_dir) "/repo"))
              ((symbol-function 'jaunder--git-tracked-p) (lambda (_top _path) nil))
              ((symbol-function 'jaunder--fetch-service-features)
               (lambda (_base) '("slug"))))
             (let* ((jaunder-warn-zone-mismatch nil)
                    (msgs (mapcar
                           (lambda (w) (nth 1 w))
                           (jaunder-test--capturing-warnings
                            (jaunder--warn-zone-mismatch "America/New_York")
                            (jaunder--warn-untracked-media
                             (list (list :path "/repo/a.png")))
                            (jaunder--warn-missing-format-media-type "https://blog")))))
               (should (= (length msgs) 2))
               (should-not (seq-find (lambda (m) (string-match-p "timezone" m)) msgs))
               (should (seq-find (lambda (m) (string-match-p "not tracked" m)) msgs))
               (should (seq-find (lambda (m) (string-match-p "format-media-type" m)) msgs))))))

(ert-deftest jaunder-publish-request-identical-with-warnings ()
  ;; AC-S1: the publish request/return is byte-identical whether the warnings
  ;; fire or are all suppressed — they are side-effect-free on the publish path.
  (let* ((dir (file-truename (make-temp-file "jaunder-s1-" t)))
         (file (expand-file-name "post.org" dir))
         (jaunder-blogs (list (cons dir (list :base-url "https://blog.example"
                                              :username "alice"))))
         (captured nil))
    (unwind-protect
        (cl-letf (((symbol-function 'jaunder--current-zone-name)
                   (lambda () "Europe/London"))
                  ((symbol-function 'jaunder--fetch-service-features)
                   (lambda (_base) '("slug")))
                  ((symbol-function 'jaunder--write-back) (lambda (&rest _) nil))
                  ((symbol-function 'jaunder--http-request)
                   (lambda (method _url &optional body &rest _)
                     (when (member method '("POST" "PUT"))
                       (setq captured body))
                     (jaunder-test--response 201 nil ""))))
                 (cl-flet ((do-publish ()
                             (with-temp-buffer
                               (org-mode)
                               (insert (concat "#+TITLE: T\n"
                                               "#+PROPERTY: JAUNDER_DATE_TZ America/New_York\n"
                                               "\nBody.\n"))
                               (set-visited-file-name file nil t)
                               (setq captured nil)
                               (jaunder-publish)
                               (set-buffer-modified-p nil)
                               captured)))
                          ;; All three warnings WANT to fire here (recorded zone differs,
                          ;; service doc lacks the feature).
                          (let ((body-enabled (let ((jaunder--service-doc-cache nil))
                                                (do-publish)))
                                (body-suppressed
                                 (let ((jaunder-warn-zone-mismatch nil)
                                       (jaunder-warn-untracked-media nil)
                                       (jaunder-warn-missing-format-media-type nil)
                                       (jaunder--service-doc-cache nil))
                                   (do-publish))))
                            (should (stringp body-enabled))
                            (should (equal body-enabled body-suppressed)))))
      (delete-directory dir t))))

;;; #79 — create idempotency key + auto-retry

(ert-deftest jaunder-idempotency-key-is-fresh-and-nonempty ()
  ;; AC-C1/C6: each call yields a non-empty token, and two calls differ.
  (let ((k1 (jaunder--idempotency-key))
        (k2 (jaunder--idempotency-key)))
    (should (stringp k1))
    (should (> (length k1) 0))
    (should-not (equal k1 k2))))

(ert-deftest jaunder-create-retry-sends-key-and-retries-5xx ()
  ;; AC-C2/C3/C4: a 5xx retries with the SAME key, then succeeds.
  (let ((calls 0)
        (keys nil))
    (cl-letf (((symbol-function 'sleep-for) (lambda (&rest _) nil))
              ((symbol-function 'jaunder--http-request)
               (lambda (_method _url _body _ctype extra-headers)
                 (setq calls (1+ calls))
                 (push (cdr (assoc "Idempotency-Key" extra-headers)) keys)
                 (if (= calls 1)
                     '(:status 503 :body "")
                   '(:status 201 :body "ok")))))
             (let ((resp (jaunder--create-with-retry "http://x/posts" "<xml/>")))
               (should (= (plist-get resp :status) 201))
               (should (= calls 2))
               (should (equal (nth 0 keys) (nth 1 keys)))
               (should (> (length (nth 0 keys)) 0))))))

(ert-deftest jaunder-create-retry-does-not-retry-4xx ()
  ;; AC-C3: a 4xx returns immediately, no retry.
  (let ((calls 0))
    (cl-letf (((symbol-function 'sleep-for) (lambda (&rest _) nil))
              ((symbol-function 'jaunder--http-request)
               (lambda (&rest _)
                 (setq calls (1+ calls))
                 '(:status 400 :body ""))))
             (let ((resp (jaunder--create-with-retry "http://x/posts" "<xml/>")))
               (should (= (plist-get resp :status) 400))
               (should (= calls 1))))))

(ert-deftest jaunder-create-retry-exhausts-on-transport-error ()
  ;; AC-C5: after 3 transport failures the publish errors. The stub signals a
  ;; `plz-error' subtype because that is what `jaunder--http-request' re-signals
  ;; when a transport failure carries no response.
  (let ((calls 0))
    (cl-letf (((symbol-function 'sleep-for) (lambda (&rest _) nil))
              ((symbol-function 'jaunder--http-request)
               (lambda (&rest _)
                 (setq calls (1+ calls))
                 (signal 'plz-curl-error (list "Curl error" (make-plz-error))))))
             (should-error (jaunder--create-with-retry "http://x/posts" "<xml/>"))
             (should (= calls 3)))))

(ert-deftest jaunder-create-retry-does-not-retry-a-config-error ()
  ;; #945: a non-transport error (e.g. no auth-source entry) cannot succeed on
  ;; retry, so it surfaces on the first attempt with no backoff.
  (let ((calls 0)
        (sleeps 0))
    (cl-letf (((symbol-function 'sleep-for) (lambda (&rest _) (setq sleeps (1+ sleeps))))
              ((symbol-function 'jaunder--http-request)
               (lambda (&rest _)
                 (setq calls (1+ calls))
                 (error "jaunder: no auth-source entry for a@b"))))
             (should-error (jaunder--create-with-retry "http://x/posts" "<xml/>"))
             (should (= calls 1))
             (should (= sleeps 0)))))

(ert-deftest jaunder-publish-commands-require-visiting-file ()
  "Interactive publish must not silently manufacture request context."
  (with-temp-buffer
    (should-error (jaunder-publish) :type 'error)
    (should-error (jaunder--rename-to-slug "post") :type 'error)))

(ert-deftest jaunder-new-post-collects-metadata-before-creating-file ()
  "The interactive command writes one coherent template from all prompt answers."
  (let* ((root (file-name-as-directory (make-temp-file "jaunder-new-post-" t)))
         (jaunder-blogs
          (list (cons root '(:base-url "https://blog" :username "alice"))))
         (default-directory root)
         (tag-answers '("Rust" "rust" "emacs" ""))
         tag-collections
         created)
    (unwind-protect
        (cl-letf
         (((symbol-function 'read-string) (lambda (&rest _) "Prompted Post"))
          ((symbol-function 'completing-read)
           (lambda (prompt collection &rest _)
             (cond
              ((string-prefix-p "Tag" prompt)
               (push collection tag-collections)
               (pop tag-answers))
              ((string-prefix-p "Status" prompt) "published")
              (t (error "unexpected prompt: %s" prompt)))))
          ((symbol-function 'jaunder--http-request)
           (lambda (method url &rest _)
             (should (equal method "GET"))
             (should (equal url "https://blog/atompub/service"))
             (list
              :status 200
              :body
              (concat
               "<app:service xmlns:app=\"http://www.w3.org/2007/app\""
               " xmlns:atom=\"http://www.w3.org/2005/Atom\">"
               "<app:workspace>"
               "<app:collection href=\"https://blog/atompub/alice/posts\">"
               "<app:accept>application/atom+xml;type=entry</app:accept>"
               "<app:categories><atom:category term=\"rust\"/>"
               "<atom:category term=\"emacs\"/></app:categories>"
               "</app:collection>"
               "<app:collection href=\"https://blog/atompub/alice/media\">"
               "<app:accept>image/*</app:accept>"
               "<app:categories><atom:category term=\"ignored\"/></app:categories>"
               "</app:collection></app:workspace></app:service>")))))
         (jaunder-new-post nil)
         (setq created (current-buffer))
         (should (equal (jaunder--buffer-keyword "TITLE") "Prompted Post"))
         (should (equal (jaunder--buffer-keyword "KEYWORDS") "Rust, emacs"))
         (should (equal (jaunder--buffer-property "JAUNDER_STATUS") "published"))
         (should-not (jaunder--buffer-property "JAUNDER_FORMAT"))
         (should (= (length tag-collections) 4))
         (dolist (collection tag-collections)
           (should (equal collection '("rust" "emacs")))))
      (when (buffer-live-p created) (kill-buffer created))
      (delete-directory root t))))

(ert-deftest jaunder-new-post-cancellation-leaves-no-file ()
  "Cancelling metadata collection cannot leave a partial local Post."
  (let* ((root (file-name-as-directory (make-temp-file "jaunder-cancel-" t)))
         (jaunder-blogs
          (list (cons root '(:base-url "https://blog" :username "alice"))))
         (default-directory root))
    (unwind-protect
        (cl-letf
         (((symbol-function 'read-string) (lambda (&rest _) "Cancelled"))
          ((symbol-function 'completing-read)
           (lambda (prompt &rest _)
             (cond
              ((string-prefix-p "Tag" prompt) "")
              ((string-prefix-p "Status" prompt) (signal 'quit nil))
              (t (error "unexpected prompt: %s" prompt)))))
          ((symbol-function 'jaunder--http-request)
           (lambda (&rest _)
             '(:status 200
                       :body
                       "<service><workspace><collection><accept>application/atom+xml;type=entry</accept></collection></workspace></service>"))))
         (should
          (eq (condition-case nil
                  (jaunder-new-post nil)
                (quit 'cancelled))
              'cancelled))
         (should-not
          (directory-files root nil "\\`draft-[^.]+\\.org\\'")))
      (delete-directory root t))))

(ert-deftest jaunder-new-post-aborts-when-selected-blog-disappears ()
  "A configuration race cannot create a Post in a no-longer-configured root."
  (let* ((root (file-name-as-directory (make-temp-file "jaunder-removed-" t)))
         (other (file-name-as-directory (make-temp-file "jaunder-other-" t)))
         (jaunder-blogs
          (list (cons root '(:base-url "https://blog" :username "alice"))))
         (default-directory other))
    (unwind-protect
        (cl-letf
         (((symbol-function 'completing-read)
           (lambda (prompt &rest _)
             (should (string-prefix-p "Blog" prompt))
             (setq jaunder-blogs nil)
             root)))
         (should-error (jaunder-new-post nil) :type 'error)
         (should-not
          (directory-files root nil "\\`draft-[^.]+\\.org\\'")))
      (delete-directory root t)
      (delete-directory other t))))

(ert-deftest jaunder-new-post-malformed-blog-keeps-free-text-tags ()
  "Invalid server configuration is visible but does not block local creation."
  (let* ((root (file-name-as-directory (make-temp-file "jaunder-malformed-" t)))
         (jaunder-blogs
          (list (cons root '(:base-url "relative" :username "alice"))))
         (default-directory root)
         messages
         created)
    (unwind-protect
        (cl-letf
         (((symbol-function 'read-string) (lambda (&rest _) ""))
          ((symbol-function 'completing-read)
           (lambda (prompt &rest _)
             (cond
              ((string-prefix-p "Tag" prompt) "")
              ((string-prefix-p "Status" prompt) "draft")
              (t (error "unexpected prompt: %s" prompt)))))
          ((symbol-function 'jaunder--http-request)
           (lambda (&rest _) (error "unexpected server request")))
          ((symbol-function 'message)
           (lambda (format-string &rest args)
             (push (apply #'format format-string args) messages))))
         (jaunder-new-post nil)
         (setq created (current-buffer))
         (should
          (cl-some
           (lambda (text) (string-match-p "malformed :base-url" text))
           messages)))
      (when (buffer-live-p created) (kill-buffer created))
      (delete-directory root t))))

(ert-deftest jaunder-new-post-falls-back-to-valid-free-text-tags ()
  "Unavailable completion stays visible without blocking new valid Tag labels."
  (let* ((root (file-name-as-directory (make-temp-file "jaunder-tags-" t)))
         (jaunder-blogs
          (list (cons root '(:base-url "https://blog" :username "alice"))))
         (default-directory root)
         (tag-answers '("bad tag" "NewTag" ""))
         messages
         created)
    (unwind-protect
        (cl-letf
         (((symbol-function 'read-string) (lambda (&rest _) ""))
          ((symbol-function 'completing-read)
           (lambda (prompt &rest _)
             (cond
              ((string-prefix-p "Tag" prompt) (pop tag-answers))
              ((string-prefix-p "Status" prompt) "draft")
              (t (error "unexpected prompt: %s" prompt)))))
          ((symbol-function 'jaunder--http-request)
           (lambda (&rest _)
             '(:status 200
                       :body
                       "<service><workspace><collection><accept>application/atom+xml;type=entry</accept><categories><category term=\"bad tag\"/></categories></collection></workspace></service>")))
          ((symbol-function 'message)
           (lambda (format-string &rest args)
             (push (apply #'format format-string args) messages))))
         (jaunder-new-post nil)
         (setq created (current-buffer))
         (should (equal (jaunder--buffer-keyword "KEYWORDS") "NewTag"))
         (should
          (cl-some
           (lambda (text)
             (string-match-p "Tag completion unavailable" text))
           messages))
         (should
          (cl-some
           (lambda (text)
             (string-match-p "Tag must match" text))
           messages)))
      (when (buffer-live-p created) (kill-buffer created))
      (delete-directory root t))))

(ert-deftest jaunder-new-post-reprompts-until-schedule-is-future ()
  "Scheduled creation never writes a file from invalid or non-future input."
  (let* ((root (file-name-as-directory (make-temp-file "jaunder-scheduled-" t)))
         (jaunder-blogs
          (list (cons root '(:base-url "https://blog" :username "alice"))))
         (default-directory root)
         (now (encode-time 0 0 12 30 8 2026))
         (past (time-subtract now (seconds-to-time 60)))
         (same-minute-future (time-add now (seconds-to-time 30)))
         (future (time-add now (seconds-to-time 3600)))
         (date-answers (list 'invalid now past same-minute-future future))
         (date-prompts 0)
         created)
    (unwind-protect
        (cl-letf
         (((symbol-function 'read-string) (lambda (&rest _) ""))
          ((symbol-function 'completing-read)
           (lambda (prompt &rest _)
             (cond
              ((string-prefix-p "Tag" prompt) "")
              ((string-prefix-p "Status" prompt) "scheduled")
              (t (error "unexpected prompt: %s" prompt)))))
          ((symbol-function 'jaunder--http-request)
           (lambda (&rest _)
             '(:status 200
                       :body
                       "<service><workspace><collection><accept>application/atom+xml;type=entry</accept></collection></workspace></service>")))
          ((symbol-function 'current-time) (lambda () now))
          ((symbol-function 'org-read-date)
           (lambda (&rest _)
             (cl-incf date-prompts)
             (should-not
              (directory-files root nil "\\`draft-[^.]+\\.org\\'"))
             (let ((answer (pop date-answers)))
               (if (eq answer 'invalid)
                   (error "invalid date")
                 answer)))))
         (jaunder-new-post nil)
         (setq created (current-buffer))
         (should (= date-prompts 5))
         (should
          (equal
           (jaunder--buffer-keyword "DATE")
           (format-time-string "[%Y-%m-%d %a %H:%M]" future))))
      (when (buffer-live-p created) (kill-buffer created))
      (delete-directory root t))))

(ert-deftest jaunder-new-post-prefix-creates-minimal-post-without-prompts ()
  "The prefix path preserves minimal local creation without server dependency."
  (let* ((root (file-name-as-directory (make-temp-file "jaunder-minimal-" t)))
         (jaunder-blogs
          (list (cons root '(:base-url "https://blog" :username "alice"))))
         (default-directory root)
         created)
    (unwind-protect
        (cl-letf
         (((symbol-function 'read-string)
           (lambda (&rest _) (error "unexpected title prompt")))
          ((symbol-function 'completing-read)
           (lambda (&rest _) (error "unexpected completion prompt")))
          ((symbol-function 'jaunder--http-request)
           (lambda (&rest _) (error "unexpected server request"))))
         (jaunder-new-post '(4))
         (setq created (current-buffer))
         (should (equal (jaunder--buffer-keyword "TITLE") ""))
         (should (equal (jaunder--buffer-keyword "KEYWORDS") ""))
         (should (equal (jaunder--buffer-property "JAUNDER_STATUS") "draft"))
         (should (= (point) (point-max))))
      (when (buffer-live-p created) (kill-buffer created))
      (delete-directory root t))))

(ert-deftest jaunder-new-post-prefix-rejects-unmatched-configured-location ()
  "Prompt-free creation fails rather than guessing among configured blogs."
  (let* ((root (file-name-as-directory (make-temp-file "jaunder-configured-" t)))
         (other (file-name-as-directory (make-temp-file "jaunder-unmatched-" t)))
         (jaunder-blogs
          (list (cons root '(:base-url "https://blog" :username "alice"))))
         (default-directory other))
    (unwind-protect
        (cl-letf
         (((symbol-function 'read-string)
           (lambda (&rest _) (error "unexpected title prompt")))
          ((symbol-function 'completing-read)
           (lambda (&rest _) (error "unexpected completion prompt")))
          ((symbol-function 'jaunder--http-request)
           (lambda (&rest _) (error "unexpected server request"))))
         (should-error (jaunder-new-post '(4)) :type 'user-error)
         (should-not
          (directory-files root nil "\\`draft-[^.]+\\.org\\'"))
         (should-not
          (directory-files other nil "\\`draft-[^.]+\\.org\\'")))
      (delete-directory root t)
      (delete-directory other t))))

(ert-deftest jaunder-new-post-prefix-uses-directory-when-unconfigured ()
  "A wholly unconfigured client still has a deterministic prompt-free target."
  (let* ((root (file-name-as-directory (make-temp-file "jaunder-unconfigured-" t)))
         (jaunder-blogs nil)
         (default-directory root)
         created)
    (unwind-protect
        (cl-letf
         (((symbol-function 'read-string)
           (lambda (&rest _) (error "unexpected title prompt")))
          ((symbol-function 'completing-read)
           (lambda (&rest _) (error "unexpected completion prompt")))
          ((symbol-function 'jaunder--http-request)
           (lambda (&rest _) (error "unexpected server request"))))
         (jaunder-new-post '(4))
         (setq created (current-buffer))
         (should
          (equal (file-name-directory (buffer-file-name)) root)))
      (when (buffer-live-p created) (kill-buffer created))
      (delete-directory root t))))

(ert-deftest jaunder-new-post-prompts-for-a-blog-when-directory-is-unmapped ()
  "New-post selects an explicit configured blog rather than using an unrelated cwd."
  (let* ((root (make-temp-file "jaunder-new-post-" t))
         (other (make-temp-file "jaunder-other-" t))
         (jaunder-blogs (list (cons (file-name-as-directory root)
                                    '(:base-url "https://blog" :username "alice"))))
         (default-directory other)
         selected)
    (unwind-protect
        (cl-letf (((symbol-function 'read-string) (lambda (&rest _) ""))
                  ((symbol-function 'completing-read)
                   (lambda (prompt &rest _)
                     (cond
                      ((string-prefix-p "Blog" prompt)
                       (setq selected (file-name-as-directory root)))
                      ((string-prefix-p "Tag" prompt) "")
                      ((string-prefix-p "Status" prompt) "draft")
                      (t (error "unexpected prompt: %s" prompt)))))
                  ((symbol-function 'jaunder--http-request)
                   (lambda (&rest _) '(:status 503)))
                  ((symbol-function 'format-time-string)
                   (lambda (&rest _) "20260829T000000")))
                 (jaunder-new-post)
                 (should (equal selected (file-name-as-directory root)))
                 (should (equal (buffer-file-name)
                                (expand-file-name "draft-20260829T000000.org" root))))
      (when (buffer-file-name) (kill-buffer (current-buffer)))
      (delete-directory root t)
      (delete-directory other t))))

(ert-deftest jaunder-new-post-uses-default-directory-without-configured-blogs ()
  "Without configured blogs, a draft is deliberately created in the current directory."
  (let* ((root (file-name-as-directory (make-temp-file "jaunder-new-post-" t)))
         (default-directory root)
         (jaunder-blogs nil)
         created)
    (unwind-protect
        (cl-letf (((symbol-function 'read-string) (lambda (&rest _) ""))
                  ((symbol-function 'completing-read)
                   (lambda (prompt &rest _)
                     (cond
                      ((string-prefix-p "Tag" prompt) "")
                      ((string-prefix-p "Status" prompt) "draft")
                      (t (error "unexpected prompt: %s" prompt)))))
                  ((symbol-function 'format-time-string)
                   (lambda (&rest _) "20260829T000001")))
                 (jaunder-new-post)
                 (setq created (current-buffer))
                 (should (equal (buffer-file-name)
                                (expand-file-name "draft-20260829T000001.org" root))))
      (when (buffer-live-p created) (kill-buffer created))
      (delete-directory root t))))

;;; jaunder-publish-test.el ends here
