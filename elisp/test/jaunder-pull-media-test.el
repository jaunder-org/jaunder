;;; jaunder-pull-media-test.el --- Pure pulled-media plan tests -*- lexical-binding: t; -*-

;; Copyright (C) 2026 Jaunder contributors

;;; Commentary:
;; Locks the source-preserving localization boundary before transport exists.

;;; Code:

(require 'ert)
(require 'jaunder)

(defconst jaunder-pull-media-test--hash
  "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855")
(defconst jaunder-pull-media-test--origin "https://Jaunder.example:443")

(defun jaunder-pull-media-test--url (filename &optional fragment)
  "Return the canonical public fixture URL for FILENAME and FRAGMENT."
  (format "https://jaunder.example/media/upload/e3/b0/%s/%s%s"
          jaunder-pull-media-test--hash filename (or fragment "")))

(defun jaunder-pull-media-test--rewrite (format body)
  "Return FORMAT BODY after pure localization planning and application."
  (jaunder--pull-media-apply-plan
   (jaunder--pull-media-plan format body jaunder-pull-media-test--origin)))

(ert-deftest jaunder-pull-media-org-plan-preserves-labels-fragments-and-duplicates ()
  ;; Repeated canonical URLs make one acquisition reference but retain every link.
  (let* ((url (jaunder-pull-media-test--url "my%20photo%25%E6%97%A5%E6%9C%AC.jpg" "#crop"))
         (body (format "Before [[%s][label]] and [[%s]] after" url url))
         (plan (jaunder--pull-media-plan "org" body jaunder-pull-media-test--origin))
         (reference (car (jaunder-pull-media-plan-references plan))))
    (should (= 1 (length (jaunder-pull-media-plan-references plan))))
    (should (equal (jaunder-pull-media-reference-leaf reference) "my photo%日本.jpg"))
    (should (= 2 (length (jaunder-pull-media-reference-replacements reference))))
    (should (equal (jaunder-pull-media-test--rewrite "org" body)
                   (concat "Before [[file:local-media/" jaunder-pull-media-test--hash
                           "/my%20photo%25%E6%97%A5%E6%9C%AC.jpg#crop][label]] and [[file:local-media/"
                           jaunder-pull-media-test--hash
                           "/my%20photo%25%E6%97%A5%E6%9C%AC.jpg#crop]] after")))))

(ert-deftest jaunder-pull-media-markdown-plan-rewrites-links-and-images-only ()
  ;; Markdown label and alt source are opaque; only their destinations change.
  (let* ((url (jaunder-pull-media-test--url "cafe%20%25%E2%98%95.png" "#view"))
         (body (format "[doc](%s) ![alt *kept*](%s) bare %s" url url url)))
    (should (equal (jaunder-pull-media-test--rewrite "markdown" body)
                   (format "[doc](local-media/%s/cafe%%20%%25%%E2%%98%%95.png#view) ![alt *kept*](local-media/%s/cafe%%20%%25%%E2%%98%%95.png#view) bare %s"
                           jaunder-pull-media-test--hash jaunder-pull-media-test--hash url)))))

(ert-deftest jaunder-pull-media-html-plan-rewrites-supported-attributes-and-srcset ()
  ;; HTML keeps attributes, ordering, script/CSS text, and non-link data intact.
  (let* ((one (jaunder-pull-media-test--url "one%20%25.jpg" "#a"))
         (two (jaunder-pull-media-test--url "%E6%97%A5%E6%9C%AC.png"))
         (body (format "<img src=\"%s\" srcset=\"%s 1x, %s 2x\" alt=\"x\"><a href=\"%s\">L</a><video poster=\"%s\"></video><style>x{background:url(%s)}</style><script>const x='%s'</script>" one one two two one one two))
         (out (jaunder-pull-media-test--rewrite "html" body)))
    (should (string-match-p (regexp-quote (format "src=\"local-media/%s/one%%20%%25.jpg#a\"" jaunder-pull-media-test--hash)) out))
    (should (string-match-p (regexp-quote (format "srcset=\"local-media/%s/one%%20%%25.jpg#a 1x, local-media/%s/%%E6%%97%%A5%%E6%%9C%%AC.png 2x\"" jaunder-pull-media-test--hash jaunder-pull-media-test--hash)) out))
    (should (string-match-p (regexp-quote (format "href=\"local-media/%s/%%E6%%97%%A5%%E6%%9C%%AC.png\"" jaunder-pull-media-test--hash)) out))
    (should (string-match-p (regexp-quote (format "poster=\"local-media/%s/one%%20%%25.jpg#a\"" jaunder-pull-media-test--hash)) out))
    (should (string-match-p (regexp-quote (format "url(%s)" one)) out))
    (should (string-match-p (regexp-quote (format "const x='%s'" two)) out))))

(ert-deftest jaunder-pull-media-rejects-every-non-candidate-class ()
  ;; Only canonical, same-origin public media destinations create a plan entry.
  (let* ((valid (jaunder-pull-media-test--url "ok.png"))
         (invalid (list
                   "http://jaunder.example/media/upload/e3/b0/e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855/ok.png"
                   "https://jaunder.example:444/media/upload/e3/b0/e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855/ok.png"
                   "//jaunder.example/media/upload/e3/b0/e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855/ok.png"
                   "https://user@jaunder.example/media/upload/e3/b0/e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855/ok.png"
                   "https://jaunder.example/media/upload/e3/b0/e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855/ok.png?x=1"
                   "https://jaunder.example/atompub/a/media/e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855/ok.png"
                   "https://jaunder.example/media/upload/ff/b0/e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855/ok.png"
                   "https://jaunder.example/media/upload/e3/b0/E3B0C44298FC1C149AFBF4C8996FB92427AE41E4649B934CA495991B7852B855/ok.png"
                   "data:image/png;base64,x"
                   "https://jaunder.example/media/upload/e3/b0/e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855/a%2Fb.png"))
         (body (concat (format "![good](%s)" valid)
                       (mapconcat (lambda (url) (format " [bad](%s)" url)) invalid "")))
         (plan (jaunder--pull-media-plan "markdown" body jaunder-pull-media-test--origin)))
    (should (= 1 (length (jaunder-pull-media-plan-references plan))))
    (dolist (url invalid)
      (should (string-match-p (regexp-quote url)
                              (jaunder--pull-media-apply-plan plan))))))

(provide 'jaunder-pull-media-test)
;;; jaunder-pull-media-test.el ends here
