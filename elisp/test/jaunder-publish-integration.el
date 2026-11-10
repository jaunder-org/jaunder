;;; jaunder-publish-integration.el --- live publish tests -*- lexical-binding: t; -*-
;;; Commentary:
;; End-to-end publish flow against a real server (harness, ADR-0035).
;; Runs via `cargo xtask elisp-integration'.  Each post lives in its own tempdir
;; registered in `jaunder-blogs' (pointing at the harness's live server), so the
;; publish commands resolve the blog by directory the way real usage does.
;;; Code:

(require 'ert)
(require 'jaunder)
(require 'jaunder-integration-helper)

(defmacro jaunder-pub-test--in-buffer (contents &rest body)
  "Write CONTENTS to a temp .org file, visit it, run BODY, then clean up."
  (declare (indent 1) (debug t))
  `(let* ((dir (make-temp-file "jaunder-pub-" t))
          (path (expand-file-name "draft-20260101T000000.org" dir))
          (jaunder-blogs (list (cons (file-name-as-directory dir)
                                     (list :base-url jaunder-test-base-url
                                           :username jaunder-test-username))))
          (buf (progn (with-temp-file path (insert ,contents))
                      (find-file-noselect path))))
     (unwind-protect
         (with-current-buffer buf ,@body)
       (when (buffer-live-p buf) (with-current-buffer buf (set-buffer-modified-p nil)))
       (when (buffer-live-p buf) (kill-buffer buf))
       (delete-directory dir t))))

(ert-deftest jaunder-publish-creates-then-updates ()
  (jaunder-test--with-live-server
   (jaunder-pub-test--in-buffer
    "#+TITLE: Hello\n#+PROPERTY: JAUNDER_STATUS published\n\nFirst body.\n"
    (jaunder-publish)
    (should (string-match-p
             (concat "#\\+PROPERTY: JAUNDER_STATUS published\n"
                     "#\\+PROPERTY: JAUNDER_AUDIENCE private\n"
                     "#\\+PROPERTY: JAUNDER_DATE_TZ [^\n]+\n"
                     "#\\+PROPERTY: JAUNDER_DATE_UTC [^\n]+\n"
                     "#\\+PROPERTY: JAUNDER_SLUG [^\n]+\n"
                     "#\\+PROPERTY: JAUNDER_ID [^\n]+\n"
                     "#\\+PROPERTY: JAUNDER_SYNCED [^\n]+\n"
                     "#\\+PROPERTY: JAUNDER_SYNCED_AT [^\n]+\n")
             (buffer-string)))
    (let ((id (jaunder--buffer-property "JAUNDER_ID"))
          (slug (jaunder--buffer-property "JAUNDER_SLUG"))
          (synced (jaunder--buffer-property "JAUNDER_SYNCED")))
      (should id)
      (should slug)
      (should synced)
      (should (equal (jaunder--buffer-property "JAUNDER_AUDIENCE") "private"))
      (let ((post-path (buffer-file-name)))
        (should (string-match-p
                 "^#\\+PROPERTY: JAUNDER_AUDIENCE private$"
                 (with-temp-buffer
                   (insert-file-contents post-path)
                   (buffer-string)))))
      (let* ((member (jaunder--http-request "GET" (jaunder--member-url id)))
             (fields (jaunder--harvest-response-fields (plist-get member :body))))
        (should (equal (cdr (assq 'audiences fields)) '("private"))))
      (should (equal (file-name-nondirectory (buffer-file-name))
                     (concat slug ".org")))
      ;; A headerless update does not guess a scope: the confirmed existing
      ;; private audience is restored, and this remains the same Post.
      (jaunder--remove-property "JAUNDER_AUDIENCE")
      (goto-char (point-max)) (insert "More.\n") (save-buffer)
      (jaunder-publish)
      (should (equal (jaunder--buffer-property "JAUNDER_ID") id))
      (should (equal (jaunder--buffer-property "JAUNDER_AUDIENCE") "private"))
      (let* ((member (jaunder--http-request "GET" (jaunder--member-url id)))
             (fields (jaunder--harvest-response-fields (plist-get member :body))))
        (should (equal (cdr (assq 'audiences fields)) '("private"))))))))

(ert-deftest jaunder-publish-round-trips-explicit-audience-unions ()
  (jaunder-test--with-live-server
   (jaunder-pub-test--in-buffer
    (concat "#+TITLE: Audience\n"
            "#+PROPERTY: JAUNDER_STATUS published\n"
            "#+PROPERTY: JAUNDER_AUDIENCE private\n\nBody.\n")
    (jaunder-publish)
    (let* ((id (jaunder--buffer-property "JAUNDER_ID"))
           (member-url (jaunder--member-url id))
           (private (jaunder--http-request "GET" member-url))
           (private-fields
            (jaunder--harvest-response-fields (plist-get private :body))))
      (should (equal (cdr (assq 'audiences private-fields)) '("private")))
      (goto-char (point-min))
      (re-search-forward "JAUNDER_AUDIENCE private")
      (replace-match "JAUNDER_AUDIENCE subscribers")
      (end-of-line)
      (insert "\n#+PROPERTY: JAUNDER_AUDIENCE public")
      (save-buffer)
      (jaunder-publish)
      (let* ((union (jaunder--http-request "GET" member-url))
             (union-fields
              (jaunder--harvest-response-fields (plist-get union :body))))
        (should (equal (cdr (assq 'audiences union-fields))
                       '("public" "subscribers")))
        (should (string-match-p
                 (regexp-quote
                  (concat "#+PROPERTY: JAUNDER_AUDIENCE public\n"
                          "#+PROPERTY: JAUNDER_AUDIENCE subscribers\n"))
                 (buffer-string)))
        (should (= (how-many "^#\\+PROPERTY: JAUNDER_AUDIENCE"
                             (point-min) (jaunder--body-start)) 2)))))))

(ert-deftest jaunder-publish-uploads-pdf-and-sends-harvested-media-url ()
  (jaunder-test--with-live-server
   (jaunder-pub-test--in-buffer
    (concat "#+TITLE: Document\n"
            "#+PROPERTY: JAUNDER_STATUS published\n\n"
            "[[file:document.pdf][download]]\n")
    (let ((pdf (expand-file-name "document.pdf" dir)))
      (with-temp-file pdf
        (set-buffer-multibyte nil)
        (insert "PDF-BYTES"))
      (jaunder-publish)
      (let* ((id (jaunder--buffer-property "JAUNDER_ID"))
             (response
              (jaunder--http-request
               "GET"
               (jaunder--build-url
                jaunder-test-base-url "atompub" jaunder-test-username "posts" id)))
             (xml (plist-get response :body))
             (media-prefix
              (jaunder--build-url jaunder-test-base-url "media" "upload")))
        (should id)
        (should (eq (plist-get response :status) 200))
        (should (string-match-p "download" xml))
        (should (string-match-p (regexp-quote media-prefix) xml))
        (should-not (string-match-p (regexp-quote "file:document.pdf") xml))
        (should-not (string-match-p (regexp-quote pdf) xml))
        (should
         (string-match-p
          (regexp-quote "[[file:document.pdf][download]]")
          (buffer-string))))))))

(ert-deftest jaunder-publish-stale-if-match-surfaces-412 ()
  (jaunder-test--with-live-server
   (jaunder-pub-test--in-buffer
    "#+TITLE: T\n#+PROPERTY: JAUNDER_STATUS published\n\nBody.\n"
    (jaunder-publish)
    ;; Corrupt the stored ETag → the next PUT must 412 and leave the file intact.
    (jaunder--set-property "JAUNDER_SYNCED" "\"stale\"") (save-buffer)
    (let ((before (buffer-string)))
      (should (equal (jaunder--buffer-property "JAUNDER_AUDIENCE") "private"))
      (should-error (jaunder-publish))
      (should (equal (buffer-string) before))))))

(ert-deftest jaunder-publish-untitled-note ()
  (jaunder-test--with-live-server
   (jaunder-pub-test--in-buffer
    "#+PROPERTY: JAUNDER_STATUS published\n\n🎉✨\n"
    (jaunder-publish)
    (should (jaunder--buffer-property "JAUNDER_SLUG")))))

(ert-deftest jaunder-publish-scheduled-future ()
  "A scheduled post with a future #+DATE: is accepted and gets an id."
  (jaunder-test--with-live-server
   (jaunder-pub-test--in-buffer
    (concat "#+TITLE: Later\n"
            "#+DATE: [2999-01-01 Tue 00:00]\n"
            "#+PROPERTY: JAUNDER_STATUS scheduled\n\nFuture body.\n")
    (jaunder-publish)
    (should (jaunder--buffer-property "JAUNDER_ID"))
    (should (jaunder--buffer-property "JAUNDER_DATE_UTC")))))

(ert-deftest jaunder-publish-rejects-empty-body ()
  (jaunder-test--with-live-server
   (jaunder-pub-test--in-buffer
    "#+TITLE: T\n#+PROPERTY: JAUNDER_STATUS published\n\n\n"
    (let ((before (buffer-string)))
      (should-error (jaunder-publish))
      (should (null (jaunder--buffer-property "JAUNDER_ID")))
      (should (equal (buffer-string) before))))))

(ert-deftest jaunder-save-draft-pushes-server-side-draft ()
  (jaunder-test--with-live-server
   (jaunder-pub-test--in-buffer
    "#+TITLE: D\n#+DATE: [2026-07-01 Wed 09:00]\n#+PROPERTY: JAUNDER_STATUS published\n\nDraft body.\n"
    ;; Force-draft even though status=published; must succeed and get an id.
    (jaunder-save-draft)
    (let* ((id (jaunder--buffer-property "JAUNDER_ID"))
           (path (buffer-file-name))
           (member (jaunder--http-request "GET" (jaunder--member-url id)))
           (fields (jaunder--harvest-response-fields (plist-get member :body))))
      (should id)
      (should (equal (jaunder--buffer-property "JAUNDER_AUDIENCE") "private"))
      (should (string-match-p
               "^#\\+PROPERTY: JAUNDER_AUDIENCE private$"
               (with-temp-buffer (insert-file-contents path) (buffer-string))))
      (should (equal (cdr (assq 'audiences fields)) '("private")))))))

(defun jaunder-pub-test--web-zone-edit (id zone utc)
  "Edit harness Post ID through the browser API in ZONE, retaining UTC."
  (let* ((login (jaunder--plz-response->plist
                 (plz 'post (concat jaunder-test-base-url "/api/auth/login")
                      :headers (list '("Content-Type" . "application/x-www-form-urlencoded")
                                     (cons "Origin" jaunder-test-base-url))
                      :body (concat "username=" jaunder-test-username "&password=password123")
                      :as 'response)))
         (cookie (car (split-string (or (jaunder--response-header login "Set-Cookie") "") ";"))))
    (should (eq (plist-get login :status) 200))
    (should (string-prefix-p "session=" cookie))
    (let ((edited
           (plz 'post (concat jaunder-test-base-url "/api/posts/update")
                :headers (list '("Content-Type" . "application/json")
                               (cons "Origin" jaunder-test-base-url) (cons "Cookie" cookie))
                :body (json-encode
                       `((post_id . ,(string-to-number id))
                         (post . ((body . "London web edit.") (format . "org")
                                  (publish . t) (publish_at . ,utc) (timezone . ,zone)))))
                :as 'response)))
      (should (eq (plz-response-status edited) 200)))))

