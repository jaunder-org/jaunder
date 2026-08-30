;;; jaunder-pull-media.el --- Pulled-media localization and verified copies -*- lexical-binding: t; -*-

;; Copyright (C) 2026 Jaunder contributors

;;; Commentary:
;; Plan native-source media localization, then fetch and materialize verified
;; Local Media Copies.  The server URL is the authority for content hash and
;; canonical filename; public-media requests are anonymous and direct.

;;; Code:

(require 'cl-lib) ;; cov:ignore: load-time dependency declaration has no Edebug execution stop
(require 'url-parse) ;; cov:ignore: load-time dependency declaration has no Edebug execution stop
(require 'url-util) ;; cov:ignore: load-time dependency declaration has no Edebug execution stop
(require 'cmark) ;; cov:ignore: load-time dependency declaration has no Edebug execution stop
(require 'plz) ;; cov:ignore: load-time dependency declaration has no Edebug execution stop
(require 'org) ;; cov:ignore: load-time dependency declaration has no Edebug execution stop
(require 'org-element) ;; cov:ignore: load-time dependency declaration has no Edebug execution stop
(require 'jaunder-warn) ;; cov:ignore: load-time dependency declaration has no Edebug execution stop
(cl-defstruct (jaunder-pull-media-reference ;; cov:ignore: structure declaration has no Edebug execution stop
               (:constructor jaunder--make-pull-media-reference))
              "One immutable local-media acquisition and its native replacements."
              url hash leaf target replacements)

(cl-defstruct (jaunder-pull-media-plan ;; cov:ignore: structure declaration has no Edebug execution stop
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
      (if (equal (downcase (url-type url)) "https") 443 80))) ;; cov:ignore: Edebug omits the scheme-default branch covered by jaunder-pull-media-markdown-scanner-covers-delimiter-edge-cases

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
  (let ((case-fold-search nil)
        (candidate (condition-case nil
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
          (error "jaunder pull media: malformed canonical media URL: %s" url)))))))

(defun jaunder--pull-media-target (format hash filename fragment)
  "Return FORMAT's native target for HASH/FILENAME plus original FRAGMENT."
  (let ((path (format "local-media/%s/%s" hash filename)))
    (concat (if (equal format "org") (concat "file:" path) path)
            (or fragment ""))))

(defun jaunder--pull-media-add-candidate
    (table format origin source start end
           &optional replacement-start replacement-end label)
  "Record SOURCE's URL slice START through END in TABLE when eligible.

TABLE maps canonical URLs without fragments to mutable reference accumulators.
REPLACEMENT-START and REPLACEMENT-END may widen the replaced syntax around the
URL.  LABEL requests an explicit Markdown link preserving that displayed text."
  (let* ((raw (substring source start end))
         (split (string-search "#" raw))
         (url (if split (substring raw 0 split) raw))
         (fragment (and split (substring raw split)))
         (parts (jaunder--pull-media-url-parts url origin)))
    (when parts
      (let* ((hash (nth 0 parts))
             (leaf (nth 1 parts))
             ;; Recover the canonical terminal segment without normalizing the URL.
             (encoded
              (car
               (last
                (split-string
                 (url-filename (url-generic-parse-url url)) "/" t))))
             (key url)
             (reference (gethash key table)))
        (unless reference
          (setq reference
                (list hash leaf
                      (jaunder--pull-media-target
                       format hash encoded nil)
                      nil))
          (puthash key reference table))
        (setf
         (nth 3 reference)
         (cons
          (list (or replacement-start start)
                (or replacement-end end)
                fragment label)
          (nth 3 reference)))))))

(defun jaunder--pull-media-org-candidates (table format origin body)
  "Collect actual Org link destinations from BODY."
  (with-temp-buffer
    (insert body)
    (org-mode)
    (dolist (link (org-element-map (org-element-parse-buffer) 'link #'identity))
      (let* ((raw-link (org-element-property :raw-link link))
             (element-start (1- (org-element-property :begin link)))
             (element-end (1- (org-element-property :end link)))
             (offset
              (string-search
               raw-link (substring body element-start element-end))))
        (when offset
          (let ((start (+ element-start offset)))
            (jaunder--pull-media-add-candidate
             table format origin body start (+ start (length raw-link))))))))
  )

(defun jaunder--pull-media-markdown-escaped-p (body position)
  "Return non-nil when the character before POSITION escapes it."
  (let ((cursor (1- position))
        (slashes 0))
    (while (and (>= cursor 0) (= (aref body cursor) ?\\))
      (setq slashes (1+ slashes)
            cursor (1- cursor)))
    (= 1 (% slashes 2))))

(defun jaunder--pull-media-markdown-reference (parser body start end)
  "Return cmark's normalized label and selected destination for BODY's label.

The public AST deliberately discards reference-definition identity.  This is
the one narrow cmark-el compatibility seam retained to map that parser-owned
choice back to immutable source bytes."
  (let* ((label (substring body start end))
         (normalized (cmark--normalizeReference label))
         (link (gethash normalized (cmark-Parser-refmap parser))))
    (list normalized (and link (cmark--link-destination link)))))

(defun jaunder--pull-media-markdown-run-end (body position character limit)
  "Return the first index after CHARACTER's run at POSITION before LIMIT."
  (while (and (< position limit) (= (aref body position) character))
    (setq position (1+ position)))
  position)

(defun jaunder--pull-media-markdown-code-end (body position width limit)
  "Return the end of a WIDTH-backtick code span after POSITION, or nil."
  (let (end)
    (while (and (< position limit) (not end))
      (if (= (aref body position) ?`)
          (let ((run-end
                 (jaunder--pull-media-markdown-run-end
                  body position ?` limit)))
            (if (= (- run-end position) width)
                (setq end run-end)
              (setq position run-end)))
        (setq position (1+ position))))
    end))

(defun jaunder--pull-media-markdown-label-end (body position limit)
  "Return BODY's balanced, unescaped label closer before LIMIT, or nil."
  (let ((cursor position)
        (depth 1)
        end)
    (while (and (< cursor limit) (not end))
      (let ((character (aref body cursor)))
        (cond
         ((and (= character ?\\) (< (1+ cursor) limit))
          (setq cursor (+ cursor 2)))
         ((= character ?`)
          (let* ((run-end
                  (jaunder--pull-media-markdown-run-end
                   body cursor ?` limit))
                 (close
                  (jaunder--pull-media-markdown-code-end
                   body run-end (- run-end cursor) limit)))
            (setq cursor (or close run-end))))
         ((= character ?\[)
          (setq depth (1+ depth)
                cursor (1+ cursor)))
         ((= character ?\])
          (setq depth (1- depth))
          (if (= depth 0)
              (setq end cursor)
            (setq cursor (1+ cursor))))
         (t
          (setq cursor (1+ cursor))))))
    end))

