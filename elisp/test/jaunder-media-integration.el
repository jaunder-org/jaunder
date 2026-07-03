;;; jaunder-media-integration.el --- C3 live media upload tests -*- lexical-binding: t; -*-

;;; Commentary:
;; Exercises media upload + content-src substitution end-to-end against a real
;; server (#137 harness, ADR-0035).  Runs via `cargo xtask elisp-integration'.

;;; Code:

(require 'ert)
(require 'jaunder)
(require 'jaunder-integration-helper)

(ert-deftest jaunder-media-upload-and-substitute-sent-body ()
  "A local image is uploaded and its link rewritten in the sent body only."
  (jaunder-test--with-live-server
   (let* ((dir (make-temp-file "jaunder-media-" t))
          (img (expand-file-name "pic.png" dir)))
     (unwind-protect
         (progn
           (with-temp-file img (insert "PNG-BYTES-abc"))
           (with-temp-buffer
             (insert (format "#+TITLE: T\n\nHere [[file:%s]] ok.\n" img))
             (org-mode)
             (let* ((before (buffer-string))
                    (body (jaunder-entry-body (jaunder--org->atom)))
                    (out (jaunder--localize-media body)))
               (should (string-match-p "/media/upload/" out))
               (should (string-match-p "pic.png" out))
               (should-not (string-match-p (regexp-quote img) out))
               (should (equal (buffer-string) before)))))
       (delete-directory dir t)))))

(ert-deftest jaunder-media-upload-is-idempotent ()
  "Re-uploading identical bytes returns HTTP 200 (existed) and the same URL.
Asserting the status — not just URL equality — is what proves dedup: the URL is
content-addressed, so it would match even if the server stored a duplicate; only
the 200 (vs 201) distinguishes the `existed' branch (media.rs)."
  (jaunder-test--with-live-server
   (let* ((dir (make-temp-file "jaunder-media-" t))
          (img (expand-file-name "same.png" dir)))
     (unwind-protect
         (progn
           (with-temp-file img (insert "IDEMPOTENT"))
           (let* ((u1 (jaunder--upload-media img "image/png"))
                  (resp2 (jaunder--http-request
                          "POST"
                          (jaunder--build-url jaunder-base-url "atompub"
                                              jaunder-username "media")
                          (list 'file img) "image/png"
                          (list (cons "Slug" "same.png"))))
                  (u2 (cdr (assq 'content-src
                                 (jaunder--atom-entry-fields
                                  (plist-get resp2 :body))))))
             (should (string-match-p "/media/upload/" u1))
             (should (= (plist-get resp2 :status) 200))
             (should (equal u1 u2))))
       (delete-directory dir t)))))

(ert-deftest jaunder-media-upload-rejection-surfaces-non-2xx ()
  "A rejected upload (wrong-user path) returns a non-2xx status, not a signal.
Confirms over the wire that the server really rejects with a 4xx — the condition
`jaunder--upload-media' turns into an error (its own abort branch is unit-tested
with a stub in Task 7).  Uses a mismatched username in the path with valid alice
credentials, so `require_user_match' fails deterministically (403)."
  (jaunder-test--with-live-server
   (let* ((dir (make-temp-file "jaunder-media-" t))
          (img (expand-file-name "x.png" dir)))
     (unwind-protect
         (progn
           (with-temp-file img (insert "X"))
           (let ((resp (jaunder--http-request
                        "POST"
                        (jaunder--build-url jaunder-base-url "atompub" "not-alice" "media")
                        (list 'file img) "image/png"
                        (list (cons "Slug" "x.png")))))
             (should (>= (plist-get resp :status) 400))
             (should (< (plist-get resp :status) 500))))
       (delete-directory dir t)))))

(ert-deftest jaunder-media-attachment-resolves-and-uploads ()
  "An `attachment:' link resolves via a per-heading DIR and uploads."
  (jaunder-test--with-live-server
   (let* ((dir (make-temp-file "jaunder-att-" t))
          (attach (expand-file-name "att" dir))
          (img (expand-file-name "a.png" attach)))
     (unwind-protect
         (progn
           (make-directory attach t)
           (with-temp-file img (insert "ATTACH"))
           (with-temp-buffer
             (org-mode)
             (insert (format "* H\n:PROPERTIES:\n:DIR: %s\n:END:\n\n[[attachment:a.png]]\n"
                             attach))
             (let ((out (jaunder--localize-media
                         (jaunder-entry-body (jaunder--org->atom)))))
               (should (string-match-p "/media/upload/" out))
               (should (string-match-p "a.png" out)))))
       (delete-directory dir t)))))

(provide 'jaunder-media-integration)
;;; jaunder-media-integration.el ends here