(ert-deftest jaunder-publication-timezone-live-org-web-reconcile-round-trip ()
  "NY authoring -> London web edit -> server-ahead pull preserves the instant."
  (jaunder-test--with-live-server
   (let ((process-environment (copy-sequence process-environment)))
     (jaunder-pub-test--in-buffer
      (concat "#+TITLE: Timezone round trip\n#+DATE: [2024-07-01 Mon 09:00]\n"
              "#+PROPERTY: JAUNDER_STATUS published\n\nNY Org source.\n")
      (setenv "TZ" "America/New_York")
      (jaunder-publish)
      (let ((id (jaunder--buffer-property "JAUNDER_ID"))
            (utc (jaunder--buffer-property "JAUNDER_DATE_UTC"))
            (local (current-buffer)))
        (should (equal utc "2024-07-01T13:00:00Z"))
        (should (equal (jaunder--buffer-property "JAUNDER_DATE_TZ") "America/New_York"))
        (jaunder-pub-test--web-zone-edit id "Europe/London" utc)
        ;; Pull runs on the NY device.  It must adopt London's server pair,
        ;; never recapture NY as though reading were an authored edit.
        (save-window-excursion
          (jaunder-reconcile dir)
          (with-current-buffer "*Jaunder Reconcile*"
            (let ((row (cl-find id (jaunder-reconcile-report-rows jaunder-reconcile-report)
                                :key #'jaunder--reconcile-row-post-id :test #'equal)))
              (should (eq (jaunder-reconcile-row-state row) 'server-ahead))
              (puthash (concat "post:" id) t jaunder-reconcile-marks))
            (cl-letf (((symbol-function 'y-or-n-p) (lambda (&rest _) t)))
              (jaunder-reconcile-pull-selected))
            (should (eq (jaunder-reconcile-result-outcome
                         (car jaunder-reconcile-last-batch-results)) 'success))))
        (with-current-buffer local
          (should (equal (jaunder--buffer-property "JAUNDER_DATE_TZ") "Europe/London"))
          (should (equal (jaunder--buffer-keyword "DATE") "[2024-07-01 Mon 14:00]"))
          (should (equal (jaunder--buffer-property "JAUNDER_DATE_UTC") utc))
          (should (string-match-p "London web edit" (buffer-string)))
          ;; A new authored NY PUT legitimately changes the saved zone back,
          ;; but cannot reinterpret London's unchanged wall DATE as NY time.
          (jaunder-publish)
          (should (equal (jaunder--buffer-property "JAUNDER_DATE_TZ") "America/New_York"))
          (should (equal (jaunder--buffer-keyword "DATE") "[2024-07-01 Mon 09:00]"))
          (let ((fields (jaunder--harvest-response-fields
                         (plist-get (jaunder--http-request "GET" (jaunder--member-url id)) :body))))
            (should (member (cdr (assq 'published fields))
                            '("2024-07-01T13:00:00Z" "2024-07-01T13:00:00+00:00")))
            (should (equal (jaunder--buffer-property "JAUNDER_DATE_UTC") utc))
            (should (equal (cdr (assq 'timezones fields)) '("America/New_York"))))))))))

(ert-deftest jaunder-publication-timezone-live-scheduled-later-fold-retains-precision ()
  (jaunder-test--with-live-server
   (let ((process-environment (copy-sequence process-environment)))
     (jaunder-pub-test--in-buffer "#+PROPERTY: JAUNDER_STATUS draft\n\nBody.\n"
                                  (setenv "TZ" "America/New_York")
                                  (jaunder-publish)
                                  (let* ((id (jaunder--buffer-property "JAUNDER_ID"))
                                         (url (jaunder--member-url id))
                                         (utc "2030-11-03T06:30:17.987654321Z")
                                         (response (jaunder--http-request
                                                    "PUT" url
                                                    (jaunder--atom-entry->xml
                                                     (jaunder--make-entry :body "Scheduled body." :content-type "text/org"
                                                                          :timezone "America/New_York" :published utc))
                                                    jaunder--entry-content-type
                                                    (list (cons "If-Match" (jaunder--buffer-property "JAUNDER_SYNCED")))))
                                         (org (jaunder--atom->org (plist-get response :body)
                                                                  (jaunder--response-header response "ETag")
                                                                  (current-time) "UTC" t)))
                                    (should (eq (plist-get response :status) 200))
                                    (erase-buffer) (insert org) (save-buffer)
                                    (should (equal (jaunder--buffer-keyword "DATE") "[2030-11-03 Sun 01:30]"))
                                    (setenv "TZ" "Europe/London")
                                    (jaunder-publish)
                                    (should (equal (jaunder--buffer-property "JAUNDER_DATE_UTC") utc))
                                    (should (equal (jaunder--buffer-keyword "DATE") "[2030-11-03 Sun 06:30]"))
                                    (should (equal (jaunder--buffer-property "JAUNDER_DATE_TZ") "Europe/London"))
                                    (let ((before (buffer-string)))
                                      (jaunder--set-property "JAUNDER_SYNCED" "\"stale\"") (save-buffer)
                                      (setq before (buffer-string))
                                      (setenv "TZ" "America/New_York")
                                      (should-error (jaunder-publish))
                                      (should (equal before (buffer-string)))))))))

(ert-deftest jaunder-publication-timezone-live-zoneinfo-only-capture-uses-utc ()
  (jaunder-test--with-live-server
   (let ((process-environment (copy-sequence process-environment)) warnings)
     (jaunder-pub-test--in-buffer "#+PROPERTY: JAUNDER_STATUS draft\n\nUTC fallback body.\n"
                                  (let* ((source (or (getenv "TZDIR") "/usr/share/zoneinfo"))
                                         (inventory-root (expand-file-name "zoneinfo" dir))
                                         (zone "posix/America/New_York"))
                                    (make-directory (expand-file-name "posix/America" inventory-root) t)
                                    (copy-file (expand-file-name "America/New_York" source) (expand-file-name zone inventory-root))
                                    (copy-file (expand-file-name "UTC" source) (expand-file-name "UTC" inventory-root))
                                    (with-temp-file (expand-file-name "tzdata.zi" inventory-root)
                                      (insert "# version fixture\nZ UTC 0 - UTC\n"))
                                    (setenv "TZDIR" inventory-root)
                                    (setenv "TZ" zone)
                                    (should (jaunder--local-baseline-zone-p zone))
                                    (cl-letf (((symbol-function 'display-warning)
                                               (lambda (type text &optional level &rest _)
                                                 (push (list type text level) warnings))))
                                      (jaunder-publish))
                                    (should (equal (jaunder--buffer-property "JAUNDER_DATE_TZ") "UTC"))
                                    (should (= (length warnings) 1))
                                    (should (string-match-p "using UTC" (cadar warnings)))
                                    (let ((fields (jaunder--harvest-response-fields
                                                   (plist-get (jaunder--http-request
                                                               "GET" (jaunder--member-url (jaunder--buffer-property "JAUNDER_ID"))) :body))))
                                      (should (equal (cdr (assq 'timezones fields)) '("UTC")))))))))

(ert-deftest jaunder-publication-timezone-live-tz-unset-uses-actual-device-offset ()
  "With TZ absent and no discoverable name, store the actual device offset."
  (jaunder-test--with-live-server
   (let ((process-environment (copy-sequence process-environment)))
     (jaunder-pub-test--in-buffer "#+PROPERTY: JAUNDER_STATUS draft\n\nActual device offset.\n"
                                  (setenv "TZ" nil)
                                  (should-not (getenv "TZ"))
                                  ;; Independent literal mapping, not the production conversion algorithm.
                                  (let* ((east (car (current-time-zone)))
                                         (expected (or (alist-get east
                                                                  '((-43200 . "Etc/GMT+12") (-39600 . "Etc/GMT+11")
                                                                    (-36000 . "Etc/GMT+10") (-32400 . "Etc/GMT+9")
                                                                    (-28800 . "Etc/GMT+8") (-25200 . "Etc/GMT+7")
                                                                    (-21600 . "Etc/GMT+6") (-18000 . "Etc/GMT+5")
                                                                    (-14400 . "Etc/GMT+4") (-10800 . "Etc/GMT+3")
                                                                    (-7200 . "Etc/GMT+2") (-3600 . "Etc/GMT+1")
                                                                    (0 . "Etc/GMT") (3600 . "Etc/GMT-1")
                                                                    (7200 . "Etc/GMT-2") (10800 . "Etc/GMT-3")
                                                                    (14400 . "Etc/GMT-4") (18000 . "Etc/GMT-5")
                                                                    (21600 . "Etc/GMT-6") (25200 . "Etc/GMT-7")
                                                                    (28800 . "Etc/GMT-8") (32400 . "Etc/GMT-9")
                                                                    (36000 . "Etc/GMT-10") (39600 . "Etc/GMT-11")
                                                                    (43200 . "Etc/GMT-12"))) "UTC"))
                                         warnings)
                                    ;; Only named discovery is unavailable; current-time-zone is untouched.
                                    (cl-letf (((symbol-function 'file-symlink-p) (lambda (_) nil))
                                              ((symbol-function 'display-warning)
                                               (lambda (_ text &rest _) (push text warnings))))
                                      (jaunder-publish))
                                    (should (equal (jaunder--buffer-property "JAUNDER_DATE_TZ") expected))
                                    (should (string-match-p (if (equal expected "UTC") "using UTC" "inferred fixed offset")
                                                            (car warnings)))
                                    (let ((fields (jaunder--harvest-response-fields
                                                   (plist-get (jaunder--http-request
                                                               "GET" (jaunder--member-url (jaunder--buffer-property "JAUNDER_ID"))) :body))))
                                      (should (equal (cdr (assq 'timezones fields)) (list expected)))))))))

(ert-deftest jaunder-publication-timezone-live-tz-unset-stores-device-offset-and-preserves-instant ()
  "An unnamed device publishes and changes zone without moving its instant."
  (jaunder-test--with-live-server
   (let ((process-environment (copy-sequence process-environment)) warnings)
     (jaunder-pub-test--in-buffer
      (concat "#+TITLE: Unnamed device\n#+DATE: [2024-07-01 Mon 09:00]\n"
              "#+PROPERTY: JAUNDER_STATUS published\n\nOffset source.\n")
      (setenv "TZ" nil)
      (should-not (getenv "TZ"))
      ;; Deterministic device boundary, not a stored-zone override: TZ is truly
      ;; absent and no localtime name can be recovered. Exercise real POST/PUT/GET.
      (cl-letf (((symbol-function 'file-symlink-p) (lambda (_) nil))
                ((symbol-function 'current-time-zone) (lambda (&rest _) '(-18000 "unknown")))
                ((symbol-function 'display-warning)
                 (lambda (_ text &rest _) (push text warnings))))
        (jaunder-publish))
      (should (equal (jaunder--buffer-property "JAUNDER_DATE_TZ") "Etc/GMT+5"))
      (should (equal (jaunder--buffer-property "JAUNDER_DATE_UTC") "2024-07-01T14:00:00Z"))
      (should (string-match-p (regexp-quote "inferred fixed offset Etc/GMT+5") (car warnings)))
      (cl-letf (((symbol-function 'file-symlink-p) (lambda (_) nil))
                ((symbol-function 'current-time-zone) (lambda (&rest _) '(3600 "unknown")))
                ((symbol-function 'display-warning) (lambda (&rest _))))
        (jaunder-publish))
      (should (equal (jaunder--buffer-property "JAUNDER_DATE_TZ") "Etc/GMT-1"))
      (should (equal (jaunder--buffer-keyword "DATE") "[2024-07-01 Mon 15:00]"))
      (should (equal (jaunder--buffer-property "JAUNDER_DATE_UTC") "2024-07-01T14:00:00Z"))
      (let ((fields (jaunder--harvest-response-fields
                     (plist-get (jaunder--http-request
                                 "GET" (jaunder--member-url (jaunder--buffer-property "JAUNDER_ID"))) :body))))
        (should (equal (cdr (assq 'timezones fields)) '("Etc/GMT-1")))
        (should (member (cdr (assq 'published fields))
                        '("2024-07-01T14:00:00Z" "2024-07-01T14:00:00+00:00"))))))))

(provide 'jaunder-publish-integration)
;;; jaunder-publish-integration.el ends here
