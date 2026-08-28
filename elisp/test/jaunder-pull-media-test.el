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

(ert-deftest jaunder-pull-media-html-scans-only-real-case-insensitive-attributes ()
  ;; Attribute-looking samples, data-* names, comments, and raw text are opaque.
  (let* ((url (jaunder-pull-media-test--url "safe.png"))
         (body (format "<!-- <img SRC=%s> --><img DATA-src=\"%s\" SRC=\"%s\"><script><img href=\"%s\"></script>"
                       url url url url))
         (out (jaunder-pull-media-test--rewrite "html" body)))
    (should (string-match-p (regexp-quote (format "SRC=\"local-media/%s/safe.png\""
                                                  jaunder-pull-media-test--hash)) out))
    (should (string-match-p (regexp-quote (format "DATA-src=\"%s\"" url)) out))
    (should (string-match-p (regexp-quote (format "<!-- <img SRC=%s> -->" url)) out))
    (should (string-match-p (regexp-quote (format "<script><img href=\"%s\"></script>" url)) out))))

(ert-deftest jaunder-pull-media-markdown-rewrites-standard-destinations-only ()
  (let* ((url (jaunder-pull-media-test--url "safe.png"))
         (body (format "[angle](<%s> \"title\")\n[ref]: <%s> 'title'\n`[no](%s)`\n```\n[x](%s)\n```"
                       url url url url))
         (out (jaunder-pull-media-test--rewrite "markdown" body)))
    (should (string-match-p (regexp-quote (format "[angle](<local-media/%s/safe.png> \"title\")"
                                                  jaunder-pull-media-test--hash)) out))
    (should (string-match-p (regexp-quote (format "`[no](%s)`" url)) out))))

(ert-deftest jaunder-pull-media-markdown-ast-keeps-literals-and-used-references ()
  (let* ((url (jaunder-pull-media-test--url "safe.png"))
         (body (format "[outer [label]](%s \"title\") [use][r]\n[r]: <%s> 'title'\n[unused]: %s\n~~~~\n[x](%s)\n~~~~\n``[code](%s)``"
                       url url url url url))
         (out (jaunder-pull-media-test--rewrite "markdown" body)))
    (should (string-match-p (regexp-quote (format "[outer [label]](local-media/%s/safe.png \"title\")"
                                                  jaunder-pull-media-test--hash)) out))
    (should (string-match-p (regexp-quote (format "[r]: <local-media/%s/safe.png> 'title'"
                                                  jaunder-pull-media-test--hash)) out))
    (should (string-match-p (regexp-quote (format "[unused]: %s" url)) out))
    (should (string-match-p (regexp-quote (format "[x](%s)" url)) out))
    (should (string-match-p (regexp-quote (format "``[code](%s)``" url)) out))))

(ert-deftest jaunder-pull-media-org-rewrites-bracket-plain-and-angle-links ()
  (let* ((url (jaunder-pull-media-test--url "safe.png"))
         (target
          (format "file:local-media/%s/safe.png"
                  jaunder-pull-media-test--hash))
         (body (format "plain %s angle <%s> bracket [[%s]]"
                       url url url)))
    (should
     (equal
      (jaunder-pull-media-test--rewrite "org" body)
      (format "plain %s angle <%s> bracket [[%s]]"
              target target target)))))

