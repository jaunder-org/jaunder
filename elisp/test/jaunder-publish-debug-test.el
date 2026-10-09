;;; jaunder-publish-debug-test.el --- Publish and deletion diagnostics -*- lexical-binding: t; -*-

;;; Commentary:
;; Real publishing, durable create recovery, conditional updates, checkpoints
;; and deletion retain exact wire/local effects with diagnostics off/on.

;;; Code:

(require 'ert)
(require 'cl-lib)
(require 'rx)
(require 'jaunder)
(load (expand-file-name "jaunder-debug-boundary-fixture.el"
                        (file-name-directory (or load-file-name buffer-file-name))) nil t)

(defun jaunder-publish-debug--bytes (path)
  "Read literal bytes from PATH."
  (with-temp-buffer (set-buffer-multibyte nil) (insert-file-contents-literally path) (buffer-string)))

(defun jaunder-publish-debug--assert-intent-before-transfer (text invocations)
  "Require complete intent preparation before each transfer in TEXT, INVOCATIONS times."
  (let (events)
    (dolist (line (split-string text "\n" t))
      (when (string-match (rx " label=" (group (or "publish.recover" "publish.create"))
                              " phase=" (group (or "start" "end"))) line)
        (push (cons (match-string 1 line) (match-string 2 line)) events)))
    (should (equal (nreverse events)
                   (apply #'append
                          (make-list invocations '(("publish.recover" . "start") ("publish.recover" . "end")
                                                   ("publish.create" . "start") ("publish.create" . "end"))))))))

(defun jaunder-publish-debug--run (enabled scenario &optional kind)
  "Run real SCENARIO with ENABLED, optionally faulting native wire or rename with KIND."
  (jaunder-debug-boundary--with-session
   (let* ((root (file-name-as-directory (make-temp-file "jaunder-publish-private-" t)))
          (path (expand-file-name "private-input.org" root))
          (jaunder-blogs (list (cons root '(:base-url "https://private.example" :username "private-user"))))
          (jaunder--service-doc-cache nil)
          (formatter (symbol-function 'format-time-string))
          (saver (symbol-function 'jaunder--save-buffer-silently))
          (renamer (symbol-function 'rename-file))
          (fixed (encode-time 0 0 0 7 10 2026 t))
          (failure '("private-condition" 17))
          (recover (memq scenario '(recover changed-recover)))
          (existing (memq scenario '(update delete decline)))
          (losing recover)
          (saves 0) (renames 0) requests before-replay first-result result buffer)
     (with-temp-file path
       (insert "#+TITLE: private-title\n#+DATE: [2026-10-07 Wed 00:00]\n"
               "#+PROPERTY: JAUNDER_STATUS published\n#+PROPERTY: JAUNDER_DATE_TZ UTC\n"
               "#+PROPERTY: JAUNDER_AUDIENCE private\n")
       (when existing (insert "#+PROPERTY: JAUNDER_ID 42\n#+PROPERTY: JAUNDER_SYNCED \"private-old\"\n"))
       (insert "\nprivate-body\n"))
     (setq buffer (find-file-noselect path) jaunder-debug enabled)
     (unwind-protect
         (with-current-buffer buffer
           (cl-letf (((symbol-function 'format-time-string)
                      (lambda (format &optional _time zone) (funcall formatter format fixed zone)))
                     ((symbol-function 'jaunder--idempotency-key) (lambda () "private-key"))
                     ((symbol-function 'jaunder--save-buffer-silently)
                      (lambda () (setq saves (1+ saves)) (funcall saver)))
                     ((symbol-function 'rename-file)
                      (lambda (&rest args)
                        (setq renames (1+ renames))
                        (when (and kind (eq scenario 'rename)) (signal kind failure))
                        (apply renamer args)))
                     ((symbol-function 'y-or-n-p) (lambda (&rest _) (not (eq scenario 'decline))))
                     ((symbol-function 'jaunder--http-request)
                      (lambda (method url &optional body type headers)
                        (push (list method url body type headers) requests)
                        (when (and (not (equal method "GET"))
                                   (or losing (and kind (not (eq scenario 'rename)))))
                          (signal (or kind 'error) failure))
                        (cond
                         ((equal method "GET")
                          (list :status 200 :body
                                (concat "<service xmlns=\"http://www.w3.org/2007/app\""
                                        " xmlns:atom=\"http://www.w3.org/2005/Atom\""
                                        " xmlns:j=\"https://jaunder.org/ns/atompub\">"
                                        "<workspace><atom:title>private-service</atom:title>"
                                        "<j:extension version=\"1\" features=\"audience format-media-type\"/>"
                                        "</workspace></service>")))
                         ((equal method "DELETE") '(:status 204))
                         (t (list :status (if (or recover existing) 200 201)
                                  :headers '(("location" . "https://private.example/atompub/private-user/posts/42")
                                             ("etag" . "\"private-new\""))
                                  :body (concat "<entry xmlns=\"http://www.w3.org/2005/Atom\""
                                                " xmlns:j=\"https://jaunder.org/ns/atompub\">"
                                                "<j:slug>private-post</j:slug><j:audience>private</j:audience>"
                                                "<content type=\"text/org\">private-body</content></entry>")))))))
             (when recover
               (setq first-result (condition-case condition (jaunder-publish) (error condition) (quit condition)))
               (should (equal first-result (cons (or kind 'error) failure)))
               (setq before-replay (jaunder-publish-debug--bytes path) losing nil)
               (when (eq scenario 'changed-recover) (insert "private-edit\n") (save-buffer)))
             (setq result (condition-case condition
                              (pcase scenario
                                ((or 'delete 'decline) (jaunder-delete-post))
                                ('draft (jaunder-save-draft))
                                (_ (jaunder-publish)))
                            (error condition) (quit condition)))
             (when kind (should (equal result (cons kind failure))))
             ;; Paths are the sole per-run entropy in primary result projections.
             (when (and (listp result) (plist-member result :source-path))
               (setq result (copy-sequence result))
               (plist-put result :source-path (file-name-nondirectory (plist-get result :source-path)))
               (plist-put result :path (file-name-nondirectory (plist-get result :path))))
             (let ((text (when enabled (jaunder-debug-boundary--text))))
               (if enabled
                   (progn
                     (jaunder-debug-boundary--assert-tree
                      text
                      (cond (recover '("publish.post" "publish.post"))
                            ((eq scenario 'draft) '("publish.draft"))
                            ((memq scenario '(delete decline)) '("delete.post"))
                            (t '("publish.post")))
                      (append (when (eq scenario 'draft) '(("publish.post" . "publish.draft")))
                              '(("publish.validate" . "publish.post")
                                ("publish.recover" . "publish.post")
                                ("publish.create" . "publish.post")
                                ("publish.update" . "publish.post")
                                ("publish.checkpoint" . "publish.post"))))
                     (dolist (label (if (memq scenario '(delete decline)) '("delete.post")
                                      (append '("publish.post" "publish.validate")
                                              (when (eq scenario 'draft) '("publish.draft"))
                                              (if existing '("publish.update") '("publish.create" "publish.recover"))
                                              (unless (and kind (not (eq scenario 'rename))) '("publish.checkpoint")))))
                       (should (> (jaunder-debug-boundary--label-count label text) 0)))
                     (unless existing
                       (jaunder-publish-debug--assert-intent-before-transfer text (if recover 2 1)))
                     (unless (memq scenario '(delete decline))
                       (should (= (if recover 8 4) (jaunder-debug-boundary--label-count "publish.validate" text)))
                       (should (= (if (and kind (not (eq scenario 'rename))) 0 4)
                                  (jaunder-debug-boundary--label-count "publish.checkpoint" text))))
                     (should-not (string-match-p (rx (or "private" "https://")) text)))
                 (should (zerop jaunder--debug-id-counter))
                 (should-not (get-buffer jaunder--debug-buffer-name)))
               (list :result result :first-result first-result :before-replay before-replay
                     :requests (nreverse requests) :saves saves :renames renames
                     :live (buffer-live-p buffer)
                     :modified (and (buffer-live-p buffer) (buffer-modified-p buffer))
                     :files (mapcar (lambda (file) (cons (file-name-nondirectory file) (jaunder-publish-debug--bytes file)))
                                    (directory-files root t (rx ".org" string-end)))))))
       (when (buffer-live-p buffer)
         (with-current-buffer buffer (set-buffer-modified-p nil)) (kill-buffer buffer))
       (delete-directory root t)))))

(ert-deftest jaunder-publish-debug-real-create-draft-update-and-delete-off-on ()
  "Preserve actual create/draft/conditional-update/delete return and wire effects."
  (dolist (scenario '(create draft update delete decline))
    (let ((off (jaunder-publish-debug--run nil scenario))
          (on (jaunder-publish-debug--run t scenario)))
      (should (equal off on))
      (let ((requests (plist-get on :requests)))
        (pcase scenario
          ('decline (should-not requests) (should (plist-get on :live)))
          ('delete
           (should (equal (mapcar #'car requests) '("DELETE")))
           (should-not (plist-get on :files)) (should-not (plist-get on :live)))
          (_
           (should (equal (mapcar #'car requests) (list "GET" "GET" (if (eq scenario 'update) "PUT" "POST"))))
           (should (equal (mapcar #'car (plist-get on :files)) '("private-post.org")))
           (should (string-match-p "JAUNDER_ID 42" (cdar (plist-get on :files))))
           (should (equal (plist-get (plist-get on :result) :http-status) (if (eq scenario 'update) 200 201)))
           (if (eq scenario 'update)
               (should (equal (cdr (assoc "If-Match" (nth 4 (car (last requests))))) "\"private-old\""))
             (should (equal (cdr (assoc "Idempotency-Key" (nth 4 (car (last requests))))) "private-key")))
           (when (eq scenario 'draft)
             (should (string-match-p "<app:draft>yes</app:draft>" (nth 2 (car (last requests))))))))))))

(ert-deftest jaunder-publish-debug-real-durable-recovery-off-on ()
  "Response-loss replay reuses intent and marks changed authored input local-ahead."
  (dolist (scenario '(recover changed-recover))
    (let ((off (jaunder-publish-debug--run nil scenario))
          (on (jaunder-publish-debug--run t scenario)))
      (should (equal off on))
      (should (string-match-p "JAUNDER_CREATE_KEY private-key" (plist-get on :before-replay)))
      (should (equal (plist-get (plist-get on :result) :http-status) 200))
      (let ((posts (cl-remove-if-not (lambda (r) (equal (car r) "POST")) (plist-get on :requests)))
            (installed (cdar (plist-get on :files))))
        (should (= (length posts) 2))
        (should (equal (mapcar (lambda (r) (cdr (assoc "Idempotency-Key" (nth 4 r)))) posts)
                       '("private-key" "private-key")))
        (should-not (string-match-p "JAUNDER_CREATE_KEY" installed))
        (when (eq scenario 'changed-recover)
          (should (string-match-p "JAUNDER_LOCAL_AHEAD true" installed))
          (should (string-match-p "private-edit" installed)))))))

(ert-deftest jaunder-publish-debug-native-wire-and-rename-errors-quits ()
  "Native failures preserve exact data, checkpoints and partial local effects."
  (dolist (scenario '(create update delete rename))
    (dolist (kind '(error quit))
      (let ((off (jaunder-publish-debug--run nil scenario kind))
            (on (jaunder-publish-debug--run t scenario kind)))
        (should (equal off on))
        (should (plist-get on :live))
        (should-not (plist-get on :modified))
        (should (equal (mapcar #'car (plist-get on :files)) '("private-input.org")))
        (when (eq scenario 'rename)
          (should (string-match-p "JAUNDER_ID 42" (cdar (plist-get on :files)))))))))

(ert-deftest jaunder-publish-debug-reviewed-update-standalone-off-on-conditions ()
  "Standalone reviewed conditional sends preserve opaque returns and conditions."
  (jaunder-debug-boundary--with-session
   (let ((response (list :status 200 :body "private-response")) snapshots)
     (dolist (enabled '(nil t))
       (setq jaunder-debug enabled)
       (let (requests)
         (cl-letf (((symbol-function 'jaunder--http-request)
                    (lambda (&rest args) (push args requests) response)))
           (should (eq response (jaunder--send-reviewed-update
                                 "https://private.example/edit/42" "\"private-old\"" "private-xml"))))
         (push requests snapshots))
       (dolist (kind '(error quit))
         (let ((data '("private-condition" 17)))
           (cl-letf (((symbol-function 'jaunder--http-request) (lambda (&rest _) (signal kind data))))
             (should (equal (condition-case condition
                                (jaunder--send-reviewed-update "https://private.example/edit/42"
                                                               "\"private-old\"" "private-xml")
                              (error condition) (quit condition))
                            (cons kind data))))))
       (unless enabled (should (zerop jaunder--debug-id-counter))
               (should-not (get-buffer jaunder--debug-buffer-name))))
     (should (equal (car snapshots) (cadr snapshots)))
     (let ((text (jaunder-debug-boundary--text)))
       (should (= 6 (jaunder-debug-boundary--label-count "publish.update" text)))
       (jaunder-debug-boundary--assert-tree text '("publish.update" "publish.update" "publish.update"))
       (should-not (string-match-p "private" text))))))

(ert-deftest jaunder-publish-debug-create-retries-and-source-validation-standalone ()
  "Standalone retries retain delays/key/payload; validation keeps native results."
  (jaunder-debug-boundary--with-session
   (let (snapshots)
     (dolist (enabled '(nil t))
       (setq jaunder-debug enabled)
       (let ((responses '((:status 500) (:status 502) (:status 201))) requests delays)
         (cl-letf (((symbol-function 'jaunder--http-request)
                    (lambda (&rest args) (push args requests) (pop responses)))
                   ((symbol-function 'sleep-for) (lambda (delay) (push delay delays))))
           (should (equal (jaunder--create-with-retry "https://private.example/create" "private-xml" "private-key")
                          '(:status 201))))
         (should (= 3 (length requests)))
         (should (equal (nreverse delays) '(1 2)))
         (push requests snapshots))
       (should-not (jaunder--validate-publish (jaunder--make-entry :body "private-body") nil nil nil))
       (should-error (jaunder--validate-publish (jaunder--make-entry :body "") nil nil nil))
       (unless enabled (should (zerop jaunder--debug-id-counter))
               (should-not (get-buffer jaunder--debug-buffer-name))))
     (should (equal (car snapshots) (cadr snapshots)))
     (let ((text (jaunder-debug-boundary--text)))
       (should (= 2 (jaunder-debug-boundary--label-count "publish.create" text)))
       (should (= 4 (jaunder-debug-boundary--label-count "publish.validate" text)))
       (jaunder-debug-boundary--assert-tree text '("publish.create" "publish.validate" "publish.validate"))
       (should-not (string-match-p "private" text))))))

(ert-deftest jaunder-publish-debug-invalid-title-precedes-resolution-and-io ()
  "Rejected source still produces validation timing before configuration/I/O."
  (jaunder-debug-boundary--with-session
   (let (conditions)
     (dolist (enabled '(nil t))
       (setq jaunder-debug enabled)
       (with-temp-buffer
         (org-mode)
         (setq buffer-file-name "/private-root/private-input.org")
         (insert "#+TITLE: private-one\n#+TITLE: private-two\n\nprivate-body\n")
         (cl-letf (((symbol-function 'jaunder--resolve-blog) (lambda (&rest _) (ert-fail "source rejection resolved configuration")))
                   ((symbol-function 'jaunder--http-request) (lambda (&rest _) (ert-fail "source rejection performed I/O"))))
           (push (should-error (jaunder-publish)) conditions)))
       (unless enabled (should (zerop jaunder--debug-id-counter))
               (should-not (get-buffer jaunder--debug-buffer-name))))
     (should (equal (car conditions) (cadr conditions)))
     (let ((text (jaunder-debug-boundary--text)))
       (should (= 2 (jaunder-debug-boundary--label-count "publish.post" text)))
       (should (= 2 (jaunder-debug-boundary--label-count "publish.validate" text)))
       (jaunder-debug-boundary--assert-tree text '("publish.post")
                                            '(("publish.validate" . "publish.post")))
       (should-not (string-match-p "private" text))))))

(ert-deftest jaunder-publish-debug-disabled-owners-bypass-diagnostic-factories ()
  "Disabled real workflows cannot sample/format/update/allocate diagnostics."
  (let ((calls 0))
    (cl-letf (((symbol-function 'jaunder--debug-begin)
               (lambda (&rest _) (setq calls (1+ calls)) (ert-fail "disabled span setup")))
              ((symbol-function 'jaunder--debug-timestamp)
               (lambda (&rest _) (setq calls (1+ calls)) (ert-fail "disabled diagnostic clock")))
              ((symbol-function 'jaunder--debug-common-event)
               (lambda (&rest _) (setq calls (1+ calls)) (ert-fail "disabled event formatter")))
              ((symbol-function 'jaunder--debug-fields-internal)
               (lambda (&rest _) (setq calls (1+ calls)) (ert-fail "disabled field construction")))
              ((symbol-function 'jaunder--debug-buffer)
               (lambda (&rest _) (setq calls (1+ calls)) (ert-fail "disabled buffer allocation")))
              ((symbol-function 'jaunder--debug-complete)
               (lambda (&rest _) (setq calls (1+ calls)) (ert-fail "disabled terminal emission"))))
      (dolist (scenario '(create draft update recover changed-recover delete decline))
        (jaunder-publish-debug--run nil scenario)))
    ;; Count outside fault containment: an ancillary error cannot hide a call.
    (should (zerop calls))))

(defun jaunder-publish-debug--standalone-checkpoints (enabled)
  "Project real standalone intent, write-back and rename effects with ENABLED."
  (jaunder-debug-boundary--with-session
   (let* ((root (make-temp-file "jaunder-standalone-private-" t))
          (path (expand-file-name "private-input.org" root))
          (fixed (encode-time 0 0 0 7 10 2026 t))
          (formatter (symbol-function 'format-time-string))
          buffer)
     (with-temp-file path (insert "#+TITLE: private-title\n#+PROPERTY: JAUNDER_DATE_TZ UTC\n\nprivate-body\n"))
     (setq buffer (find-file-noselect path) jaunder-debug enabled)
     (unwind-protect
         (with-current-buffer buffer
           (cl-letf (((symbol-function 'format-time-string)
                      (lambda (format &optional _time zone) (funcall formatter format fixed zone)))
                     ((symbol-function 'jaunder--idempotency-key) (lambda () "private-key")))
             (let* ((first (jaunder--create-intent "private-xml"))
                    (intent-bytes (jaunder-publish-debug--bytes path))
                    (second (jaunder--create-intent "private-xml"))
                    (changed (jaunder--create-intent "private-changed-xml")))
               (should (equal first '(:key "private-key" :matches matched)))
               (should (equal first second))
               (should (equal changed '(:key "private-key" :matches changed)))
               (should (equal intent-bytes (jaunder-publish-debug--bytes path)))
               (let* ((slug (jaunder--write-back
                             '(:status 201 :headers (("location" . "https://private.example/posts/42")
                                                     ("etag" . "\"private-new\""))
                                       :body "<entry xmlns=\"http://www.w3.org/2005/Atom\" xmlns:j=\"https://jaunder.org/ns/atompub\"><j:slug>private-post</j:slug></entry>")
                             t 'matched))
                      (checkpoint (jaunder-publish-debug--bytes path))
                      (destination (jaunder--rename-to-slug slug))
                      (no-op (jaunder--rename-to-slug slug)))
                 (should (equal slug "private-post"))
                 (should (eq no-op (buffer-file-name)))
                 (should-not (file-exists-p path))
                 (should (equal checkpoint (jaunder-publish-debug--bytes destination)))
                 (should (string-match-p "JAUNDER_ID 42" checkpoint))
                 (should (string-match-p "JAUNDER_SYNCED \"private-new\"" checkpoint))
                 (should-not (string-match-p "JAUNDER_CREATE_KEY" checkpoint))
                 (should-not (buffer-modified-p))
                 (if enabled
                     (let ((text (jaunder-debug-boundary--text)))
                       (jaunder-debug-boundary--assert-tree
                        text '("publish.recover" "publish.recover" "publish.recover"
                               "publish.checkpoint" "publish.checkpoint" "publish.checkpoint"))
                       (should-not (string-match-p "private" text)))
                   (should (zerop jaunder--debug-id-counter))
                   (should-not (get-buffer jaunder--debug-buffer-name)))
                 (list first second changed intent-bytes slug checkpoint
                       (file-name-nondirectory destination) (file-name-nondirectory no-op)
                       (buffer-live-p buffer) (buffer-modified-p))))))
       (when (buffer-live-p buffer)
         (with-current-buffer buffer (set-buffer-modified-p nil)) (kill-buffer buffer))
       (delete-directory root t)))))

(ert-deftest jaunder-publish-debug-intent-write-back-rename-standalone-off-on ()
  "Each actual helper owns a root while retaining native returns and local bytes."
  (should (equal (jaunder-publish-debug--standalone-checkpoints nil)
                 (jaunder-publish-debug--standalone-checkpoints t))))

(ert-deftest jaunder-publish-debug-tree-assertion-rejects-false-topology ()
  "The proof rejects fabricated lifetimes, cycles and misordered terminals."
  (let* ((root-start "at=2026-10-07T00:00:00.000Z correlation=1 span=2 label=publish.post phase=start\n")
         (root-end "at=2026-10-07T00:00:00.000Z correlation=1 span=2 label=publish.post phase=end\n")
         (recover-start "at=2026-10-07T00:00:00.000Z correlation=1 span=3 label=publish.recover phase=start parent=2\n")
         (recover-end "at=2026-10-07T00:00:00.000Z correlation=1 span=3 label=publish.recover phase=end parent=2\n")
         (create-start "at=2026-10-07T00:00:00.000Z correlation=1 span=4 label=publish.create phase=start parent=3\n")
         (create-end "at=2026-10-07T00:00:00.000Z correlation=1 span=4 label=publish.create phase=end parent=3\n")
         (siblings '(("publish.recover" . "publish.post") ("publish.create" . "publish.post"))))
    ;; Pair counts and common correlation alone would accept this false nesting.
    (should-error (jaunder-debug-boundary--assert-tree
                   (concat root-start recover-start create-start create-end recover-end root-end)
                   '("publish.post") siblings) :type 'ert-test-failed)
    (dolist (text (list
                   (concat root-start recover-start root-end recover-end)
                   (concat root-start root-end recover-start recover-end)
                   (concat root-start (replace-regexp-in-string "parent=2" "parent=3" recover-start)
                           (replace-regexp-in-string "parent=2" "parent=3" recover-end) root-end)
                   (concat (replace-regexp-in-string "phase=start" "phase=start parent=3" root-start)
                           recover-start recover-end
                           (replace-regexp-in-string "phase=end" "phase=end parent=3" root-end))))
      (should-error (jaunder-debug-boundary--assert-tree text '("publish.post")) :type 'ert-test-failed))
    (should-error (jaunder-debug-boundary--assert-tree (concat root-start root-end) '("publish.draft"))
                  :type 'ert-test-failed)
    (should-error (jaunder-publish-debug--assert-intent-before-transfer
                   (concat root-start
                           (replace-regexp-in-string "parent=3" "parent=2" create-start)
                           (replace-regexp-in-string "parent=3" "parent=2" create-end)
                           recover-start recover-end root-end) 1)
                  :type 'ert-test-failed)))

(provide 'jaunder-publish-debug-test)
;;; jaunder-publish-debug-test.el ends here
