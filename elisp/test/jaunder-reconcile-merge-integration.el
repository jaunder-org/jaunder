;;; jaunder-reconcile-merge-integration.el --- Live operation boundaries -*- lexical-binding: t; -*-

;; Copyright (C) 2026 Jaunder contributors

;;; Commentary:
;; Real server, plz/curl, Org files and public preview/merge/finish commands.
;; Ediff startup alone is controlled.  Lost response and checkpoint failure are
;; client-injected AFTER an observed real successful PUT, not server faults.

;;; Code:
(require 'ert)
(require 'jaunder)
(require 'jaunder-integration-helper)

(defun jaunder-test--merge-live-create (root name title body status)
  "Publish NAME under ROOT with TITLE, BODY and STATUS; return wire identity."
  (let ((buffer (find-file-noselect (expand-file-name name root))))
    (unwind-protect
        (with-current-buffer buffer
          (insert (format "#+TITLE: %s\n#+PROPERTY: JAUNDER_STATUS %s\n#+PROPERTY: JAUNDER_DATE_TZ UTC\n\n%s\n" title status body))
          (save-buffer)
          (let ((published (jaunder-publish)))
            (list :id (jaunder--buffer-property "JAUNDER_ID")
                  :path (buffer-file-name)
                  :etag (jaunder--buffer-property "JAUNDER_SYNCED")
                  :href (car (cdr (assq 'alternate-uris
                                        (jaunder--harvest-response-fields
                                         (plist-get (plist-get published :response) :body))))))))
      (when (buffer-live-p buffer) (kill-buffer buffer)))))

(ert-deftest jaunder-reconcile-live-merge-scopes-and-committed-failures ()
  "Each finish is fresh; actual commits cannot become rollback or safe retry."
  (dolist (fault '(nil stale lost checkpoint))
    (jaunder-test--with-live-server
     (let* ((root (file-name-as-directory (make-temp-file "jaunder-live-merge-" t)))
            (jaunder-blogs (list (cons root (list :base-url jaunder-test-base-url
                                                  :username jaunder-test-username))))
            (counts (make-hash-table :test #'eq))
            (member-counts (make-hash-table :test #'eq))
            (phase 'setup) scratch session report-buffer writes source target reviewed committed before-local
            (prefix (file-name-nondirectory (directory-file-name root))))
       (unwind-protect
           (jaunder--call-with-blog
            root
            (lambda ()
              (setq target (jaunder-test--merge-live-create root "target.org" (concat prefix " target") "Target body." "published")
                    source (jaunder-test--merge-live-create
                            root "source.org" (concat prefix " source")
                            (format "Initial [[file:./%s][Target]]."
                                    (file-name-nondirectory (plist-get target :path))) "draft"))
              (should (plist-get target :href))
              (let* ((uri (jaunder--member-url (plist-get source :id)))
                     (remote (jaunder--http-request
                              "PUT" uri
                              (jaunder--atom-entry->xml
                               (jaunder--make-entry :title (concat prefix " source") :draft t
                                                    :content-type "text/org"
                                                    :body (format "Remote [[%s][Target]]." (plist-get target :href))))
                              jaunder--entry-content-type
                              (list (cons "If-Match" (plist-get source :etag))))))
                (should (memq (plist-get remote :status) '(200 201)))
                (setq reviewed (jaunder--response-header remote "ETag")))
              (let ((buffer (find-file-noselect (plist-get source :path))))
                (with-current-buffer buffer
                  (goto-char (point-max)) (insert "Local author choice.\n") (save-buffer)
                  (setq before-local (buffer-string)))
                (kill-buffer buffer))
              (set-file-times (plist-get source :path) (time-add (current-time) (seconds-to-time 3)))
              (let ((real-http (symbol-function 'jaunder--http-request))
                    (real-refresh (symbol-function 'jaunder--reconcile-refresh-buffer))
                    (real-save (symbol-function 'save-buffer)))
                (cl-letf (((symbol-function 'y-or-n-p) (lambda (&rest _) t))
                          ((symbol-function 'jaunder--http-request)
                           (lambda (method url &rest args)
                             (when (and (equal method "GET") (equal (car (split-string url "?")) (jaunder--collection-url)))
                               (puthash phase (1+ (gethash phase counts 0)) counts))
                             (when (and (equal method "GET")
                                        (equal url (jaunder--member-url (plist-get source :id))))
                               (puthash phase (1+ (gethash phase member-counts 0)) member-counts))
                             (when (and (memq phase '(finish retry)) (equal method "PUT"))
                               (should (string-match-p (regexp-quote (plist-get target :href)) (car args)))
                               (push (nth 2 args) writes))
                             (let ((response (apply real-http method url args)))
                               (when (and (eq phase 'finish) (equal method "PUT"))
                                 (should (memq (plist-get response :status) '(200 201)))
                                 (setq committed response)
                                 (when (eq fault 'lost)
                                   (signal 'plz-error '("injected loss after real server commit"))))
                               response)))
                          ((symbol-function 'save-buffer)
                           (lambda (&rest args)
                             (if (and committed (eq fault 'checkpoint) (eq phase 'finish)
                                      (equal (buffer-file-name) (plist-get source :path)))
                                 (error "injected local save failure after real server commit")
                               (apply real-save args))))
                          ((symbol-function 'jaunder--reconcile-refresh-buffer)
                           (lambda (buffer)
                             (should-not (jaunder--operation-active-p))
                             (let ((before phase))
                               (setq phase 'refresh)
                               (unwind-protect (funcall real-refresh buffer)
                                 (setq phase before)))))
                          ((symbol-function 'ediff-merge-buffers)
                           (lambda (a _b &optional startup _job _file)
                             (should-not (jaunder--operation-active-p))
                             (setq scratch (generate-new-buffer " *Live merge C*"))
                             (with-current-buffer scratch
                               (insert (with-current-buffer a (buffer-string)) "\nLive merged result.\n"))
                             (with-temp-buffer
                               (setq-local ediff-buffer-C scratch)
                               (dolist (function startup) (funcall function))))))
                  (setq phase 'opening)
                  (jaunder-reconcile root)
                  (setq report-buffer
                        (cl-find-if (lambda (buffer)
                                      (with-current-buffer buffer
                                        (and (eq major-mode 'jaunder-reconcile-report-mode)
                                             (equal root (jaunder-reconcile-report-root jaunder-reconcile-report)))))
                                    (buffer-list)))
                  (should report-buffer)
                  (with-current-buffer report-buffer
                    (let ((row (cl-find (plist-get source :id)
                                        (jaunder-reconcile-report-rows jaunder-reconcile-report)
                                        :key #'jaunder--reconcile-row-post-id :test #'equal)))
                      (should (eq (jaunder-reconcile-row-state row) 'conflict))
                      (puthash (jaunder--reconcile-stable-row-key row) t jaunder-reconcile-marks))
                    (setq phase 'preparation)
                    (jaunder-reconcile-merge-selected))
                  (should (buffer-live-p scratch))
                  (setq session (buffer-local-value 'jaunder-reconcile-merge-session scratch))
                  (setq phase 'editing)
                  (should-not (jaunder--operation-active-p))
                  (when (eq fault 'stale)
                    (let ((response
                           (jaunder--http-request
                            "PUT" (jaunder--member-url (plist-get source :id))
                            (jaunder--atom-entry->xml
                             (jaunder--make-entry :title (concat prefix " source") :draft t
                                                  :content-type "text/org" :body "Changed during human editing."))
                            jaunder--entry-content-type (list (cons "If-Match" reviewed)))))
                      (should (memq (plist-get response :status) '(200 201)))))
                  (setq phase 'finish)
                  (let ((result (with-current-buffer scratch (jaunder-reconcile-merge-finish))))
                    (should (eq (plist-get result :outcome)
                                (pcase fault ('stale 'blocked) ('lost 'unknown)
                                       ('checkpoint 'partial) (_ 'success))))
                    (if (eq fault 'stale)
                        (progn (should (buffer-live-p scratch)) (should-not writes))
                      (should committed)
                      (should (= (length writes) 1))
                      (should (equal (cdr (assoc "If-Match" (car writes))) reviewed))
                      (setq phase 'verification)
                      (let ((member (jaunder--http-request "GET" (jaunder--member-url (plist-get source :id)))))
                        (should (string-match-p "Live merged result" (plist-get member :body))))
                      (setq phase 'finish))
                    (should (= (gethash 'preparation member-counts 0) 3))
                    (should (= (gethash 'finish member-counts 0) (if (eq fault 'stale) 1 2)))
                    (when (eq fault 'lost)
                      (should (equal before-local (with-temp-buffer
                                                    (insert-file-contents (plist-get source :path))
                                                    (buffer-string)))))
                    (when (eq fault 'checkpoint)
                      (should (string-match-p "Live merged result" (with-temp-buffer
                                                                     (insert-file-contents (plist-get source :path))
                                                                     (buffer-string)))))
                    (let ((pages (gethash 'opening counts 0)))
                      (should (> pages 0))
                      (dolist (part '(preparation finish refresh))
                        (should (= (gethash part counts 0) pages))))
                    (when (memq fault '(lost checkpoint))
                      (should (buffer-live-p scratch))
                      (should (string-match-p "Live merged result"
                                              (with-current-buffer scratch (buffer-string))))
                      (should (eq (jaunder-reconcile-result-outcome
                                   (car (buffer-local-value 'jaunder-reconcile-last-batch-results report-buffer)))
                                  (plist-get result :outcome)))
                      (setq phase 'retry)
                      (should (eq (plist-get (with-current-buffer scratch
                                               (jaunder-reconcile-merge-finish)) :outcome) 'blocked))
                      (should (= (length writes) 1)))
                    (message "Live merge fault=%s: %s; pages opening/preparation/finish=%s/%s/%s; PUTs=%s"
                             fault (plist-get result :outcome) (gethash 'opening counts)
                             (gethash 'preparation counts) (gethash 'finish counts) (length writes)))))))
         (when session
           (setf (jaunder-reconcile-merge-session-ediff-active session) nil)
           (jaunder--reconcile-merge-close-views session)
           (jaunder--reconcile-merge-cleanup session))
         (when (buffer-live-p report-buffer) (kill-buffer report-buffer))
         (delete-directory root t))))))

(provide 'jaunder-reconcile-merge-integration)
;;; jaunder-reconcile-merge-integration.el ends here
