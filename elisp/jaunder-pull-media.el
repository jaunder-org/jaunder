;;; jaunder-pull-media.el --- Pulled-media localization and verified copies -*- lexical-binding: t; -*-

;; Copyright (C) 2026 Jaunder contributors

;;; Commentary:
;; Plan native-source media localization, then fetch and materialize verified
;; Local Media Copies.  The server URL is the authority for content hash and
;; canonical filename; public-media requests are anonymous and direct.

;;; Code:

(require 'cl-lib)
(require 'url-parse)
(require 'url-util)
(require 'seq)
(require 'plz)
(require 'org)
(require 'org-element)
(require 'jaunder-warn)
(cl-defstruct (jaunder-pull-media-reference
               (:constructor jaunder--make-pull-media-reference))
              "One immutable local-media acquisition and its native replacements."
              url hash leaf target replacements)

(cl-defstruct (jaunder-pull-media-plan
               (:constructor jaunder--make-pull-media-plan))
              "Immutable localization plan for one native body."
              format body references)

(defun jaunder--pull-media-control-character-p (character)
  "Return non-nil when CHARACTER is a Unicode control character."
  (eq (get-char-code-property character 'general-category) 'Cc))

(defun jaunder--pull-media-safe-leaf-p (leaf)
  "Return non-nil when LEAF is a safe decoded local filename component."
  (and (stringp leaf)
       (not (string-empty-p leaf))
       (not (member leaf '("." "..")))
       (equal leaf (file-name-nondirectory leaf))
       (not (string-match-p "[\\\\/]" leaf))
       (not (cl-some #'jaunder--pull-media-control-character-p leaf))))

(defun jaunder--pull-media-encode-filename (leaf)
  "Return LEAF in the server's canonical percent-encoded filename spelling."
  (mapconcat
   (lambda (byte)
     (if (or (and (>= byte ?A) (<= byte ?Z))
             (and (>= byte ?a) (<= byte ?z))
             (and (>= byte ?0) (<= byte ?9))
             (memq byte '(?- ?. ?_ ?~)))
         (char-to-string byte)
       (format "%%%02X" byte)))
   (string-to-list (encode-coding-string leaf 'utf-8)) ""))

(defun jaunder--pull-media-decode-filename (encoded)
  "Decode canonical ENCODED filename once, returning nil when it is unsafe."
  (when (and (stringp encoded)
             (string-match-p "\\`\\(?:[[:alnum:]_.~-]\\|%[0-9A-F][0-9A-F]\\)+\\'" encoded))
    (condition-case nil
        (let ((leaf (decode-coding-string (url-unhex-string encoded) 'utf-8)))
          (when (and (jaunder--pull-media-safe-leaf-p leaf)
                     (equal encoded (jaunder--pull-media-encode-filename leaf)))
            leaf))
      (error nil))))

(defun jaunder--pull-media-effective-port (url)
  "Return URL's explicit or scheme-default port as a number."
  (or (url-port url)
      (if (equal (downcase (url-type url)) "https") 443 80)))

(defun jaunder--pull-media-same-origin-p (candidate origin)
  "Return non-nil when CANDIDATE has ORIGIN's normalized HTTP(S) origin."
  (and (member (downcase (or (url-type candidate) "")) '("http" "https"))
       (equal (downcase (or (url-type candidate) ""))
              (downcase (or (url-type origin) "")))
       (equal (downcase (or (url-host candidate) ""))
              (downcase (or (url-host origin) "")))
       (= (jaunder--pull-media-effective-port candidate)
          (jaunder--pull-media-effective-port origin))))

(defun jaunder--pull-media-url-parts (url origin)
  "Return (HASH LEAF) when URL is eligible canonical media at ORIGIN.
Return nil for every non-candidate form."
  (let ((candidate (condition-case nil
                       (url-generic-parse-url url)
                     (error nil)))
        (configured (condition-case nil
                        (url-generic-parse-url origin)
                      (error nil))))
    (when (and (url-type candidate) (url-host candidate)
               (not (url-user candidate))
               (not (url-password candidate))
               (not (string-search "?" url))
               (jaunder--pull-media-same-origin-p candidate configured))
      (let ((path (url-filename candidate)))
        (cond
         ((string-match
           "\\`/media/\\(upload\\|cached\\)/\\([0-9a-f][0-9a-f]\\)/\\([0-9a-f][0-9a-f]\\)/\\([0-9a-f]\\{64\\}\\)/\\([^/?#]+\\)\\'"
           path)
          (let ((p1 (match-string 2 path))
                (p2 (match-string 3 path))
                (hash (match-string 4 path))
                (filename (match-string 5 path)))
            (unless (and (equal p1 (substring hash 0 2))
                         (equal p2 (substring hash 2 4)))
              (error "jaunder pull media: malformed canonical media URL: %s" url))
            (let ((leaf (jaunder--pull-media-decode-filename filename)))
              (unless leaf
                (error "jaunder pull media: malformed canonical media filename: %s" url))
              (list hash leaf))))
         ((string-match-p "\\`/media/\\(?:upload\\|cached\\)/" path)
          ;; This resembles an authoritative media route.  Falling back to
          ;; a remote link here would silently weaken the offline contract.
          (error "jaunder pull media: malformed canonical media URL: %s" url))))))
  )

(defun jaunder--pull-media-target (format hash filename fragment)
  "Return FORMAT's native target for HASH/FILENAME plus original FRAGMENT."
  (let ((path (format "local-media/%s/%s" hash filename)))
    (concat (if (equal format "org") (concat "file:" path) path)
            (or fragment ""))))

(defun jaunder--pull-media-add-candidate (table format origin source start end)
  "Record SOURCE's URL slice START through END in TABLE when eligible.
TABLE maps canonical URLs without fragments to mutable reference accumulators."
  (let* ((raw (substring source start end))
         (split (string-search "#" raw))
         (url (if split (substring raw 0 split) raw))
         (fragment (and split (substring raw split)))
         (parts (jaunder--pull-media-url-parts url origin)))
    (when parts
      (let* ((hash (nth 0 parts))
             (leaf (nth 1 parts))
             ;; Recover the canonical terminal segment without normalizing the URL.
             (encoded (car (last (split-string (url-filename (url-generic-parse-url url)) "/" t))))
             (key url)
             (reference (gethash key table)))
        (unless reference
          (setq reference (list hash leaf
                                (jaunder--pull-media-target format hash encoded nil)
                                nil))
          (puthash key reference table))
        (setf (nth 3 reference)
              (cons (list start end fragment) (nth 3 reference)))))))

(defun jaunder--pull-media-org-candidates (table format origin body)
  "Collect actual Org link destinations from BODY."
  (with-temp-buffer
    (insert body)
    (org-mode)
    (dolist (link (org-element-map (org-element-parse-buffer) 'link #'identity))
      (let* ((raw-link (org-element-property :raw-link link))
             (start (+ (org-element-property :begin link) 1)))
        (jaunder--pull-media-add-candidate
         table format origin body start (+ start (length raw-link)))))))

(defun jaunder--pull-media-markdown-literal-p (body position line-start)
  "Return non-nil when POSITION is literal Markdown after LINE-START in BODY."
  (let* ((line-end (or (string-match "$" body position) (length body)))
         (line (substring body line-start line-end))
         (before (substring body line-start position)))
    (or (string-match-p "\\`\\(?:    \\|\t\\)" line)
        (string-match-p "\\\\\\(?:\\\\\\\\\\)*\\'" before)
        (= 1 (% (cl-count ?` before) 2)))))

(defun jaunder--pull-media-markdown-candidates (table format origin body)
  "Collect Markdown inline and reference-definition destinations from BODY."
  (let ((position 0)
        (fenced nil)
        (inline "!?\\[[^]]*\\][ \t]*([ \t]*\\(?:<\\([^> \t\r\n]+\\)>\\|\\([^() \t\r\n]+\\)\\)")
        (definition "^[ \t]*\\[[^]]+\\]:[ \t]+\\(?:<\\([^> \t\r\n]+\\)>\\|\\([^ \t\r\n]+\\)\\)"))
    (while (and (<= position (length body))
                (string-match "\\(?:^.*$\\)" body position))
      (let ((line-start (match-beginning 0))
            (line-end (match-end 0))
            (line (match-string 0 body)))
        (when (string-match-p "^[ \t]*\\(?:```\\|~~~\\)" line)
          (setq fenced (not fenced)))
        (unless fenced
          (dolist (regexp (list inline definition))
            (let ((scan line-start))
              (while (and (< scan line-end) (string-match regexp body scan))
                (let ((next-scan (match-end 0)))
                  (if (> next-scan line-end)
                      (setq scan line-end)
                    (let ((start (or (match-beginning 1) (match-beginning 2)))
                          (end (or (match-end 1) (match-end 2))))
                      (unless (jaunder--pull-media-markdown-literal-p
                               body start line-start)
                        (jaunder--pull-media-add-candidate
                         table format origin body start end))
                      (setq scan next-scan))))))))
        (setq position
              (if (= line-end (length body)) (1+ line-end) line-end))))))

(defun jaunder--pull-media-html-comment-position-p (body position)
  "Return non-nil when POSITION occurs inside an HTML comment in BODY."
  (let ((prefix (substring body 0 position))
        (starts 0)
        (ends 0)
        (scan 0))
    (while (string-match "<!--" prefix scan)
      (setq starts (1+ starts) scan (match-end 0)))
    (setq scan 0)
    (while (string-match "-->" prefix scan)
      (setq ends (1+ ends) scan (match-end 0)))
    (> starts ends)))

(defun jaunder--pull-media-html-attribute-spans (body)
  "Return actual HTML attribute value spans in BODY.
Comments and script/style raw-text content are skipped.  Names are ASCII
case-insensitive and each result is (NAME START END), with NAME downcased."
  (let ((position 0) spans)
    (while (string-match "<\\([^!/?][^ \t\r\n/>]*\\)\\([^>]*\\)>" body position)
      (let* ((tag (downcase (match-string 1 body)))
             (attributes (match-string 2 body))
             (base (match-beginning 2))
             (next (match-end 0)))
        (unless (or (member tag '("script" "style"))
                    (save-match-data
                      (jaunder--pull-media-html-comment-position-p body (match-beginning 0))))
          (let ((attribute-position 0))
            (while (string-match
                    "\\(?:\\`\\|[ \t\r\n]\\)\\([[:alnum:]:-]+\\)[ \t\r\n]*=[ \t\r\n]*\\(?:\\(['\"]\\)\\([^'\"]*\\)\\2\\|\\([^'\" \t\r\n>]+\\)\\)"
                    attributes attribute-position)
              (let ((name (downcase (match-string 1 attributes)))
                    (start (or (match-beginning 3) (match-beginning 4)))
                    (end (or (match-end 3) (match-end 4))))
                (push (list name (+ base start) (+ base end)) spans)
                (setq attribute-position (match-end 0))))))
        (setq position
              (if (member tag '("script" "style"))
                  (if (string-match (format "</[ \t\r\n]*%s[ \t\r\n]*>" tag) body next)
                      (match-end 0)
                    (length body))
                next))))
    (nreverse spans)))

(defun jaunder--pull-media-html-attribute-candidates (table format origin body)
  "Collect supported HTML link attributes using actual attribute spans."
  (dolist (span (jaunder--pull-media-html-attribute-spans body))
    (when (member (car span) '("src" "href" "poster"))
      (jaunder--pull-media-add-candidate table format origin body
                                         (nth 1 span) (nth 2 span)))))

(defun jaunder--pull-media-html-srcset-candidates (table format origin body)
  "Collect individual srcset destinations from actual HTML attribute spans."
  (dolist (span (jaunder--pull-media-html-attribute-spans body))
    (when (equal (car span) "srcset")
      (let ((value-start (nth 1 span))
            (value (substring body (nth 1 span) (nth 2 span)))
            (item-position 0))
        (while (string-match "\\(?:\\`\\|,\\)[ \t\r\n]*\\([^, \t\r\n]+\\)" value item-position)
          (let ((start (match-beginning 1))
                (end (match-end 1))
                (next-position (match-end 0)))
            (jaunder--pull-media-add-candidate
             table format origin body
             (+ value-start start) (+ value-start end))
            (setq item-position next-position)))))))

(defun jaunder--pull-media-plan (format body origin)
  "Return a pure localization plan for FORMAT BODY at configured ORIGIN."
  (unless (member format '("org" "markdown" "html"))
    (error "jaunder pull media: unsupported format %S" format))
  (let ((table (make-hash-table :test #'equal)))
    (pcase format
      ("org" (jaunder--pull-media-org-candidates table format origin body))
      ("markdown" (jaunder--pull-media-markdown-candidates table format origin body))
      ("html" (jaunder--pull-media-html-attribute-candidates table format origin body)
       (jaunder--pull-media-html-srcset-candidates table format origin body)))
    (let (references)
      (maphash
       (lambda (url value)
         (push (jaunder--make-pull-media-reference
                :url url :hash (nth 0 value) :leaf (nth 1 value)
                :target (nth 2 value) :replacements (nreverse (nth 3 value)))
               references))
       table)
      (jaunder--make-pull-media-plan :format format :body body
                                     :references (sort references
                                                       (lambda (a b)
                                                         (string-lessp
                                                          (jaunder-pull-media-reference-url a)
                                                          (jaunder-pull-media-reference-url b))))))))

(defun jaunder--pull-media-apply-plan (plan)
  "Apply PLAN's replacements in one source-order pass without altering other bytes."
  (let ((replacements
         (sort (cl-mapcan
                (lambda (reference)
                  (mapcar (lambda (replacement)
                            (list (nth 0 replacement) (nth 1 replacement)
                                  (concat (jaunder-pull-media-reference-target reference)
                                          (or (nth 2 replacement) ""))))
                          (jaunder-pull-media-reference-replacements reference)))
                (jaunder-pull-media-plan-references plan))
               (lambda (a b) (< (car a) (car b)))))
        (body (jaunder-pull-media-plan-body plan))
        (position 0)
        pieces)
    (dolist (replacement replacements)
      (push (substring body position (nth 0 replacement)) pieces)
      (push (nth 2 replacement) pieces)
      (setq position (nth 1 replacement)))
    (push (substring body position) pieces)
    (apply #'concat (nreverse pieces))))

(defconst jaunder--pull-media-instance-id-regexp
  "\\`[0-9a-f]\\{8\\}-[0-9a-f]\\{4\\}-[0-9a-f]\\{4\\}-[0-9a-f]\\{4\\}-[0-9a-f]\\{12\\}\\'")

(defconst jaunder--pull-media-sha256-regexp "\\`[0-9a-f]\\{64\\}\\'")

(defun jaunder--pull-media-header-values (response name)
  "Return all RESPONSE header values named NAME, case-insensitively."
  (let ((downcased (downcase name)))
    (cl-loop for (key . value) in (plist-get response :headers)
             when (equal (downcase (format "%s" key)) downcased)
             collect value)))

(defun jaunder--pull-media-write-bytes (bytes destination)
  "Write un-decoded response BYTES to DESTINATION exactly once."
  (let ((coding-system-for-write 'no-conversion))
    (write-region bytes nil destination nil 'silent)))

(defun jaunder--pull-media-get (url destination)
  "Fetch public media URL anonymously into DESTINATION and return its metadata.
The pinned plz 0.9.1 API cannot combine a file response with parsed headers, so
this requests an un-decoded response and writes its body with no coding
conversion.  Binding curl's default arguments removes its redirect option:
public media identity and URL hash are valid only for the direct response."
  (unless (and (stringp destination) (not (file-exists-p destination)))
    (error "jaunder pull media: temporary destination already exists: %S" destination))
  (let ((plz-curl-default-args
         (delete "--location" (copy-sequence plz-curl-default-args))))
    (condition-case err
        (let ((response (plz 'get url :as 'response :decode nil)))
          (jaunder--pull-media-write-bytes (plz-response-body response) destination)
          (list :status (plz-response-status response)
                :headers (plz-response-headers response)))
      (plz-error
       (let* ((plz-error (seq-find #'plz-error-p (cdr err)))
              (response (and plz-error (plz-error-response plz-error))))
         (if response
             (progn
               (jaunder--pull-media-write-bytes (plz-response-body response) destination)
               (list :status (plz-response-status response)
                     :headers (plz-response-headers response)))
           (signal (car err) (cdr err))))))))

(defun jaunder--pull-media-ensure-directory (directory)
  "Return DIRECTORY after refusing symlinks and non-directory components."
  (cond
   ((file-symlink-p directory)
    (error "jaunder pull media: refusing symlink directory: %s" directory))
   ((file-exists-p directory)
    (unless (file-directory-p directory)
      (error "jaunder pull media: non-directory path component: %s" directory)))
   (t
    (make-directory directory)))
  (unless (file-writable-p directory)
    (error "jaunder pull media: unwritable directory: %s" directory))
  directory)

(defun jaunder--pull-media-target-path (root hash leaf)
  "Return ROOT's safe Local Media Copy path for HASH and decoded LEAF."
  (unless (and (stringp root) (file-name-absolute-p (expand-file-name root)))
    (error "jaunder pull media: configured root is invalid: %S" root))
  (unless (string-match-p jaunder--pull-media-sha256-regexp hash)
    (error "jaunder pull media: invalid planned hash: %S" hash))
  (unless (jaunder--pull-media-safe-leaf-p leaf)
    (error "jaunder pull media: invalid decoded filename: %S" leaf))
  (let ((root (directory-file-name (expand-file-name root))))
    (jaunder--pull-media-ensure-directory root)
    (let ((media (expand-file-name "local-media" root)))
      (jaunder--pull-media-ensure-directory media)
      (let ((digest (expand-file-name hash media)))
        (jaunder--pull-media-ensure-directory digest)
        (expand-file-name leaf digest)))))

(defun jaunder--pull-media-file-sha256 (path)
  "Return SHA-256 of PATH's literal bytes without decoding its contents."
  (with-temp-buffer
    (set-buffer-multibyte nil)
    (insert-file-contents-literally path)
    (secure-hash 'sha256 (current-buffer))))

(defun jaunder--pull-media-verified-file-p (path hash)
  "Return non-nil when PATH is a regular file with SHA-256 HASH."
  (and (not (file-symlink-p path))
       (file-regular-p path)
       (equal (jaunder--pull-media-file-sha256 path) hash)))

(defun jaunder--pull-media-require-existing-copy (path hash)
  "Verify existing PATH has HASH, rejecting every unsafe or corrupt reuse."
  (unless (jaunder--pull-media-verified-file-p path hash)
    (error "jaunder pull media: existing copy is unsafe or corrupt: %s" path)))

(defun jaunder--pull-media-validate-response (response instance-id hash temporary)
  "Validate RESPONSE and TEMPORARY against INSTANCE-ID and planned HASH."
  (unless (= (plist-get response :status) 200)
    (error "jaunder pull media: expected direct 200, got %S"
           (plist-get response :status)))
  (let ((instances (jaunder--pull-media-header-values response "x-jaunder-instance"))
        (etags (jaunder--pull-media-header-values response "etag")))
    (unless (and (= (length instances) 1)
                 (string-match-p jaunder--pull-media-instance-id-regexp (car instances))
                 (equal (car instances) instance-id))
      (error "jaunder pull media: media response has invalid instance identity"))
    (unless (and (= (length etags) 1)
                 (string-match
                  (concat "\\`\"sha256-" "\\([0-9a-f]\\{64\\}\\)" "\"\\'") (car etags))
                 (equal (match-string 1 (car etags)) hash))
      (error "jaunder pull media: media response ETag disagrees with URL hash")))
  (unless (equal (jaunder--pull-media-file-sha256 temporary) hash)
    (error "jaunder pull media: downloaded bytes disagree with URL hash")))

(defun jaunder--pull-media-temporary-path (target)
  "Reserve a same-filesystem temporary name beside TARGET without retaining it."
  (let ((temporary (make-temp-file
                    (expand-file-name ".jaunder-media-" (file-name-directory target)))))
    (delete-file temporary)
    temporary))

(defun jaunder--pull-media-clean-temporary (temporary)
  "Attempt to remove TEMPORARY without obscuring the triggering failure."
  (when (file-exists-p temporary)
    (condition-case cleanup-error
        (delete-file temporary)
      (error
       (jaunder--warn "could not remove pulled-media temporary %s: %s"
                      temporary (error-message-string cleanup-error))))))

(defun jaunder--pull-media-materialize (root instance-id plan)
  "Materialize PLAN's verified Local Media Copies under configured ROOT.
Every distinct target is staged and verified before any installation.  Existing
verified copies are reused without a request.  A no-overwrite installation race
may reuse only a byte-for-byte verified concurrent copy."
  (unless (and (stringp instance-id)
               (string-match-p jaunder--pull-media-instance-id-regexp instance-id))
    (error "jaunder pull media: Member instance identity is not canonical"))
  (let ((targets (make-hash-table :test #'equal))
        staged)
    (dolist (reference (jaunder-pull-media-plan-references plan))
      (let* ((hash (jaunder-pull-media-reference-hash reference))
             (leaf (jaunder-pull-media-reference-leaf reference))
             (target (jaunder--pull-media-target-path root hash leaf)))
        (unless (gethash target targets)
          (puthash target reference targets))))
    (unwind-protect
        (progn
          (maphash
           (lambda (target reference)
             (let ((hash (jaunder-pull-media-reference-hash reference)))
               (if (or (file-exists-p target) (file-symlink-p target))
                   (jaunder--pull-media-require-existing-copy target hash)
                 (let ((temporary (jaunder--pull-media-temporary-path target)))
                   (condition-case err
                       (let ((response (jaunder--pull-media-get
                                        (jaunder-pull-media-reference-url reference) temporary)))
                         (jaunder--pull-media-validate-response
                          response instance-id hash temporary)
                         (push (list temporary target hash
                                     (jaunder-pull-media-reference-leaf reference))
                               staged))
                     (error
                      (jaunder--pull-media-clean-temporary temporary)
                      (signal (car err) (cdr err))))))))
           targets)
          (dolist (copy (nreverse staged))
            (pcase-let ((`(,temporary ,target ,hash ,leaf) copy))
              ;; Re-check immediately before mutation: a parent safe during
              ;; staging may have been replaced by a symlink meanwhile.
              (unless (equal target (jaunder--pull-media-target-path root hash leaf))
                (error "jaunder pull media: target changed during installation: %s" target))
              (condition-case err
                  (rename-file temporary target nil)
                (file-already-exists
                 (jaunder--pull-media-require-existing-copy target hash))
                (file-error
                 ;; A concurrent creator may win after the failed no-overwrite rename.
                 (if (or (file-exists-p target) (file-symlink-p target))
                     (jaunder--pull-media-require-existing-copy target hash)
                   (signal (car err) (cdr err)))))))
          nil)
      (dolist (copy staged)
        (jaunder--pull-media-clean-temporary (car copy))))))

(provide 'jaunder-pull-media)
;;; jaunder-pull-media.el ends here
