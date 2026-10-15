;;; jaunder-media-integration.el --- live media upload tests -*- lexical-binding: t; -*-

;;; Commentary:
;; Exercises media upload + content-src substitution end-to-end against a real
;; server (harness, ADR-0035).  Runs via `cargo xtask elisp-integration'.

;;; Code:

(require 'ert)
(require 'jaunder)
(require 'jaunder-integration-helper)

(ert-deftest jaunder-media-upload-and-substitute-sent-body ()
  "A local image is uploaded and its link rewritten in the sent body only."
  (jaunder-test--with-live-server
   (jaunder-test--with-temp-directory (dir "jaunder-media-")
                                      (let ((img (expand-file-name "pic.png" dir)))
                                        (jaunder-test--copy-image "png-sanitized.png" img)
                                        (with-temp-buffer
                                          (insert (format "#+TITLE: T\n\nHere [[file:%s]] ok.\n" img))
                                          (org-mode)
                                          (let* ((before (buffer-string))
                                                 (body (jaunder-entry-body (jaunder--org->atom)))
                                                 (out (jaunder--localize-media body)))
                                            (should (string-match-p "/media/upload/" out))
                                            (should (string-match-p "pic.png" out))
                                            (should-not (string-match-p (regexp-quote img) out))
                                            (should (equal (buffer-string) before))))))))

(ert-deftest jaunder-media-upload-is-idempotent ()
  "Re-uploading identical bytes returns HTTP 200 (existed) and the same URL.
Asserting the status — not just URL equality — is what proves dedup: the URL is
content-addressed, so it would match even if the server stored a duplicate; only
the 200 (vs 201) distinguishes the `existed' branch (media.rs)."
  (jaunder-test--with-live-server
   (jaunder-test--with-temp-directory (dir "jaunder-media-")
                                      (let ((img (expand-file-name "same.png" dir)))
                                        (jaunder-test--copy-image "png-sanitized.png" img)
                                        (let* ((u1 (jaunder--upload-media img "image/png"))
                                               (resp2 (jaunder--http-request
                                                       "POST"
                                                       (jaunder--build-url jaunder-test-base-url "atompub"
                                                                           jaunder-test-username "media")
                                                       (list 'file img) "image/png"
                                                       (list (cons "Slug" "same.png"))))
                                               (u2 (cdr (assq 'content-src
                                                              (jaunder--harvest-response-fields
                                                               (plist-get resp2 :body))))))
                                          (should (string-match-p "/media/upload/" u1))
                                          (should (= (plist-get resp2 :status) 200))
                                          (should (equal u1 u2)))))))

(ert-deftest jaunder-media-upload-rejection-surfaces-non-2xx ()
  "A rejected upload (wrong-user path) returns a non-2xx status, not a signal.
Confirms over the wire that the server really rejects with a 4xx — the condition
`jaunder--upload-media' turns into an error (its own abort branch is unit-tested
with a stub).  Uses a mismatched username in the path with valid alice
credentials, so `require_user_match' fails deterministically (403)."
  (jaunder-test--with-live-server
   (jaunder-test--with-temp-directory (dir "jaunder-media-")
                                      (let ((img (expand-file-name "x.png" dir)))
                                        (with-temp-file img (insert "X"))
                                        (let ((resp (jaunder--http-request
                                                     "POST"
                                                     (jaunder--build-url jaunder-test-base-url "atompub" "not-alice" "media")
                                                     (list 'file img) "image/png"
                                                     (list (cons "Slug" "x.png")))))
                                          (should (>= (plist-get resp :status) 400))
                                          (should (< (plist-get resp :status) 500)))))))

(ert-deftest jaunder-media-attachment-resolves-and-uploads ()
  "An `attachment:' link resolves via a per-heading DIR and uploads."
  (jaunder-test--with-live-server
   (jaunder-test--with-temp-directory (dir "jaunder-att-")
                                      (let* ((attach (expand-file-name "att" dir))
                                             (img (expand-file-name "a.png" attach)))
                                        (make-directory attach t)
                                        (jaunder-test--copy-image "png-sanitized.png" img)
                                        (with-temp-buffer
                                          (org-mode)
                                          (insert (format "* H\n:PROPERTIES:\n:DIR: %s\n:END:\n\n[[attachment:a.png]]\n"
                                                          attach))
                                          (let ((out (jaunder--localize-media
                                                      (jaunder-entry-body (jaunder--org->atom)))))
                                            (should (string-match-p "/media/upload/" out))
                                            (should (string-match-p "a.png" out))))))))

(ert-deftest jaunder-media-private-images-keep-author-originals-and-serve-sanitized-identity ()
  "PNG/JPEG uploads retain author originals but serve only sanitized bytes."
  (jaunder-test--with-live-server
   (jaunder-test--with-temp-directory
    (dir "jaunder-private-media-")
    (dolist (case '(("png-original.png" "png-sanitized.png" "image/png")
                    ("jpeg-original.jpg" "jpeg-sanitized.jpg" "image/jpeg")))
      (let* ((original (jaunder-test--image-bytes (car case)))
             (sanitized (jaunder-test--image-bytes (cadr case)))
             (image (expand-file-name (car case) dir))
             (download (make-temp-file (expand-file-name "public-copy-" dir)))
             (hash (secure-hash 'sha256 sanitized)))
        (jaunder-test--copy-image (car case) image)
        (let* ((url (jaunder--upload-media image (nth 2 case)))
               (response (jaunder--pull-media-get url download)))
          (should-not (equal original sanitized))
          (should-not (string-match-p (secure-hash 'sha256 original) url))
          (should (string-match-p hash url))
          (should (= (plist-get response :status) 200))
          (should (equal (jaunder--pull-media-header-values response "etag")
                         (list (format "\"sha256-%s\"" hash))))
          (should (equal (jaunder-test--file-bytes download) sanitized))
          (should (equal (jaunder-test--file-bytes image) original))
          (should (equal (jaunder--upload-media image (nth 2 case)) url))
          (should (equal (jaunder-test--file-bytes image) original))))))))

(provide 'jaunder-media-integration)
;;; jaunder-media-integration.el ends here
