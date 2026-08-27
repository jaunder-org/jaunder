;;; jaunder-delete-test.el --- Explicit Post deletion tests -*- lexical-binding: t; -*-

;; Copyright (C) 2026 Jaunder contributors

;;; Commentary:
;; Pure contracts for local validation, confirmation, conditional AtomPub
;; deletion, and the strict local-state preservation rule.

;;; Code:

(require 'ert)
(require 'cl-lib)
(require 'jaunder)

(defmacro jaunder-delete-test--with-visited-post (contents &rest body)
  "Run BODY in a visited Post containing CONTENTS."
  (declare (indent 1) (debug t))
  `(let* ((root (make-temp-file "jaunder-delete-post-" t))
          (path (expand-file-name "post.org" root))
          (jaunder-blogs
           (list (cons (file-name-as-directory root)
                       (list :base-url "https://example.test" :username "alice"))))
          buffer)
     (unwind-protect
         (progn
           (with-temp-file path (insert ,contents))
           (setq buffer (find-file-noselect path))
           (with-current-buffer buffer
             (org-mode)
             ,@body))
       (when (buffer-live-p buffer)
         (with-current-buffer buffer (set-buffer-modified-p nil))
         (kill-buffer buffer))
       (when (file-exists-p root)
         (delete-directory root t)))))

(defun jaunder-delete-test--assert-post-preserved (path buffer contents)
  "Assert PATH and BUFFER still contain CONTENTS."
  (should (file-exists-p path))
  (should (buffer-live-p buffer))
  (should (equal (with-temp-buffer
                   (insert-file-contents path)
                   (buffer-string))
                 contents))
  (with-current-buffer buffer
    (should (equal (buffer-string) contents))))

(ert-deftest jaunder-strong-etag-p-accepts-only-http-etagc ()
  "Strong validators exclude whitespace and controls but retain valid etagc."
  (should (jaunder--strong-etag-p "\"!#~\""))
  (should (jaunder--strong-etag-p (concat "\"" (string #x80) "\"")))
  (dolist (etag (list "\"two words\""
                      "\"two\twords\""
                      (concat "\"two" (string 1) "words\"")
                      (concat "\"two" (string 127) "words\"")))
    (should-not (jaunder--strong-etag-p etag))))

(ert-deftest jaunder-delete-post-rejects-unvisited-buffer-before-prompt-or-network ()
  "A deletion must originate from a visited Org buffer."
  (let (prompted requested)
    (cl-letf (((symbol-function 'y-or-n-p) (lambda (_) (setq prompted t)))
              ((symbol-function 'jaunder--http-request)
               (lambda (&rest _) (setq requested t))))
             (with-temp-buffer
               (should-error (jaunder-delete-post))))
    (should-not prompted)
    (should-not requested)))

(ert-deftest jaunder-delete-post-validates-id-and-etag-before-prompt-or-network ()
  "Malformed local deletion markers never reach confirmation or transport."
  (dolist (markers '("#+PROPERTY: JAUNDER_SYNCED \"etag\"\n"
                     "#+PROPERTY: JAUNDER_ID nope\n#+PROPERTY: JAUNDER_SYNCED \"etag\"\n"
                     "#+PROPERTY: JAUNDER_ID 7\n"
                     "#+PROPERTY: JAUNDER_ID 7\n#+PROPERTY: JAUNDER_SYNCED \"etag\n"
                     "#+PROPERTY: JAUNDER_ID 7\n#+PROPERTY: JAUNDER_SYNCED W/\"etag\"\n"
                     "#+PROPERTY: JAUNDER_ID 7\n#+PROPERTY: JAUNDER_SYNCED etag\n"
                     "#+PROPERTY: JAUNDER_ID 7\n#+PROPERTY: JAUNDER_SYNCED \"two words\"\n"
                     "#+PROPERTY: JAUNDER_ID 7\n#+PROPERTY: JAUNDER_SYNCED \"bad\tvalue\"\n"
                     "#+PROPERTY: JAUNDER_ID 7\n#+PROPERTY: JAUNDER_SYNCED \"bad\C-a value\"\n"))
    (let ((contents (concat markers "\nBody\n"))
          prompted requested)
      (jaunder-delete-test--with-visited-post contents
                                              (cl-letf (((symbol-function 'y-or-n-p) (lambda (_) (setq prompted t)))
                                                        ((symbol-function 'jaunder--http-request)
                                                         (lambda (&rest _) (setq requested t))))
                                                       (should-error (jaunder-delete-post)))
                                              (should-not prompted)
                                              (should-not requested)
                                              (jaunder-delete-test--assert-post-preserved path buffer contents)))))

(ert-deftest jaunder-delete-post-cancellation-preserves-local-post-without-request ()
  "Cancellation leaves the visited file and buffer untouched."
  (let ((contents "#+PROPERTY: JAUNDER_ID 7\n#+PROPERTY: JAUNDER_SYNCED \"etag\"\n\nBody\n")
        requested)
    (jaunder-delete-test--with-visited-post contents
                                            (cl-letf (((symbol-function 'y-or-n-p) (lambda (_) nil))
                                                      ((symbol-function 'jaunder--http-request)
                                                       (lambda (&rest _) (setq requested t))))
                                                     (jaunder-delete-post))
                                            (should-not requested)
                                            (jaunder-delete-test--assert-post-preserved path buffer contents))))

(ert-deftest jaunder-delete-post-sends-conditional-member-delete-and-removes-local-post-on-204 ()
  "A confirmed 204 removes only the corresponding local file and buffer."
  (let ((contents "#+PROPERTY: JAUNDER_ID 0007\n#+PROPERTY: JAUNDER_SYNCED \"etag\"\n\nBody\n")
        request)
    (jaunder-delete-test--with-visited-post contents
                                            (cl-letf (((symbol-function 'y-or-n-p) (lambda (_) t))
                                                      ((symbol-function 'jaunder--http-request)
                                                       (lambda (&rest args) (setq request args) '(:status 204))))
                                                     (jaunder-delete-post))
                                            (should (equal request
                                                           '("DELETE" "https://example.test/atompub/alice/posts/7"
                                                             nil nil (("If-Match" . "\"etag\"")))))
                                            (should-not (file-exists-p path))
                                            (should-not (buffer-live-p buffer)))))

(ert-deftest jaunder-delete-post-preserves-local-post-on-http-failure ()
  "Every non-204 status surfaces its exact status and preserves local state."
  (dolist (status '(404 412 200))
    (let ((contents "#+PROPERTY: JAUNDER_ID 7\n#+PROPERTY: JAUNDER_SYNCED \"etag\"\n\nBody\n"))
      (jaunder-delete-test--with-visited-post contents
                                              (cl-letf (((symbol-function 'y-or-n-p) (lambda (_) t))
                                                        ((symbol-function 'jaunder--http-request)
                                                         (lambda (&rest _) (list :status status))))
                                                       (let ((err (should-error (jaunder-delete-post))))
                                                         (should (equal (error-message-string err)
                                                                        (format "jaunder: delete failed (HTTP %s)" status))))
                                                       (jaunder-delete-test--assert-post-preserved path buffer contents))))))

(ert-deftest jaunder-delete-post-preserves-local-post-on-transport-failure ()
  "The original transport condition is surfaced while local state is retained."
  (let ((contents "#+PROPERTY: JAUNDER_ID 7\n#+PROPERTY: JAUNDER_SYNCED \"etag\"\n\nBody\n"))
    (jaunder-delete-test--with-visited-post contents
                                            (cl-letf (((symbol-function 'y-or-n-p) (lambda (_) t))
                                                      ((symbol-function 'jaunder--http-request)
                                                       (lambda (&rest _) (error "offline"))))
                                                     (let ((err (should-error (jaunder-delete-post))))
                                                       (should (equal (error-message-string err) "offline"))
                                                       (should (equal err '(error "offline")))))
                                            (jaunder-delete-test--assert-post-preserved path buffer contents))))

(provide 'jaunder-delete-test)
;;; jaunder-delete-test.el ends here