(ert-deftest jaunder-pull-media-markdown-handles-eof-labels-and-literal-contexts ()
  (let* ((url (jaunder-pull-media-test--url "safe.png"))
         (target
          (format "local-media/%s/safe.png"
                  jaunder-pull-media-test--hash))
         (reference
          (format "[use][asset\\]]\n[asset\\]]: <%s>" url))
         (contexts
          (format
           (concat
            "[label ``]`` kept](<%s>)\n"
            "```\n```not-a-closer\n[x](%s)\n```\n"
            "<!-- <%s> -->\n<script>[x](%s)</script>\n"
            "<div>\n[x](%s)\n</div>\n\n"
            "[real](%s)")
           url url url url url url))
         (unclosed (format "<script>\n[x](%s)" url))
         (out (jaunder-pull-media-test--rewrite "markdown" contexts)))
    (should
     (equal
      (jaunder-pull-media-test--rewrite "markdown" reference)
      (format "[use][asset\\]]\n[asset\\]]: <%s>" target)))
    (should
     (string-match-p
      (regexp-quote (format "[label ``]`` kept](<%s>)" target))
      out))
    (should
     (string-match-p (regexp-quote (format "[x](%s)" url)) out))
    (should
     (string-match-p (regexp-quote (format "<!-- <%s> -->" url)) out))
    (should
     (string-match-p
      (regexp-quote (format "<script>[x](%s)</script>" url))
      out))
    (should
     (string-match-p
      (regexp-quote (format "<div>\n[x](%s)\n</div>" url))
      out))
    (should
     (equal
      (jaunder-pull-media-test--rewrite "markdown" unclosed)
      unclosed))
    (should
     (string-match-p
      (regexp-quote (format "[real](%s)" target))
      out))))

(ert-deftest jaunder-pull-media-markdown-ast-handles-html-and-quoted-code-blocks ()
  (let* ((url (jaunder-pull-media-test--url "safe.png"))
         (target
          (format "local-media/%s/safe.png"
                  jaunder-pull-media-test--hash))
         (inline-html
          (format "<span>[inline](%s)</span>" url))
         (inline-pre
          (format "prefix <pre>[inline](%s)</pre>" url))
         (fenced-html
          (format
           "```\n<pre>\n[x](%s)\n</pre>\n```\n[after](%s)"
           url url))
         (quoted-fence
          (format
           "> ```\n> [x](%s)\n> ```\n[after](%s)"
           url url)))
    (should
     (equal
      (jaunder-pull-media-test--rewrite "markdown" inline-html)
      (format "<span>[inline](%s)</span>" target)))
    (should
     (equal
      (jaunder-pull-media-test--rewrite "markdown" inline-pre)
      (format "prefix <pre>[inline](%s)</pre>" target)))
    (let ((out
           (jaunder-pull-media-test--rewrite
            "markdown" fenced-html)))
      (should
       (string-match-p
        (regexp-quote (format "[x](%s)" url))
        out))
      (should
       (string-match-p
        (regexp-quote (format "[after](%s)" target))
        out)))
    (let ((out
           (jaunder-pull-media-test--rewrite
            "markdown" quoted-fence)))
      (should
       (string-match-p
        (regexp-quote (format "> [x](%s)" url))
        out))
      (should
       (string-match-p
        (regexp-quote (format "[after](%s)" target))
        out)))))

(ert-deftest jaunder-pull-media-html-rejects-incomplete-and-raw-text-markup ()
  (let* ((url (jaunder-pull-media-test--url "safe.png"))
         (target
          (format "local-media/%s/safe.png"
                  jaunder-pull-media-test--hash))
         (incomplete (format "<img src=\"%s\"" url))
         (malformed (format "<1 href=\"%s\">" url))
         (body
          (format
           (concat
            "<textarea><img src=\"%s\"></textarea>"
            "<title><a href=\"%s\"></title>"
            "<pre><img src=\"%s\"></pre>"
            "<iframe src=\"%s\"></iframe>"
            "<IMG SRC=\"%s\">")
           url url url url url))
         (out (jaunder-pull-media-test--rewrite "html" body)))
    (should
     (equal
      (jaunder-pull-media-test--rewrite "html" incomplete)
      incomplete))
    (should
     (equal
      (jaunder-pull-media-test--rewrite "html" malformed)
      malformed))
    (should
     (string-match-p
      (regexp-quote
       (format "<textarea><img src=\"%s\"></textarea>" url))
      out))
    (should
     (string-match-p
      (regexp-quote
       (format "<title><a href=\"%s\"></title>" url))
      out))
    (should
     (string-match-p
      (regexp-quote
       (format "<pre><img src=\"%s\"></pre>" target))
      out))
    (should
     (string-match-p
      (regexp-quote
       (format "<iframe src=\"%s\"></iframe>" target))
      out))
    (should
     (string-suffix-p (format "<IMG SRC=\"%s\">" target) out))))