(defun jaunder--pull-media-markdown-spnl (body position limit)
  "Consume cmark's spaces/tabs and at most one line ending from POSITION."
  (while (and (< position limit) (memq (aref body position) '(?\s ?\t)))
    (setq position (1+ position)))
  (when (and (< position limit) (memq (aref body position) '(?\n ?\r)))
    (setq position
          (if (and (= (aref body position) ?\r)
                   (< (1+ position) limit)
                   (= (aref body (1+ position)) ?\n))
              (+ position 2)
            (1+ position)))
    (while (and (< position limit) (memq (aref body position) '(?\s ?\t)))
      (setq position (1+ position))))
  position)

(defun jaunder--pull-media-markdown-destination (body position closing limit)
  "Return (START END AFTER) for a complete Markdown destination before LIMIT.
CLOSING is the required closing delimiter.  Return nil for malformed text."
  (setq position (jaunder--pull-media-markdown-spnl body position limit))
  (let ((start position)
        end)
    (if (and (< position limit) (= (aref body position) ?<))
        (progn
          (setq start (1+ position)
                position start)
          (while (and (< position limit)
                      (/= (aref body position) ?>)
                      (not (memq (aref body position) '(?\n ?\r))))
            (setq position (1+ position)))
          (when (and (< position limit) (> position start))
            (setq end position
                  position (1+ position))))
      (let ((depth 0))
        (while (and (< position limit)
                    (or (> depth 0)
                        (not (memq (aref body position)
                                   (list ?\s ?\t ?\n ?\r closing)))))
          (cond
           ((and (= (aref body position) ?\\) (< (1+ position) limit))
            (setq position (+ position 2)))
           ((= (aref body position) ?\()
            (setq depth (1+ depth)
                  position (1+ position)))
           ((= (aref body position) ?\))
            (setq depth (1- depth)
                  position (1+ position)))
           (t
            (setq position (1+ position)))))
        (when (and (= depth 0) (> position start))
          (setq end position))))
    (when end
      (setq position (jaunder--pull-media-markdown-spnl body position limit))
      (let ((title-valid t))
        (when (and (< position limit)
                   (memq (aref body position) '(?\" ?' ?\()))
          (let* ((opener (aref body position))
                 (title-close (if (= opener ?\() ?\) opener))
                 title-end)
            (setq position (1+ position))
            (while (and (< position limit) (not title-end)
                        (not (memq (aref body position) '(?\n ?\r))))
              (cond
               ((and (= (aref body position) ?\\) (< (1+ position) limit))
                (setq position (+ position 2)))
               ((= (aref body position) title-close)
                (setq title-end position
                      position (1+ position)))
               (t
                (setq position (1+ position)))))
            (unless title-end
              (setq title-valid nil))))
        (if (= closing ?\n)
            (while (and (< position limit)
                        (memq (aref body position) '(?\s ?\t)))
              (setq position (1+ position)))
          (setq position
                (jaunder--pull-media-markdown-spnl
                 body position limit)))
        (when (and title-valid
                   (or (and (= position limit) (= closing ?\n))
                       (and (< position limit) (= (aref body position) closing))))
          (list start end (if (< position limit) (1+ position) position)))))))

(defun jaunder--pull-media-markdown-line-starts (body)
  "Return a vector mapping one-based lines in BODY to their character offsets."
  (let ((starts (list 0))
        (position 0))
    (while (setq position (string-search "\n" body position))
      (push (1+ position) starts)
      (setq position (1+ position)))
    (vconcat (nreverse starts))))

(defun jaunder--pull-media-markdown-sourcepos-offset (sourcepos starts endpoint)
  "Return SOURCEPOS ENDPOINT's character offset using line STARTS."
  (let* ((point (if endpoint (cdr sourcepos) (car sourcepos)))
         (line (car point))
         (column (cdr point)))
    (+ (aref starts (1- line)) (1- column))))

(defun jaunder--pull-media-markdown-merge-ranges (ranges)
  "Return sorted, coalesced half-open RANGES."
  (let (merged)
    (dolist (range (sort (copy-sequence ranges)
                         (lambda (a b) (< (car a) (car b)))))
      (if (and merged (<= (car range) (cadr (car merged))))
          (setcar merged (list (caar merged)
                               (max (cadr range) (cadr (car merged)))))
        (push range merged)))
    (nreverse merged)))

(defun jaunder--pull-media-markdown-subtract-ranges (ranges excluded)
  "Return RANGES with sorted, merged EXCLUDED ranges removed."
  (let (result)
    (dolist (range ranges)
      (let ((position (car range)))
        (dolist (blocked excluded)
          (when (and (< position (cadr range)) (< (car blocked) (cadr range)))
            (when (< position (car blocked))
              (push (list position (min (car blocked) (cadr range))) result))
            (setq position (max position (cadr blocked)))))
        (when (< position (cadr range))
          (push (list position (cadr range)) result))))
    (nreverse result)))

(defun jaunder--pull-media-markdown-ast (body)
  "Return cmark destinations, parser, eligible ranges, and excluded ranges.

The parser instance retains the refmap that its public AST intentionally loses;
only `jaunder--pull-media-markdown-reference' touches that compatibility seam."
  (let* ((destinations (make-hash-table :test #'equal))
         (eligible nil)
         (excluded nil)
         (starts (jaunder--pull-media-markdown-line-starts body))
         ;; cmark-el mutates parser input; preserve repeat-parse isolation.
         (parser (cmark-create-Parser))
         (walker (cmark-Node-walker
                  (cmark-Parser-parse parser (copy-sequence body))))
         event)
    (while (setq event (cmark-NodeWalker-next walker))
      (when (cmark-event-entering event)
        (let* ((node (cmark-event-node event))
               (type (cmark-Node-type node)))
          (cond
           ((member type '("link" "image"))
            (puthash (cmark-Node-destination node) t destinations))
           ((member type '("paragraph" "heading" "code_block" "html_block"))
            (when-let ((sourcepos (cmark-Node-sourcepos node)))
              (let* ((raw-start
                      (jaunder--pull-media-markdown-sourcepos-offset
                       sourcepos starts nil))
                     (raw-end (1+
                               (jaunder--pull-media-markdown-sourcepos-offset
                                sourcepos starts t)))
                     (start (max 0 (min (length body) raw-start)))
                     (range (list start (max start
                                             (min (length body) raw-end)))))
                (if (member type '("paragraph" "heading"))
                    (push range eligible)
                  (push range excluded)))))))))
    (setq excluded (jaunder--pull-media-markdown-merge-ranges excluded))
    (list destinations parser
          (jaunder--pull-media-markdown-subtract-ranges
           (nreverse eligible) excluded)
          excluded)))

(defun jaunder--pull-media-markdown-add-candidate
    (table format origin body start end destinations)
  "Add BODY's destination span only when cmark parsed it as a link destination."
  (when (gethash (substring body start end) destinations)
    (jaunder--pull-media-add-candidate table format origin body start end)))

(defun jaunder--pull-media-markdown-inline-html-end (body position limit)
  "Return the opaque inline HTML token end at POSITION, or nil.

Quoted attribute values may contain `>'; only an unquoted delimiter ends a tag."
  (cond
   ((and (<= (+ position 4) limit)
         (equal (substring body position (+ position 4)) "<!--"))
    (let ((end (string-search "-->" body (+ position 4))))
      (if end (+ end 3) limit))) ;; cov:ignore: Edebug omits the comment-end branch covered by jaunder-pull-media-markdown-scanner-covers-delimiter-edge-cases
   ((and (< (1+ position) limit)
         (or (member (substring body position (+ position 2)) '("<?" "<!"))
             (memq (get-char-code-property (aref body (1+ position))
                                           'general-category)
                   '(Lu Ll))
             (and (= (aref body (1+ position)) ?/)
                  (< (+ position 2) limit)
                  (memq (get-char-code-property (aref body (+ position 2))
                                                'general-category)
                        '(Lu Ll)))))
    (let ((cursor (1+ position))
          quote)
      (while (and (< cursor limit)
                  (or quote (/= (aref body cursor) ?>)))
        (let ((character (aref body cursor)))
          (cond
           ((and quote (= character quote)) (setq quote nil))
           ((and (not quote) (memq character '(?\" ?'))) (setq quote character))))
        (setq cursor (1+ cursor)))
      (if (< cursor limit) (1+ cursor) limit))))) ;; cov:ignore: Edebug omits the quoted-tag terminator branch covered by jaunder-pull-media-markdown-scanner-covers-delimiter-edge-cases

(defun jaunder--pull-media-markdown-inline-candidates
    (table format origin body range destinations parser)
  "Collect cmark-authorized inline destinations and reference labels in RANGE."
  (let ((position (car range))
        (limit (cadr range))
        uses)
    (while (< position limit)
      (cond
       ((= (aref body position) ?`)
        (let* ((end (jaunder--pull-media-markdown-run-end
                     body position ?` limit))
               (close (jaunder--pull-media-markdown-code-end
                       body end (- end position) limit)))
          (setq position (or close end)))) ;; cov:ignore: Edebug omits the code-span advance covered by jaunder-pull-media-markdown-scanner-covers-delimiter-edge-cases
       ((and (= (aref body position) ?<)
             (not (jaunder--pull-media-markdown-escaped-p body position)))
        (let ((end (cl-position ?> body :start (1+ position) :end limit))
              (html-end
               (jaunder--pull-media-markdown-inline-html-end
                body position limit)))
          (cond
           ((and end
                 (not
                  (string-match-p
                   "[ \t\r\n]" (substring body (1+ position) end)))
                 (gethash
                  (substring body (1+ position) end) destinations))
            (jaunder--pull-media-add-candidate
             table format origin body (1+ position) end
             position (1+ end) (substring body (1+ position) end))
            (setq position (1+ end)))
           (html-end
            (setq position html-end))
           (t
            (setq position (1+ position))))))
       ((and (= (aref body position) ?\[)
             (not (jaunder--pull-media-markdown-escaped-p body position)))
        (let ((label-end
               (jaunder--pull-media-markdown-label-end body (1+ position) limit)))
          (if (not label-end)
              (setq position (1+ position))
            (let ((cursor (1+ label-end)))
              (cond
               ((and (< cursor limit) (= (aref body cursor) ?\x28))
                (let ((destination
                       (jaunder--pull-media-markdown-destination
                        body (1+ cursor) ?\) limit)))
                  (if destination
                      (progn
                        (jaunder--pull-media-markdown-add-candidate
                         table format origin body (car destination)
                         (cadr destination) destinations)
                        (setq position (nth 2 destination)))
                    (setq position cursor))))
               ((and (< cursor limit) (= (aref body cursor) ?\[))
                (let ((reference-end
                       (jaunder--pull-media-markdown-label-end
                        body (1+ cursor) limit)))
                  (if reference-end
                      (progn
                        (push (car
                               (jaunder--pull-media-markdown-reference
                                parser body
                                (if (= reference-end (1+ cursor))
                                    position
                                  cursor)
                                (1+ reference-end)))
                              uses)
                        (setq position (1+ reference-end)))
                    (setq position cursor))))
               (t
                (push (car (jaunder--pull-media-markdown-reference
                            parser body position (1+ label-end)))
                      uses)
                (setq position cursor))))))
        )
       (t
        (setq position (1+ position)))))
    uses))

(defun jaunder--pull-media-markdown-definitions (parser body excluded)
  "Return source-order definition spans outside sorted EXCLUDED ranges."
  (let ((position 0)
        (limit (length body))
        (blocked excluded)
        definitions)
    (while (< position limit)
      (while (and blocked
                  (<= (cadr (car blocked)) position))
        (setq blocked (cdr blocked)))
      (let* ((line-end
              (or (string-search "\n" body position) limit))
             (next-line
              (if (= line-end limit) limit (1+ line-end)))
             (next-line-end
              (if (= next-line limit)
                  limit
                (or (string-search "\n" body next-line) limit)))
             (next-position next-line))
        (unless
            (and blocked
                 (< (car (car blocked)) line-end)
                 (< position (cadr (car blocked))))
          (let ((cursor position)
                (indent 0)
                (scanning t))
            (while (and (< cursor line-end)
                        (= (aref body cursor) ?\s)
                        (< indent 4))
              (setq cursor (1+ cursor)
                    indent (1+ indent)))
            (while scanning
              (cond
               ((and (< cursor line-end)
                     (= (aref body cursor) ?>))
                (setq cursor (1+ cursor))
                (when (and (< cursor line-end)
                           (memq (aref body cursor) '(?\s ?\t)))
                  (setq cursor (1+ cursor))))
               ((and (< (1+ cursor) line-end)
                     (memq (aref body cursor) '(?- ?+ ?*))
                     (memq (aref body (1+ cursor)) '(?\s ?\t)))
                (setq cursor (+ cursor 2)))
               (t
                (setq scanning nil))))
            (when (and (< cursor line-end)
                       (= (aref body cursor) ?\[))
              (let ((label-end
                     (jaunder--pull-media-markdown-label-end
                      body (1+ cursor) line-end)))
                (when (and label-end
                           (< (1+ label-end) line-end)
                           (= (aref body (1+ label-end)) ?:))
                  (let ((destination
                         (jaunder--pull-media-markdown-destination
                          body (+ label-end 2) ?\n next-line-end)))
                    (when destination
                      (let ((reference
                             (jaunder--pull-media-markdown-reference
                              parser body cursor (1+ label-end))))
                        (push
                         (list
                          (car reference)
                          (car destination)
                          (cadr destination)
                          position
                          (max next-line (nth 2 destination))
                          (cadr reference))
                         definitions)
                        (setq next-position
                              (max next-position
                                   (nth 2 destination)))))))))))
        (setq position next-position)))
    (nreverse definitions)))

(defun jaunder--pull-media-markdown-candidates (table format origin body)
  "Map cmark-authorized Markdown destinations back to exact source spans."
  (pcase-let* ((`(,destinations ,parser ,eligible ,excluded)
                (jaunder--pull-media-markdown-ast body))
               (definitions
                (jaunder--pull-media-markdown-definitions parser body excluded))
               (definition-ranges
                (jaunder--pull-media-markdown-merge-ranges
                 (mapcar (lambda (definition)
                           (list (nth 3 definition) (nth 4 definition)))
                         definitions)))
               (uses
                (apply #'append
                       (mapcar
                        (lambda (range)
                          (jaunder--pull-media-markdown-inline-candidates
                           table format origin body range destinations parser))
                        (jaunder--pull-media-markdown-subtract-ranges
                         eligible definition-ranges))))
               (seen-definitions (make-hash-table :test #'equal)))
    (dolist (definition definitions)
      (let ((label (car definition)))
        (unless (gethash label seen-definitions)
          (puthash label t seen-definitions)
          (let ((selected (nth 5 definition)))
            (when
                (and
                 (member label uses)
                 selected
                 (equal
                  selected
                  (substring
                   body (nth 1 definition) (nth 2 definition))))
              (jaunder--pull-media-add-candidate
               table format origin body
               (nth 1 definition) (nth 2 definition)))))))))

(defconst jaunder--pull-media-html-raw-text-tags ;; cov:ignore: constant declaration has no Edebug execution stop
  '("script" "style" "textarea" "title" "xmp" "iframe"
    "noembed" "noframes" "plaintext")
  "HTML elements whose contents cannot contain active child markup.")

(defun jaunder--pull-media-html-attribute-spans (body)
  "Return actual HTML attribute spans in one forward lexical pass over BODY."
  (let ((position 0)
        (limit (length body))
        (attribute-regexp
         (concat
          "[ \t\r\n]+\\([[:alpha:]_:][[:alnum:]_.:-]*\\)"
          "[ \t\r\n]*=[ \t\r\n]*"
          "\\(?:\"\\([^\"]*\\)\"\\|'\\([^']*\\)'"
          "\\|\\([^ \t\r\n>]+\\)\\)"))
        spans)
    (while (< position limit)
      (let ((open (string-search "<" body position)))
        (if (not open)
            (setq position limit)
          (setq position open)
          (cond
           ((and (<= (+ open 4) limit)
                 (equal (substring body open (+ open 4)) "<!--"))
            (let ((end (string-search "-->" body (+ open 4))))
              (setq position (if end (+ end 3) limit))))
           ((or (>= (1+ open) limit)
                (memq (aref body (1+ open)) '(?! ?/ ??)))
            (let ((end (cl-position ?> body :start (+ open 2))))
              (setq position (if end (1+ end) limit)))) ;; cov:ignore: Edebug omits the special-tag skip covered by jaunder-pull-media-html-scanner-covers-lexical-boundaries
           (t
            (let ((cursor (1+ open)))
              (while (and (< cursor limit)
                          (memq (aref body cursor) '(?\s ?\t ?\r ?\n)))
                (setq cursor (1+ cursor)))
              (let ((name-start cursor))
                (while (and (< cursor limit)
                            (not (memq (aref body cursor)
                                       '(?\s ?\t ?\r ?\n ?/ ?>))))
                  (setq cursor (1+ cursor)))
                (let* ((tag
                        (downcase
                         (substring body name-start cursor)))
                       (valid-tag
                        (let ((case-fold-search nil))
                          (string-match-p
                           "\\`[A-Za-z][A-Za-z0-9:-]*\\'" tag)))
                       (tag-end nil)
                       (delimiter nil)
                       (tag-scan cursor))
                  (while (and (< tag-scan limit) (not tag-end))
                    (let ((character (aref body tag-scan)))
                      (cond
                       (delimiter
                        (when (= character delimiter)
                          (setq delimiter nil)))
                       ((memq character '(?\" ?\'))
                        (setq delimiter character))
                       ((= character ?>)
                        (setq tag-end (1+ tag-scan)))))
                    (setq tag-scan (1+ tag-scan)))
                  (if (not tag-end)
                      ;; An incomplete start tag is literal source, not markup.
                      (setq position limit)
                    ;; Script contents and its executable `src' are excluded.
                    ;; Other raw/RCDATA opening tags may still carry ordinary
                    ;; supported attributes such as an iframe `src'.
                    (unless (or (not valid-tag)
                                (equal tag "script"))
                      (let ((case-fold-search t)
                            (attribute-source
                             (substring body cursor tag-end))
                            (scan 0))
                        (while
                            (string-match
                             attribute-regexp attribute-source scan)
                          (let ((start
                                 (or (match-beginning 2)
                                     (match-beginning 3)
                                     (match-beginning 4)))
                                (end
                                 (or (match-end 2)
                                     (match-end 3)
                                     (match-end 4)))
                                (next-scan (match-end 0)))
                            (push
                             (list
                              (downcase
                               (match-string 1 attribute-source))
                              (+ cursor start)
                              (+ cursor end))
                             spans)
                            (setq scan next-scan)))))
                    (setq position tag-end)
                    (when
                        (member tag jaunder--pull-media-html-raw-text-tags)
                      (if (equal tag "plaintext")
                          (setq position limit)
                        (let ((case-fold-search t)
                              (close
                               (string-match
                                (format
                                 "</[ \t\r\n]*%s[ \t\r\n]*>" tag)
                                body position)))
                          (setq position
                                (if close
                                    (match-end 0)
                                  limit))))))))))))))
    (nreverse spans)))

(defun jaunder--pull-media-html-candidates (table format origin body)
  "Collect supported HTML attributes and srcset items from shared spans."
  (dolist (span (jaunder--pull-media-html-attribute-spans body))
    (pcase (car span)
      ((or "src" "href" "poster")
       (jaunder--pull-media-add-candidate table format origin body (nth 1 span) (nth 2 span)))
      ("srcset"
       (let ((value-start (nth 1 span)) (value (substring body (nth 1 span) (nth 2 span))) (cursor 0))
         (while (and (< cursor (length value))
                     (string-match "\\(?:\\`\\|,\\)[ \t\r\n]*\\([^, \t\r\n]+\\)" value cursor))
           (let ((start (match-beginning 1))
                 (end (match-end 1))
                 (next-cursor (match-end 0)))
             (jaunder--pull-media-add-candidate
              table format origin body
              (+ value-start start) (+ value-start end))
             (setq cursor next-cursor))))))))

(defun jaunder--pull-media-plan (format body origin)
  "Return a pure localization plan for FORMAT BODY at configured ORIGIN."
  (unless (member format '("org" "markdown" "html"))
    (error "jaunder pull media: unsupported format %S" format))
  (let ((table (make-hash-table :test #'equal)))
    (pcase format
      ("org" (jaunder--pull-media-org-candidates table format origin body))
      ("markdown" (jaunder--pull-media-markdown-candidates table format origin body))
      ("html" (jaunder--pull-media-html-candidates table format origin body)))
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
                  (mapcar
                   (lambda (replacement)
                     (let ((target
                            (concat
                             (jaunder-pull-media-reference-target reference)
                             (or (nth 2 replacement) ""))))
                       (list
                        (nth 0 replacement) (nth 1 replacement)
                        (if-let ((label (nth 3 replacement)))
                            (format "[%s](%s)" label target)
                          target))))
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

(defconst jaunder--pull-media-instance-id-regexp ;; cov:ignore: constant declaration has no Edebug execution stop
  "\\`[0-9a-f]\\{8\\}-[0-9a-f]\\{4\\}-[0-9a-f]\\{4\\}-[0-9a-f]\\{4\\}-[0-9a-f]\\{12\\}\\'")

(defconst jaunder--pull-media-sha256-regexp "\\`[0-9a-f]\\{64\\}\\'") ;; cov:ignore: constant declaration has no Edebug execution stop

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
  (unless (and (stringp destination)
               (file-exists-p destination)
               (not (file-symlink-p destination))
               (file-regular-p destination))
    (error "jaunder pull media: temporary destination is not a regular file: %S" destination))
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
  (unless (and (stringp root) (file-name-absolute-p root))
    (error "jaunder pull media: configured root is invalid: %S" root))
  (let ((case-fold-search nil))
    (unless (string-match-p jaunder--pull-media-sha256-regexp hash)
      (error "jaunder pull media: invalid planned hash: %S" hash)))
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
  (let ((case-fold-search nil)
        (instances (jaunder--pull-media-header-values response "x-jaunder-instance"))
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
  "Create and retain an exclusive same-filesystem staging file beside TARGET."
  (make-temp-file
   (expand-file-name ".jaunder-media-" (file-name-directory target))))

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
  (let ((case-fold-search nil))
    (unless (and (stringp instance-id)
                 (string-match-p jaunder--pull-media-instance-id-regexp instance-id))
      (error "jaunder pull media: Member instance identity is not canonical")))
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

(provide 'jaunder-pull-media) ;; cov:ignore: feature declaration has no Edebug execution stop
;;; jaunder-pull-media.el ends here
