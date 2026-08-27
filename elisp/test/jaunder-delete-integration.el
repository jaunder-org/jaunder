;;; jaunder-delete-integration.el --- live Post deletion tests -*- lexical-binding: t; -*-

;; Copyright (C) 2026 Jaunder contributors

;;; Commentary:
;; Exercise explicit conditional deletion against the live AtomPub service.  The
;; fixture publishes a real Post, then proves both two-sided successful removal
;; and that stale or absent ETags leave its remote Member, local bytes, and
;; visited buffer intact.

;;; Code:

(require 'cl-lib)
(require 'ert)
(require 'jaunder)
(require 'jaunder-integration-helper)

(defun jaunder-delete-integration--delete-member-for-cleanup (root id)
  "Conditionally delete active Member ID under ROOT during test cleanup."
  (jaunder--with-blog root
                      (let* ((url (jaunder--member-url id))
                             (member (jaunder--http-request "GET" url)))
                        (when (eq (plist-get member :status) 200)
                          (let ((etag (jaunder--response-header member "ETag")))
                            (unless etag
                              (error "Live cleanup: Member %s has no ETag" id))
                            (let ((deleted
                                   (jaunder--http-request
                                    "DELETE" url nil nil (list (cons "If-Match" etag)))))
                              (unless (eq (plist-get deleted :status) 204)
                                (error "Live cleanup: DELETE Member %s returned %s"
                                       id (plist-get deleted :status)))))))))

(defmacro jaunder-delete-integration--with-published-post (&rest body)
  "Create a real Post in a visited buffer, then run BODY with `root', `buf', and `id'."
  (declare (indent 0) (debug t))
  `(jaunder-test--with-live-server
    (let* ((root (make-temp-file "jaunder-delete-post-" t))
           (path (expand-file-name "draft-20260101T000000.org" root))
           (jaunder-blogs
            (list (cons (file-name-as-directory root)
                        (list :base-url jaunder-test-base-url
                              :username jaunder-test-username))))
           (buf (progn
                  (with-temp-file path
                    (insert "#+TITLE: Delete live fixture\n"
                            "#+PROPERTY: JAUNDER_STATUS published\n\n"
                            "Delete live fixture body.\n"))
                  (find-file-noselect path)))
           (id nil)
           (completed nil))
      (unwind-protect
          (prog1
              (with-current-buffer buf
                (jaunder-publish)
                (setq id (jaunder--buffer-property "JAUNDER_ID"))
                (should id)
                (jaunder--with-blog (buffer-file-name)
                                    ,@body))
            (setq completed t))
        ;; Preserve the original assertion/error.  A passing test makes cleanup
        ;; failures loud, while a failing one still tears down its real Member.
        (if completed
            (jaunder-delete-integration--delete-member-for-cleanup root id)
          (ignore-errors
            (jaunder-delete-integration--delete-member-for-cleanup root id)))
        (when (buffer-live-p buf)
          (with-current-buffer buf
            (set-buffer-modified-p nil))
          (kill-buffer buf))
        (delete-directory root t)))))

(ert-deftest jaunder-delete-post-removes-active-member-and-local-post ()
  "A confirmed conditional deletion removes the active Member and visited file."
  (jaunder-delete-integration--with-published-post
   (let ((path (buffer-file-name))
         (synced (jaunder--buffer-property "JAUNDER_SYNCED")))
     (should synced)
     (cl-letf (((symbol-function 'y-or-n-p) (lambda (_) t)))
              (jaunder-delete-post))
     (should-not (buffer-live-p buf))
     (should-not (file-exists-p path))
     (should (eq (plist-get
                  (jaunder--http-request "GET" (jaunder--member-url id))
                  :status)
                 404)))))

(ert-deftest jaunder-delete-post-stale-etag-preserves-member-and-local-post ()
  "A stale conditional deletion preserves the active Member and visited file."
  (jaunder-delete-integration--with-published-post
   (jaunder--set-property "JAUNDER_SYNCED" "\"stale\"")
   (save-buffer)
   (let ((path (buffer-file-name))
         (before (buffer-string)))
     (cl-letf (((symbol-function 'y-or-n-p) (lambda (_) t)))
              (let ((err (should-error (jaunder-delete-post))))
                (should (equal (error-message-string err)
                               "jaunder: delete failed (HTTP 412)"))))
     (should (buffer-live-p buf))
     (should (file-exists-p path))
     (should (equal (with-temp-buffer
                      (insert-file-contents path)
                      (buffer-string))
                    before))
     (should (eq (plist-get
                  (jaunder--http-request "GET" (jaunder--member-url id))
                  :status)
                 200)))))

(ert-deftest jaunder-delete-post-missing-etag-preserves-member-and-local-post ()
  "A missing synchronization ETag does not delete locally or remotely."
  (jaunder-delete-integration--with-published-post
   (goto-char (point-min))
   (should (re-search-forward "^#\\+PROPERTY: JAUNDER_SYNCED.*\n" nil t))
   (delete-region (match-beginning 0) (match-end 0))
   (save-buffer)
   (let ((path (buffer-file-name))
         (before (buffer-string)))
     (let ((err (should-error (jaunder-delete-post))))
       (should (equal (error-message-string err)
                      "jaunder: JAUNDER_SYNCED must be a strong ETag")))
     (should (buffer-live-p buf))
     (should (file-exists-p path))
     (should (equal (with-temp-buffer
                      (insert-file-contents path)
                      (buffer-string))
                    before))
     (should (eq (plist-get
                  (jaunder--http-request "GET" (jaunder--member-url id))
                  :status)
                 200)))))

(provide 'jaunder-delete-integration)
;;; jaunder-delete-integration.el ends here
