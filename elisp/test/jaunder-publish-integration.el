;;; jaunder-publish-integration.el --- C4 live publish tests -*- lexical-binding: t; -*-
;;; Commentary:
;; End-to-end publish flow against a real server (#137 harness, ADR-0035).
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
    (let ((id (jaunder--buffer-property "JAUNDER_ID"))
          (slug (jaunder--buffer-property "JAUNDER_SLUG"))
          (synced (jaunder--buffer-property "JAUNDER_SYNCED")))
      (should id)
      (should slug)
      (should synced)
      (should (equal (file-name-nondirectory (buffer-file-name))
                     (concat slug ".org")))
      ;; Re-publish updates the same post (id unchanged), not a duplicate.
      (goto-char (point-max)) (insert "More.\n") (save-buffer)
      (jaunder-publish)
      (should (equal (jaunder--buffer-property "JAUNDER_ID") id))))))

(ert-deftest jaunder-publish-stale-if-match-surfaces-412 ()
  (jaunder-test--with-live-server
   (jaunder-pub-test--in-buffer
    "#+TITLE: T\n#+PROPERTY: JAUNDER_STATUS published\n\nBody.\n"
    (jaunder-publish)
    ;; Corrupt the stored ETag → the next PUT must 412 and leave the file intact.
    (jaunder--set-property "JAUNDER_SYNCED" "\"stale\"") (save-buffer)
    (let ((before (buffer-string)))
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
    (should (jaunder--buffer-property "JAUNDER_ID")))))

(provide 'jaunder-publish-integration)
;;; jaunder-publish-integration.el ends here
