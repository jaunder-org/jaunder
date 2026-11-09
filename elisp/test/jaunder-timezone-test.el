;;; jaunder-timezone-test.el --- Publication timezone workflows -*- lexical-binding: t; -*-

;;; Commentary:
;; Exercise local authoring and synchronization against the HTTP boundary.

;;; Code:

(require 'ert)
(require 'jaunder)
(require 'jaunder-reconcile)
(load (expand-file-name "jaunder-reconcile-write-fixture.el"
                        (file-name-directory (or load-file-name buffer-file-name))) nil t)
(load (expand-file-name "jaunder-reconcile-merge-operation-fixture.el"
                        (file-name-directory (or load-file-name buffer-file-name))) nil t)

(defconst jaunder-timezone-test--service
  (concat "<service xmlns=\"http://www.w3.org/2007/app\""
          " xmlns:a=\"http://www.w3.org/2005/Atom\""
          " xmlns:j=\"https://jaunder.org/ns/atompub\"><workspace>"
          "<a:title>Blog</a:title><j:extension version=\"1\""
          " features=\"format-media-type audience timezone\"/>"
          "</workspace></service>"))

(defun jaunder-timezone-test--member (zone utc &optional draft)
  "Return a confirmed Member in ZONE at UTC, or DRAFT."
  (concat "<entry xmlns=\"http://www.w3.org/2005/Atom\""
          " xmlns:j=\"https://jaunder.org/ns/atompub\""
          " xmlns:app=\"http://www.w3.org/2007/app\">"
          "<title>Post</title><content type=\"text/org\">Body</content>"
          "<j:slug>post</j:slug><j:audience>public</j:audience>"
          "<j:timezone>" zone "</j:timezone>"
          "<link rel=\"edit\" href=\"https://blog/atompub/alice/posts/7\"/>"
          (if draft "<app:control><app:draft>yes</app:draft></app:control>"
            (concat "<published>" utc "</published>")) "</entry>"))

(defmacro jaunder-timezone-test--with-post (source &rest body)
  "Run BODY in a visiting Org Post containing SOURCE, cleaning up afterward."
  (declare (indent 1))
  `(let* ((root (file-name-as-directory (make-temp-file "jaunder-timezone-" t)))
          (path (expand-file-name "post.org" root))
          (jaunder-blogs (list (cons root '(:base-url "https://blog" :username "alice"))))
          (jaunder-warn-missing-format-media-type nil)
          (jaunder-warn-untracked-media nil)
          (jaunder-warn-zone-mismatch nil)
          (process-environment (copy-sequence process-environment))
          buffer)
     (unwind-protect
         (progn
           (with-temp-file path (insert ,source))
           (setq buffer (find-file-noselect path))
           (with-current-buffer buffer (org-mode) ,@body))
       (when (buffer-live-p buffer)
         (with-current-buffer buffer (set-buffer-modified-p nil))
         (kill-buffer buffer))
       (delete-directory root t))))

(ert-deftest jaunder-timezone-publish-preserves-exact-instant-and-writes-confirmed-pair ()
  (jaunder-timezone-test--with-post
   (concat "#+DATE: [2026-07-01 Wed 09:00]\n"
           "#+PROPERTY: JAUNDER_DATE_TZ America/New_York\n"
           "#+PROPERTY: JAUNDER_DATE_UTC 2026-07-01T13:00:42.123456789Z\n"
           "#+PROPERTY: JAUNDER_STATUS published\n"
           "#+PROPERTY: JAUNDER_ID 7\n#+PROPERTY: JAUNDER_SYNCED \"old\"\n\nBody\n")
   (setenv "TZ" "Europe/London")
   (let ((before (buffer-string)) sent warnings)
     (cl-letf (((symbol-function 'display-warning)
                (lambda (&rest args) (push args warnings)))
               ((symbol-function 'jaunder--http-request)
                (lambda (method _url &optional xml _type headers)
                  (if (equal method "GET")
                      (list :status 200 :body jaunder-timezone-test--service)
                    (should (equal method "PUT"))
                    (should (equal (cdr (assoc "If-Match" headers)) "\"old\""))
                    (should (equal before (buffer-string)))
                    (setq sent (jaunder--harvest-response-fields xml))
                    (list :status 200 :headers '(("etag" . "\"new\""))
                          :body (jaunder-timezone-test--member
                                 "Europe/London" "2026-07-01T13:00:42.123456789Z"))))))
       (jaunder-publish))
     (should-not warnings)
     (should (equal (cdr (assq 'published sent)) "2026-07-01T13:00:42.123456789Z"))
     (should (equal (cdr (assq 'timezones sent)) '("Europe/London")))
     (should (equal (jaunder--buffer-property "JAUNDER_DATE_TZ") "Europe/London"))
     (should (equal (jaunder--buffer-keyword "DATE") "[2026-07-01 Wed 14:00]"))
     (should (equal (jaunder--buffer-property "JAUNDER_DATE_UTC")
                    "2026-07-01T13:00:42.123456789Z")))))

(ert-deftest jaunder-timezone-pull-adopts-server-pair-not-supplied-device-zone ()
  (let ((org (jaunder--atom->org
              (jaunder-timezone-test--member "Europe/London" "2026-07-01T13:00:42.123456789Z")
              "\"remote\"" (date-to-time "2026-07-02T00:00:00Z") "America/New_York" t)))
    (should (string-match-p (regexp-quote "#+DATE: [2026-07-01 Wed 14:00]") org))
    (should (string-match-p "JAUNDER_DATE_TZ Europe/London" org))
    (should (string-match-p "JAUNDER_DATE_UTC 2026-07-01T13:00:42.123456789Z" org))))

(defun jaunder-timezone-test--receipt (xml)
  "Echo authored XML's timezone and instant as a confirmed server receipt."
  (let ((fields (jaunder--harvest-response-fields xml)))
    (list :status 200 :headers '(("etag" . "\"new\"")
                                 ("location" . "https://blog/atompub/alice/posts/7"))
          :body (jaunder-timezone-test--member
                 (car (cdr (assq 'timezones fields)))
                 (or (cdr (assq 'published fields)) "2026-07-01T13:00:00Z")
                 (equal (cdr (assq 'drafts fields)) '("yes"))))))

(ert-deftest jaunder-timezone-publish-local-drift-after-put-preserves-pair ()
  (jaunder-timezone-test--with-post
   (concat "#+DATE: [2026-07-01 Wed 09:00]\n"
           "#+PROPERTY: JAUNDER_DATE_TZ America/New_York\n"
           "#+PROPERTY: JAUNDER_DATE_UTC 2026-07-01T13:00:42.123Z\n"
           "#+PROPERTY: JAUNDER_ID 7\n#+PROPERTY: JAUNDER_SYNCED \"old\"\n\nBody\n")
   (setenv "TZ" "Europe/London")
   (cl-letf (((symbol-function 'jaunder--http-request)
              (lambda (method _url &optional xml &rest _)
                (if (equal method "GET")
                    (list :status 200 :body jaunder-timezone-test--service)
                  (goto-char (point-max)) (insert "Unsent concurrent edit\n")
                  (jaunder-timezone-test--receipt xml)))))
     (should-error (jaunder-publish)))
   (should (string-match-p "Unsent concurrent edit" (buffer-string)))
   (should (equal (jaunder--buffer-property "JAUNDER_DATE_TZ") "America/New_York"))
   (should (equal (jaunder--buffer-property "JAUNDER_SYNCED") "\"old\""))
   (should (equal (jaunder--buffer-keyword "DATE") "[2026-07-01 Wed 09:00]"))))

(ert-deftest jaunder-timezone-authored-write-requires-advertised-feature ()
  (jaunder-timezone-test--with-post "#+PROPERTY: JAUNDER_STATUS draft\n\nBody\n"
                                    (setenv "TZ" "Europe/London")
                                    (let ((writes 0))
                                      (cl-letf (((symbol-function 'jaunder--http-request)
                                                 (lambda (method _url &optional xml &rest _)
                                                   (if (equal method "GET")
                                                       (list :status 200 :body
                                                             (replace-regexp-in-string " timezone" "" jaunder-timezone-test--service))
                                                     (cl-incf writes) (jaunder-timezone-test--receipt xml)))))
                                        (should-error (jaunder-publish)))
                                      (should (= writes 0)))))

(ert-deftest jaunder-timezone-capture-failure-sends-explicit-utc-and-visible-warning ()
  (dolist (capture '(nil "" "+0530" "America/New_Yrok"))
    (jaunder-timezone-test--with-post "#+PROPERTY: JAUNDER_STATUS draft\n\nBody\n"
                                      (setenv "TZ" capture)
                                      (let (sent warnings)
                                        (cl-letf (((symbol-function 'file-symlink-p) (lambda (_) nil))
                                                  ((symbol-function 'display-warning)
                                                   (lambda (type text &optional level &rest _)
                                                     (push (list type text level) warnings)))
                                                  ((symbol-function 'jaunder--http-request)
                                                   (lambda (method _url &optional xml &rest _)
                                                     (if (equal method "GET")
                                                         (list :status 200 :body jaunder-timezone-test--service)
                                                       (setq sent (jaunder--harvest-response-fields xml))
                                                       (jaunder-timezone-test--receipt xml)))))
                                          (jaunder-publish))
                                        (should (equal (cdr (assq 'timezones sent)) '("UTC")))
                                        (should (equal (jaunder--buffer-property "JAUNDER_DATE_TZ") "UTC"))
                                        (should (= (length warnings) 1))
                                        (should (eq (caar warnings) 'jaunder))
                                        (should (eq (nth 2 (car warnings)) :warning))
                                        (should (string-match-p "using UTC" (cadar warnings)))))))

(ert-deftest jaunder-timezone-authored-dates-use-latest-zone-and-retain-fold-identity ()
  (dolist (case '((nil "[2026-07-01 Wed 09:00]" "America/New_York" nil "Europe/London"
                       "2026-07-01T08:00:00Z")
                  (t "[2026-07-01 Wed 10:15]" "America/New_York"
                     "2026-07-01T13:00:42.123Z" "Europe/London" "2026-07-01T09:15:00Z")
                  (t "[2030-11-03 Sun 01:30]" "America/New_York"
                     "2030-11-03T06:30:17.987654321Z" "Europe/London" "2030-11-03T06:30:17.987654321Z")
                  (t "[2030-11-03 Sun 01:30]" "Europe/London"
                     "2030-11-02T12:00:00Z" "America/New_York" "2030-11-03T05:30:00Z")
                  (nil "[2026-07-01 Wed 09:00]" "UTC" nil "US/Eastern" "2026-07-01T13:00:00Z")
                  (t "[2026-07-01 Wed 18:30]" "+0530"
                     "2026-07-01T13:00:42.123Z" "Europe/London" "2026-07-01T13:00:42.123Z")))
    (pcase-let ((`(,existing ,date ,old-zone ,marker ,zone ,expected) case))
      (jaunder-timezone-test--with-post
       (concat "#+DATE: " date "\n#+PROPERTY: JAUNDER_DATE_TZ " old-zone "\n"
               (when marker (concat "#+PROPERTY: JAUNDER_DATE_UTC " marker "\n"))
               (when existing "#+PROPERTY: JAUNDER_ID 7\n#+PROPERTY: JAUNDER_SYNCED \"old\"\n")
               "\nBody\n")
       (setenv "TZ" zone)
       (let (sent)
         (cl-letf (((symbol-function 'jaunder--http-request)
                    (lambda (method _url &optional xml &rest _)
                      (if (equal method "GET")
                          (list :status 200 :body jaunder-timezone-test--service)
                        (setq sent (jaunder--harvest-response-fields xml))
                        (jaunder-timezone-test--receipt xml)))))
           (jaunder-publish))
         (should (equal (cdr (assq 'published sent)) expected))
         (should (equal (jaunder--buffer-property "JAUNDER_DATE_UTC") expected))
         (should (equal (jaunder--buffer-property "JAUNDER_DATE_TZ") zone)))))))

(ert-deftest jaunder-timezone-invalid-baseline-and-gap-stop-before-post-write ()
  (dolist (marker '(nil "invalid" "2026-07-01T13:00:42.123Z"))
    (jaunder-timezone-test--with-post
     (concat "#+DATE: [2030-03-10 Sun 02:30]\n"
             "#+PROPERTY: JAUNDER_DATE_TZ Europe/London\n"
             (when marker (concat "#+PROPERTY: JAUNDER_DATE_UTC " marker "\n"))
             "#+PROPERTY: JAUNDER_ID 7\n#+PROPERTY: JAUNDER_SYNCED \"old\"\n\nBody\n")
     (setenv "TZ" "America/New_York")
     (let ((before (buffer-string)) (writes 0))
       (cl-letf (((symbol-function 'jaunder--http-request)
                  (lambda (method &rest _)
                    (unless (equal method "GET") (cl-incf writes))
                    (list :status 200 :body (jaunder-timezone-test--member
                                             "Europe/London" "2026-07-01T13:00:42.123Z")))))
         (let ((err (should-error (jaunder-publish))))
           (when (not (equal marker "2026-07-01T13:00:42.123Z"))
             (should (string-match-p "pull/reconcile" (error-message-string err))))))
       (should (zerop writes))
       (should (equal before (buffer-string)))))))

(ert-deftest jaunder-timezone-rejected-lost-and-malformed-receipts-preserve-pair ()
  (dolist (failure '(412 503 lost omitted))
    (jaunder-timezone-test--with-post
     (concat "#+DATE: [2026-07-01 Wed 09:00]\n"
             "#+PROPERTY: JAUNDER_DATE_TZ America/New_York\n"
             "#+PROPERTY: JAUNDER_DATE_UTC 2026-07-01T13:00:42.123Z\n"
             "#+PROPERTY: JAUNDER_ID 7\n#+PROPERTY: JAUNDER_SYNCED \"old\"\n\nBody\n")
     (setenv "TZ" "Europe/London")
     (let ((before (buffer-string)))
       (cl-letf (((symbol-function 'jaunder--http-request)
                  (lambda (method _url &optional _xml &rest _)
                    (if (equal method "GET")
                        (list :status 200 :body jaunder-timezone-test--service)
                      (pcase failure
                        ('lost (error "lost PUT response"))
                        ('omitted '(:status 200 :headers (("etag" . "\"new\""))
                                            :body "<entry xmlns=\"http://www.w3.org/2005/Atom\"/>"))
                        (_ (list :status failure)))))))
         (should-error (jaunder-publish)))
       (should (equal before (buffer-string)))
       (should (equal before (with-temp-buffer (insert-file-contents path) (buffer-string))))))))

(ert-deftest jaunder-timezone-pulled-draft-keeps-zone-without-publication-marker ()
  (let ((org (jaunder--atom->org (jaunder-timezone-test--member "Europe/London" nil t)
                                 "\"remote\"" (current-time) "America/New_York" t)))
    (should (string-match-p "JAUNDER_DATE_TZ Europe/London" org))
    (should-not (string-match-p "JAUNDER_DATE_UTC" org))
    (should-not (string-match-p (regexp-quote "#+DATE:") org))))

(ert-deftest jaunder-timezone-existing-draft-first-publication-needs-no-marker ()
  (jaunder-timezone-test--with-post
   (concat "#+DATE: [2026-07-01 Wed 09:00]\n#+PROPERTY: JAUNDER_STATUS published\n"
           "#+PROPERTY: JAUNDER_ID 7\n#+PROPERTY: JAUNDER_SYNCED \"old\"\n\nBody\n")
   (setenv "TZ" "Europe/London")
   (let ((reads 0) sent)
     (cl-letf (((symbol-function 'jaunder--http-request)
                (lambda (method url &optional xml &rest _)
                  (cond
                   ((string-suffix-p "/service" url)
                    (list :status 200 :body jaunder-timezone-test--service))
                   ((equal method "GET")
                    (cl-incf reads)
                    (list :status 200 :body (jaunder-timezone-test--member "UTC" nil t)))
                   (t (setq sent (jaunder--harvest-response-fields xml))
                      (jaunder-timezone-test--receipt xml))))))
       (jaunder-publish))
     (should (= reads 1))
     (should (equal (cdr (assq 'published sent)) "2026-07-01T08:00:00Z"))
     (should (equal (jaunder--buffer-property "JAUNDER_DATE_UTC") "2026-07-01T08:00:00Z")))))

(ert-deftest jaunder-timezone-directory-authored-put-checkpoints-captured-zone ()
  "Both ordinary push and reviewed keep-local retain the zone sent by that PUT."
  (dolist (action '(push keep-local))
    (let* ((utc "2026-07-01T13:00:42.123456789Z")
           sent
           (proof
            (jaunder-test--confirmed-write-batch
             action
             (lambda (event state)
               (pcase (plist-get event :phase)
                 ('setup
                  (setenv "TZ" "Europe/London")
                  (with-temp-buffer
                    (insert-file-contents (gethash "1" (plist-get state :paths)))
                    (org-mode)
                    (jaunder--set-property "JAUNDER_STATUS" "published")
                    (jaunder--set-property "JAUNDER_DATE_TZ" "America/New_York")
                    (jaunder--set-property "JAUNDER_DATE_UTC" utc)
                    (jaunder--set-keyword "DATE" "[2026-07-01 Wed 09:00]")
                    (write-region (point-min) (point-max) (gethash "1" (plist-get state :paths)) nil 'silent)))
                 ('write
                  (setq sent (jaunder--harvest-response-fields (plist-get event :xml)))
                  ;; A device-zone change while waiting for the response is not
                  ;; authority to checkpoint a zone that was never sent.
                  (setenv "TZ" "Asia/Tokyo")
                  (list :body
                        (replace-regexp-in-string
                         "<app:control><app:draft>yes</app:draft></app:control>"
                         (concat "<published>" utc "</published>")
                         (replace-regexp-in-string
                          "<j:timezone>UTC</j:timezone>" "<j:timezone>Europe/London</j:timezone>"
                          (jaunder-test--write-entry "1" "post-001" "https://example.test/~alice/post-001") t t)
                         t t))))) '(1))))
      (should (eq (jaunder-reconcile-result-outcome (car (plist-get proof :results))) 'success))
      (should (equal (cdr (assq 'timezones sent)) '("Europe/London")))
      (should (equal (cdr (assq 'published sent)) utc))
      (let ((installed (cdr (assoc "post-001.org" (plist-get proof :files)))))
        (should (string-match-p "JAUNDER_DATE_TZ Europe/London" installed))
        (should (string-match-p (regexp-quote "#+DATE: [2026-07-01 Wed 14:00]") installed))
        (should (string-match-p (concat "JAUNDER_DATE_UTC " (regexp-quote utc)) installed))))))

(ert-deftest jaunder-timezone-publish-disk-drift-after-put-preserves-local-evidence ()
  (jaunder-timezone-test--with-post
   (concat "#+DATE: [2026-07-01 Wed 09:00]\n"
           "#+PROPERTY: JAUNDER_DATE_TZ America/New_York\n"
           "#+PROPERTY: JAUNDER_DATE_UTC 2026-07-01T13:00:42.123Z\n"
           "#+PROPERTY: JAUNDER_ID 7\n#+PROPERTY: JAUNDER_SYNCED \"old\"\n\nBody\n")
   (setenv "TZ" "Europe/London")
   (let ((before (buffer-string)))
     (cl-letf (((symbol-function 'jaunder--http-request)
                (lambda (method _url &optional xml &rest _)
                  (if (equal method "GET")
                      (list :status 200 :body jaunder-timezone-test--service)
                    (with-temp-file path (insert "External replacement\n"))
                    (jaunder-timezone-test--receipt xml)))))
       (should-error (jaunder-publish)))
     (should (equal before (buffer-string)))
     (should (equal (with-temp-buffer (insert-file-contents path) (buffer-string))
                    "External replacement\n")))))

(defun jaunder-timezone-test--reject-publication-receipt (publication)
  "Require malformed PUBLICATION XML to leave the local Post byte-identical."
  (jaunder-timezone-test--with-post
   (concat "#+DATE: [2026-07-01 Wed 09:00]\n"
           "#+PROPERTY: JAUNDER_DATE_TZ America/New_York\n"
           "#+PROPERTY: JAUNDER_DATE_UTC 2026-07-01T13:00:42.123456789Z\n"
           "#+PROPERTY: JAUNDER_ID 7\n#+PROPERTY: JAUNDER_SYNCED \"old\"\n\nBody\n")
   (setenv "TZ" "Europe/London")
   (let ((before (buffer-string)) (writes 0))
     (cl-letf (((symbol-function 'jaunder--http-request)
                (lambda (method _url &optional _xml &rest _)
                  (if (equal method "GET")
                      (list :status 200 :body jaunder-timezone-test--service)
                    (cl-incf writes)
                    (list :status 200 :headers '(("etag" . "\"new\""))
                          :body (replace-regexp-in-string
                                 "<published>[^<]+</published>" publication
                                 (jaunder-timezone-test--member
                                  "Europe/London" "2026-07-01T13:00:42.123456789Z") t t))))))
       (should-error (jaunder-publish)))
     (should (= writes 1))
     (should (equal before (buffer-string)))
     (should (equal before (with-temp-buffer (insert-file-contents path) (buffer-string)))))))

(ert-deftest jaunder-timezone-duplicate-publication-receipt-does-not-checkpoint ()
  (jaunder-timezone-test--reject-publication-receipt
   (concat "<published>2026-07-01T13:00:42.123456789Z</published>"
           "<published>2026-07-02T14:15:00Z</published>")))

(ert-deftest jaunder-timezone-impossible-publication-receipt-does-not-checkpoint ()
  (jaunder-timezone-test--reject-publication-receipt
   "<published>2026-02-30T13:00:42.123456789Z</published>"))

(ert-deftest jaunder-timezone-malformed-merge-receipt-never-installs-authored-result ()
  "Even a draft receipt's optional publication value must be unambiguous/valid."
  (dolist (publication '("<published>2026-02-30T13:00:42.123456789Z</published>"
                         "<published>2026-07-01T13:00:00Z</published><published>2026-07-02T13:00:00Z</published>"))
    (let ((proof (jaunder-test--merge-lifecycle
                  (lambda (event _state)
                    (when (eq (plist-get event :kind) 'write)
                      (list :body (replace-regexp-in-string
                                   "</entry>" (concat publication "</entry>")
                                   (jaunder-test--write-entry
                                    "1" "post-001" "https://example.test/~alice/post-001") t t)))))))
      (should (eq (plist-get (car (plist-get proof :results)) :outcome) 'partial))
      (should (plist-get proof :retained))
      (should (equal (plist-get proof :bytes) (plist-get proof :original))))))

;;; jaunder-timezone-test.el ends here