(ert-deftest jaunder-pull-media-markdown-delegates-block-semantics-to-cmark ()
  ;; The source adapter may locate bytes, but cmark alone decides their meaning.
  (let* ((url (jaunder-pull-media-test--url "safe.png"))
         (target (format "local-media/%s/safe.png"
                         jaunder-pull-media-test--hash))
         (body
          (format
           (concat
            "<script>\n[one](%s)\n</script>\n[two](%s)\n"
            "<table>\n[three](%s)\n\n[four](%s)\n"
            "> <table>\n> [five](%s)\n>\n[six](%s)\n"
            "paragraph\n<custom>\n[seven](%s)\n"
            "- ```\n  [eight](%s)\n  ```\n[nine](%s)\n"
            "> ```\n> [ten](%s)\n> ```\n[eleven](%s)\n"
            "inline <pre>[twelve](%s)</pre> <span>[thirteen](%s)</span>\n"
            "<!-- [fourteen](%s) -->\n`[fifteen](%s)`\n"
            "[used][asset] [plain] < %s > <%s>\n"
            "[asset]: <%s>\n[other]: <%s>\n[bad](%s")
           url url url url url url url url url url url url url url url url url
           url url url))
         (out (jaunder-pull-media-test--rewrite "markdown" body)))
    (dolist (literal (list "one" "three" "five" "eight" "ten"
                           "fourteen" "fifteen"))
      (should (string-match-p
               (regexp-quote (format "[%s](%s)" literal url)) out)))
    (dolist (rewritten (list "two" "four" "six" "seven" "nine" "eleven"
                             "twelve" "thirteen"))
      (should (string-match-p
               (regexp-quote (format "[%s](%s)" rewritten target)) out)))
    (should (string-match-p (regexp-quote (format "[asset]: <%s>" target))
                            out))
    (should (string-match-p (regexp-quote (format "[other]: <%s>" url)) out))
    (should (string-match-p (regexp-quote (format "<%s>" target)) out))
    (should (string-suffix-p (format "[bad](%s" url) out))))

(ert-deftest jaunder-pull-media-canonical-identifiers-are-case-sensitive ()
  (let* ((upper (upcase jaunder-pull-media-test--hash))
         (url
          (format "https://jaunder.example/media/upload/E3/B0/%s/safe.png"
                  upper))
         (failure
          (should-error
           (jaunder--pull-media-url-parts
            url jaunder-pull-media-test--origin))))
    (should
     (equal (error-message-string failure)
            (format "jaunder pull media: malformed canonical media URL: %s"
                    url)))))

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
                   "data:image/png;base64,x"))
         (body (concat (format "![good](%s)" valid)
                       (mapconcat (lambda (url) (format " [bad](%s)" url)) invalid "")))
         (plan (jaunder--pull-media-plan "markdown" body jaunder-pull-media-test--origin)))
    (should (= 1 (length (jaunder-pull-media-plan-references plan))))
    (dolist (url invalid)
      (should (string-match-p (regexp-quote url)
                              (jaunder--pull-media-apply-plan plan))))))

