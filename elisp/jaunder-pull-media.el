;;; jaunder-pull-media.el --- Pure pulled-media localization -*- lexical-binding: t; -*-

;; Copyright (C) 2026 Jaunder contributors

;;; Commentary:
;; Plan and apply native-source media localization without touching transport or
;; the filesystem.  The server URL is the authority for its content hash and
;; canonical filename; this module only admits that already-canonical spelling.

;;; Code:

(require 'cl-lib)
(require 'url-parse)
(require 'url-util)

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
  (condition-case nil
      (let ((candidate (url-generic-parse-url url))
            (configured (url-generic-parse-url origin)))
        (when (and (url-type candidate) (url-host candidate)
                   (not (url-user candidate))
                   (not (url-password candidate))
                   (not (string-search "?" url))
                   (jaunder--pull-media-same-origin-p candidate configured))
          (let ((path (url-filename candidate)))
            (when (string-match
                   "\\`/media/\\(upload\\|cached\\)/\\([0-9a-f][0-9a-f]\\)/\\([0-9a-f][0-9a-f]\\)/\\([0-9a-f]\\{64\\}\\)/\\([^/?#]+\\)\\'"
                   path)
              (let ((p1 (match-string 2 path))
                    (p2 (match-string 3 path))
                    (hash (match-string 4 path))
                    (filename (match-string 5 path)))
                (when (and (equal p1 (substring hash 0 2))
                           (equal p2 (substring hash 2 4)))
                  (let ((leaf (jaunder--pull-media-decode-filename filename)))
                    (when leaf
                      (list hash leaf)))))))))
    (error nil)))

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
  "Collect Org link destination candidates from BODY into TABLE."
  (let ((position 0) (regexp "\\[\\[\\([^]]+\\)\\]\\(?:\\[[^]]*\\]\\)?\\]"))
    (while (string-match regexp body position)
      (let ((destination-start (match-beginning 1))
            (destination-end (match-end 1))
            (next-position (match-end 0)))
        (jaunder--pull-media-add-candidate table format origin body
                                           destination-start destination-end)
        (setq position next-position)))))

(defun jaunder--pull-media-markdown-candidates (table format origin body)
  "Collect Markdown link and image destination candidates from BODY into TABLE."
  (let ((position 0) (regexp "!?\\[[^]]*\\](\\([^()[:space:]]+\\))"))
    (while (string-match regexp body position)
      (let ((destination-start (match-beginning 1))
            (destination-end (match-end 1))
            (next-position (match-end 0)))
        (jaunder--pull-media-add-candidate table format origin body
                                           destination-start destination-end)
        (setq position next-position)))))

(defun jaunder--pull-media-html-attribute-candidates (table format origin body)
  "Collect HTML media-link destination candidates into TABLE.
Quoted and bare src, href, and poster attributes are eligible."
  (dolist (spec
           '(("\\(?:[ \t\r\n]\\)\\(?:src\\|href\\|poster\\)[ \t\r\n]*=[ \t\r\n]*\\(['\\\"]\\)\\([^'\\\"]*\\)\\1" 2)
             ("\\(?:[ \t\r\n]\\)\\(?:src\\|href\\|poster\\)[ \t\r\n]*=[ \t\r\n]*\\([^'\"[:space:]>]+\\)" 1)))
    (let ((position 0)
          (regexp (nth 0 spec))
          (group (nth 1 spec)))
      (while (string-match regexp body position)
        (let ((destination-start (match-beginning group))
              (destination-end (match-end group))
              (next-position (match-end 0)))
          (jaunder--pull-media-add-candidate table format origin body
                                             destination-start destination-end)
          (setq position next-position))))))

(defun jaunder--pull-media-html-srcset-candidates (table format origin body)
  "Collect each comma-delimited HTML srcset destination candidate from BODY."
  (dolist (spec
           '(("\\(?:[ \t\r\n]\\)srcset[ \t\r\n]*=[ \t\r\n]*\\(['\\\"]\\)\\([^'\\\"]*\\)\\1" 2)
             ("\\(?:[ \t\r\n]\\)srcset[ \t\r\n]*=[ \t\r\n]*\\([^'\"[:space:]>]+\\)" 1)))
    (let ((position 0)
          (attribute (nth 0 spec))
          (group (nth 1 spec)))
      (while (string-match attribute body position)
        (let ((value-start (match-beginning group))
              (value (match-string group body))
              (next-position (match-end 0))
              (item-position 0))
          (while (string-match "\\(?:\\`\\|,\\)[ \\t\\r\\n]*\\([^,[:space:]]+\\)" value item-position)
            (let ((destination-start (+ value-start (match-beginning 1)))
                  (destination-end (+ value-start (match-end 1)))
                  (next-item-position (match-end 0)))
              (jaunder--pull-media-add-candidate table format origin body
                                                 destination-start destination-end)
              (setq item-position next-item-position)))
          (setq position next-position))))))

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
  "Apply PLAN's replacements to its native body without altering other bytes."
  (let ((replacements
         (sort (cl-mapcan
                (lambda (reference)
                  (mapcar (lambda (replacement)
                            (list (nth 0 replacement) (nth 1 replacement)
                                  (concat (jaunder-pull-media-reference-target reference)
                                          (or (nth 2 replacement) ""))))
                          (jaunder-pull-media-reference-replacements reference)))
                (jaunder-pull-media-plan-references plan))
               (lambda (a b) (> (car a) (car b)))))
        (body (jaunder-pull-media-plan-body plan)))
    (dolist (replacement replacements body)
      (setq body (concat (substring body 0 (nth 0 replacement))
                         (nth 2 replacement)
                         (substring body (nth 1 replacement)))))))

(provide 'jaunder-pull-media)
;;; jaunder-pull-media.el ends here