(ert-deftest jaunder-pull-media-rejects-malformed-same-origin-media-route ()
  (dolist (url
           (list
            (format "https://jaunder.example/media/upload/ff/b0/%s/bad.png"
                    jaunder-pull-media-test--hash)
            (format "https://jaunder.example/media/upload/e3/b0/%s/a%%2Fb.png"
                    jaunder-pull-media-test--hash)))
    (let ((failure
           (should-error
            (jaunder--pull-media-plan "markdown" (format "[bad](%s)" url)
                                      jaunder-pull-media-test--origin)
            :type 'error)))
      (should (equal (error-message-string failure)
                     (if (string-match-p "a%2F" url)
                         (format "jaunder pull media: malformed canonical media filename: %s" url)
                       (format "jaunder pull media: malformed canonical media URL: %s" url)))))))

(defun jaunder-pull-media-test--materialization-plan (hash leaf &optional references)
  "Return a one-object localization plan for HASH and decoded LEAF."
  (ignore references)
  (jaunder--make-pull-media-plan
   :format "org" :body "" :references (or references
                                          (list (jaunder--make-pull-media-reference
                                                 :url (jaunder-pull-media-test--url
                                                       (jaunder--pull-media-encode-filename leaf))
                                                 :hash hash :leaf leaf
                                                 :target "" :replacements nil)))))

(defconst jaunder-pull-media-test--instance
  "123e4567-e89b-12d3-a456-426614174000")

(defun jaunder-pull-media-test--response (instance hash)
  "Return the accepted media response metadata for INSTANCE and HASH."
  (list :status 200
        :headers (list (cons "x-jaunder-instance" instance)
                       (cons "etag" (format "\"sha256-%s\"" hash)))))

(defmacro jaunder-pull-media-test--with-root (root &rest body)
  "Evaluate BODY with ROOT bound to a newly-created temporary directory."
  (declare (indent 1) (debug (symbolp body)))
  `(let ((,root (make-temp-file "jaunder-pull-media-test-" t)))
     (unwind-protect
         (progn ,@body)
       (delete-directory ,root t))))

(defun jaunder-pull-media-test--write-bytes (path bytes)
  "Write unibyte BYTES to PATH without coding conversion."
  (let ((coding-system-for-write 'no-conversion))
    (write-region bytes nil path nil 'silent)))

(ert-deftest jaunder-pull-media-file-hash-is-derived-from-literal-file-bytes ()
  ;; A pathname's bytes are not the Local Media Copy's bytes.
  (let ((path (make-temp-file "jaunder-pull-media-hash-"))
        (bytes (string-as-unibyte "\0\377media")))
    (unwind-protect
        (progn
          (jaunder-pull-media-test--write-bytes path bytes)
          (should (equal (jaunder--pull-media-file-sha256 path)
                         (secure-hash 'sha256 bytes)))
          (should-not (equal (jaunder--pull-media-file-sha256 path)
                             (secure-hash 'sha256 path))))
      (delete-file path))))
(ert-deftest jaunder-pull-media-anonymous-get-is-binary-direct-and-propagates-transport-errors ()
  ;; The public-media leg has neither credentials nor redirect following.
  (let ((destination (make-temp-file "jaunder-media-"))
        captured)
    (unwind-protect
        (cl-letf (((symbol-function 'plz)
                   (lambda (method url &rest arguments)
                     (setq captured (list method url arguments
                                          (member "--location" plz-curl-default-args)))
                     (make-plz-response :status 200 :headers '((etag . "\"x\""))
                                        :body (string-as-unibyte "\0\377bytes")))))
                 (let ((response (jaunder--pull-media-get "https://example.test/media" destination)))
                   (should (equal (car captured) 'get))
                   (should (equal (cadr captured) "https://example.test/media"))
                   (should (eq (plist-get (nth 2 captured) :as) 'response))
                   (should-not (plist-get (nth 2 captured) :decode))
                   (should-not (nth 3 captured))
                   (should (equal (plist-get response :status) 200))
                   (should (equal (with-temp-buffer
                                    (set-buffer-multibyte nil)
                                    (insert-file-contents-literally destination)
                                    (buffer-string))
                                  (string-as-unibyte "\0\377bytes")))))
      (delete-file destination)))
  (let ((destination (make-temp-file "jaunder-media-")))
    (unwind-protect
        (let ((failure
               (should-error
                (cl-letf (((symbol-function 'plz)
                           (lambda (&rest _)
                             (signal 'plz-curl-error
                                     (list (make-plz-error :message "offline"))))))
                         (jaunder--pull-media-get "https://example.test/media" destination))
                :type 'plz-curl-error)))
          (should
           (equal
            (plz-error-message (seq-find #'plz-error-p (cdr failure)))
            "offline")))
      (delete-file destination))))

(ert-deftest jaunder-pull-media-anonymous-get-returns-direct-redirect-status ()
  ;; plz 0.9.1 signals non-2xx responses; a carried response remains evidence.
  (let ((destination (make-temp-file "jaunder-media-")))
    (unwind-protect
        (cl-letf (((symbol-function 'plz)
                   (lambda (&rest _)
                     (signal 'plz-http-error
                             (list (make-plz-error
                                    :response (make-plz-response
                                               :status 302 :headers nil :body "")))))))
                 (should (= 302 (plist-get
                                 (jaunder--pull-media-get "https://example.test/media" destination)
                                 :status))))
      (when (file-exists-p destination) (delete-file destination)))))

(ert-deftest jaunder-pull-media-materialization-rejects-response-trust-failures ()
  ;; Every trust-chain failure is loud rather than becoming an absent reference.
  (dolist (case
           `((missing-instance "jaunder pull media: media response has invalid instance identity"
                               (:headers (("etag" . "\"sha256-%s\""))))
             (duplicate-instance "jaunder pull media: media response has invalid instance identity"
                                 (:headers (("x-jaunder-instance" . ,jaunder-pull-media-test--instance)
                                            ("x-jaunder-instance" . ,jaunder-pull-media-test--instance)
                                            ("etag" . "\"sha256-%s\""))))
             (malformed-instance "jaunder pull media: media response has invalid instance identity"
                                 (:headers (("x-jaunder-instance" . "not-a-uuid") ("etag" . "\"sha256-%s\""))))
             (mismatched-instance "jaunder pull media: media response has invalid instance identity"
                                  (:headers (("x-jaunder-instance" . "123e4567-e89b-12d3-a456-426614174001")
                                             ("etag" . "\"sha256-%s\""))))
             (missing-etag "jaunder pull media: media response ETag disagrees with URL hash"
                           (:headers (("x-jaunder-instance" . ,jaunder-pull-media-test--instance))))
             (malformed-etag "jaunder pull media: media response ETag disagrees with URL hash"
                             (:headers (("x-jaunder-instance" . ,jaunder-pull-media-test--instance)
                                        ("etag" . "W/\"sha256-%s\""))))
             (mismatched-etag "jaunder pull media: media response ETag disagrees with URL hash"
                              (:headers (("x-jaunder-instance" . ,jaunder-pull-media-test--instance)
                                         ("etag" . "\"sha256-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\""))))))
    (let* ((bytes (string-as-unibyte "trusted bytes"))
           (hash (secure-hash 'sha256 bytes))
           (expected (nth 1 case))
           (metadata (nth 2 case)))
      (jaunder-pull-media-test--with-root root
                                          (let ((plan (jaunder-pull-media-test--materialization-plan hash "a.bin"))
                                                message)
                                            (condition-case err
                                                (cl-letf (((symbol-function 'jaunder--pull-media-get)
                                                           (lambda (_url file)
                                                             (jaunder-pull-media-test--write-bytes file bytes)
                                                             (list :status 200
                                                                   :headers (mapcar (lambda (header)
                                                                                      (cons (car header) (format (cdr header) hash)))
                                                                                    (plist-get metadata :headers))))))
                                                         (jaunder--pull-media-materialize root jaunder-pull-media-test--instance plan))
                                              (error (setq message (error-message-string err))))
                                            (should (equal message expected)))))))

(ert-deftest jaunder-pull-media-materialization-rejects-status-and-body-hash-mismatches ()
  (dolist (case '((404 "body" "jaunder pull media: expected direct 200, got 404")
                  (200 "different body" "jaunder pull media: downloaded bytes disagree with URL hash")))
    (let* ((expected (secure-hash 'sha256 (string-as-unibyte "expected body")))
           (actual (string-as-unibyte (nth 1 case)))
           message)
      (jaunder-pull-media-test--with-root root
                                          (condition-case err
                                              (cl-letf (((symbol-function 'jaunder--pull-media-get)
                                                         (lambda (_url file)
                                                           (jaunder-pull-media-test--write-bytes file actual)
                                                           (list :status (car case)
                                                                 :headers (plist-get
                                                                           (jaunder-pull-media-test--response
                                                                            jaunder-pull-media-test--instance expected)
                                                                           :headers)))))
                                                       (jaunder--pull-media-materialize
                                                        root jaunder-pull-media-test--instance
                                                        (jaunder-pull-media-test--materialization-plan expected "a.bin")))
                                            (error (setq message (error-message-string err))))
                                          (should (equal message (nth 2 case)))))))

(ert-deftest jaunder-pull-media-materialization-reuses-verified-copy-without-get ()
  (let* ((bytes (string-as-unibyte "already local"))
         (hash (secure-hash 'sha256 bytes)))
    (jaunder-pull-media-test--with-root root
                                        (let ((target (expand-file-name (format "local-media/%s/a.bin" hash) root)))
                                          (make-directory (file-name-directory target) t)
                                          (jaunder-pull-media-test--write-bytes target bytes)
                                          (cl-letf (((symbol-function 'jaunder--pull-media-get)
                                                     (lambda (&rest _) (error "must not fetch verified copy"))))
                                                   (jaunder--pull-media-materialize
                                                    root jaunder-pull-media-test--instance
                                                    (jaunder-pull-media-test--materialization-plan hash "a.bin")))))))

(ert-deftest jaunder-pull-media-materialization-rejects-corrupt-existing-copy-and-unsafe-paths ()
  (let* ((bytes (string-as-unibyte "expected"))
         (hash (secure-hash 'sha256 bytes)))
    (jaunder-pull-media-test--with-root root
                                        (let ((target (expand-file-name (format "local-media/%s/a.bin" hash) root)))
                                          (make-directory (file-name-directory target) t)
                                          (jaunder-pull-media-test--write-bytes target (string-as-unibyte "corrupt"))
                                          (let ((failure (should-error
                                                          (jaunder--pull-media-materialize
                                                           root jaunder-pull-media-test--instance
                                                           (jaunder-pull-media-test--materialization-plan hash "a.bin")))))
                                            (should (equal (error-message-string failure)
                                                           (format "jaunder pull media: existing copy is unsafe or corrupt: %s" target))))))
    (jaunder-pull-media-test--with-root root
                                        (let ((path (expand-file-name "local-media" root)))
                                          (make-symbolic-link "/tmp" path)
                                          (let ((failure (should-error
                                                          (jaunder--pull-media-materialize
                                                           root jaunder-pull-media-test--instance
                                                           (jaunder-pull-media-test--materialization-plan hash "a.bin")))))
                                            (should (equal (error-message-string failure)
                                                           (format "jaunder pull media: refusing symlink directory: %s" path))))))))

(ert-deftest jaunder-pull-media-materialization-deduplicates-targets-and-stages-before-install ()
  (let* ((bytes-a (string-as-unibyte "first"))
         (hash-a (secure-hash 'sha256 bytes-a))
         (hash-b (secure-hash 'sha256 (string-as-unibyte "second")))
         (references
          (list (jaunder--make-pull-media-reference :url "https://one" :hash hash-a :leaf "a.bin")
                (jaunder--make-pull-media-reference :url "https://two" :hash hash-a :leaf "a.bin")
                (jaunder--make-pull-media-reference :url "https://three" :hash hash-b :leaf "b.bin")))
         (calls 0))
    (jaunder-pull-media-test--with-root root
                                        (let ((failure
                                               (should-error
                                                (cl-letf (((symbol-function 'jaunder--pull-media-get)
                                                           (lambda (_url file)
                                                             (setq calls (1+ calls))
                                                             (jaunder-pull-media-test--write-bytes
                                                              file (if (= calls 1) bytes-a (string-as-unibyte "wrong")))
                                                             (jaunder-pull-media-test--response
                                                              jaunder-pull-media-test--instance
                                                              (if (= calls 1) hash-a hash-b)))))
                                                         (jaunder--pull-media-materialize
                                                          root jaunder-pull-media-test--instance
                                                          (jaunder-pull-media-test--materialization-plan hash-a "a.bin" references))))))
                                          (should (equal (error-message-string failure)
                                                         "jaunder pull media: downloaded bytes disagree with URL hash"))
                                          (should (= calls 2))
                                          ;; The first verified download was only staged: no partial Local Media Copy.
                                          (should-not (file-exists-p
                                                       (expand-file-name (format "local-media/%s/a.bin" hash-a) root)))))))

(ert-deftest jaunder-pull-media-materialization-rejects-unsafe-leaves-and-non-directory-components ()
  (let ((hash (secure-hash 'sha256 (string-as-unibyte "bytes"))))
    (jaunder-pull-media-test--with-root root
                                        (let ((failure (should-error
                                                        (jaunder--pull-media-materialize
                                                         root jaunder-pull-media-test--instance
                                                         (jaunder-pull-media-test--materialization-plan hash "../escape")))))
                                          (should (equal (error-message-string failure)
                                                         "jaunder pull media: invalid decoded filename: \"../escape\""))))
    (jaunder-pull-media-test--with-root root
                                        (let ((path (expand-file-name "local-media" root)))
                                          (jaunder-pull-media-test--write-bytes path (string-as-unibyte "not a directory"))
                                          (let ((failure (should-error
                                                          (jaunder--pull-media-materialize
                                                           root jaunder-pull-media-test--instance
                                                           (jaunder-pull-media-test--materialization-plan hash "a.bin")))))
                                            (should (equal (error-message-string failure)
                                                           (format "jaunder pull media: non-directory path component: %s" path))))))
    (jaunder-pull-media-test--with-root root
                                        (cl-letf (((symbol-function 'file-writable-p) (lambda (&rest _) nil)))
                                                 (let ((failure (should-error
                                                                 (jaunder--pull-media-materialize
                                                                  root jaunder-pull-media-test--instance
                                                                  (jaunder-pull-media-test--materialization-plan hash "a.bin")))))
                                                   (should (equal (error-message-string failure)
                                                                  (format "jaunder pull media: unwritable directory: %s"
                                                                          (directory-file-name root)))))))))

(ert-deftest jaunder-pull-media-materialization-never-overwrites-a-verified-install-race ()
  (let* ((bytes (string-as-unibyte "race winner"))
         (hash (secure-hash 'sha256 bytes)))
    (jaunder-pull-media-test--with-root root
                                        (let ((target (expand-file-name (format "local-media/%s/a.bin" hash) root)))
                                          (cl-letf (((symbol-function 'jaunder--pull-media-get)
                                                     (lambda (_url file)
                                                       (jaunder-pull-media-test--write-bytes file bytes)
                                                       (jaunder-pull-media-test--response
                                                        jaunder-pull-media-test--instance hash)))
                                                    ((symbol-function 'rename-file)
                                                     (lambda (_from to &optional _ok)
                                                       ;; Model another process winning the atomic no-overwrite race.
                                                       (jaunder-pull-media-test--write-bytes to bytes)
                                                       (signal 'file-already-exists '("already installed")))))
                                                   (jaunder--pull-media-materialize
                                                    root jaunder-pull-media-test--instance
                                                    (jaunder-pull-media-test--materialization-plan hash "a.bin"))
                                                   (should (jaunder--pull-media-verified-file-p target hash)))))))

(ert-deftest jaunder-pull-media-revalidates-parents-immediately-before-install ()
  ;; A digest directory can be swapped after staging; the final mutation must
  ;; fail before rename rather than following the new symlink.
  (let* ((bytes (string-as-unibyte "swap"))
         (hash (secure-hash 'sha256 bytes)))
    (jaunder-pull-media-test--with-root root
                                        (let ((outside (make-temp-file "jaunder-outside-" t))
                                              (checks 0)
                                              (real-target (symbol-function 'jaunder--pull-media-target-path)))
                                          (unwind-protect
                                              (cl-letf (((symbol-function 'jaunder--pull-media-get)
                                                         (lambda (_url file)
                                                           (jaunder-pull-media-test--write-bytes file bytes)
                                                           (jaunder-pull-media-test--response
                                                            jaunder-pull-media-test--instance hash)))
                                                        ((symbol-function 'jaunder--pull-media-target-path)
                                                         (lambda (configured-root planned-hash leaf)
                                                           (setq checks (1+ checks))
                                                           (when (= checks 2)
                                                             (let ((digest (expand-file-name
                                                                            (concat "local-media/" hash) root)))
                                                               (delete-directory digest t)
                                                               (make-symbolic-link outside digest)))
                                                           (funcall real-target configured-root planned-hash leaf))))
                                                       (let ((failure
                                                              (should-error
                                                               (jaunder--pull-media-materialize
                                                                root jaunder-pull-media-test--instance
                                                                (jaunder-pull-media-test--materialization-plan hash "a.bin")))))
                                                         (should (equal (error-message-string failure)
                                                                        (format "jaunder pull media: refusing symlink directory: %s"
                                                                                (expand-file-name
                                                                                 (concat "local-media/" hash) root))))))
                                            (delete-directory outside t))))))

(ert-deftest jaunder-pull-media-cleanup-warning-preserves-primary-acquisition-error ()
  (let* ((hash (secure-hash 'sha256 (string-as-unibyte "cleanup")))
         (cleanup nil)
         warning
         primary)
    (jaunder-pull-media-test--with-root root
                                        (let ((real-delete (symbol-function 'delete-file)))
                                          (cl-letf (((symbol-function 'jaunder--pull-media-get)
                                                     (lambda (_url temporary)
                                                       (jaunder-pull-media-test--write-bytes temporary "partial")
                                                       (setq cleanup t)
                                                       (error "primary acquisition failure")))
                                                    ((symbol-function 'delete-file)
                                                     (lambda (path &optional trash)
                                                       (if cleanup (error "cleanup race") (funcall real-delete path trash))))
                                                    ((symbol-function 'jaunder--warn)
                                                     (lambda (format-string &rest args)
                                                       (setq warning (apply #'format format-string args)))))
                                                   (condition-case err
                                                       (jaunder--pull-media-materialize
                                                        root jaunder-pull-media-test--instance
                                                        (jaunder-pull-media-test--materialization-plan hash "a.bin"))
                                                     (error (setq primary (error-message-string err))))
                                                   (should (equal primary "primary acquisition failure"))
                                                   (should (string-match-p "could not remove pulled-media temporary" warning)))))))

(ert-deftest jaunder-pull-media-materialization-keeps-installed-copy-and-cleans-temporaries ()
  (let* ((bytes-a (string-as-unibyte "first"))
         (bytes-b (string-as-unibyte "second"))
         (hash-a (secure-hash 'sha256 bytes-a))
         (hash-b (secure-hash 'sha256 bytes-b))
         (references
          (list (jaunder--make-pull-media-reference :url "https://one" :hash hash-a :leaf "a.bin")
                (jaunder--make-pull-media-reference :url "https://two" :hash hash-b :leaf "b.bin"))))
    (jaunder-pull-media-test--with-root root
                                        (let ((calls 0)
                                              (rename-calls 0)
                                              (original-rename (symbol-function 'rename-file)))
                                          (let ((failure
                                                 (should-error
                                                  (cl-letf (((symbol-function 'jaunder--pull-media-get)
                                                             (lambda (_url file)
                                                               (setq calls (1+ calls))
                                                               (jaunder-pull-media-test--write-bytes
                                                                file (if (= calls 1) bytes-a bytes-b))
                                                               (jaunder-pull-media-test--response
                                                                jaunder-pull-media-test--instance
                                                                (if (= calls 1) hash-a hash-b))))
                                                            ((symbol-function 'rename-file)
                                                             (lambda (from to &optional _ok)
                                                               (setq rename-calls (1+ rename-calls))
                                                               (if (= rename-calls 2)
                                                                   (signal 'file-error '("install race"))
                                                                 (funcall original-rename from to nil)))))
                                                           (jaunder--pull-media-materialize
                                                            root jaunder-pull-media-test--instance
                                                            (jaunder-pull-media-test--materialization-plan
                                                             hash-a "a.bin" references))))))
                                            (should (equal (error-message-string failure) "install race")))
                                          (should (= rename-calls 2))
                                          (should (or (file-exists-p (expand-file-name (format "local-media/%s/a.bin" hash-a) root))
                                                      (file-exists-p (expand-file-name (format "local-media/%s/b.bin" hash-b) root))))
                                          (should-not (directory-files-recursively root "\\.jaunder-media-" nil))))))
;;; jaunder-pull-media-test.el ends here
