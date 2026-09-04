;;; jaunder-media-test.el --- ERT suite for jaunder-media -*- lexical-binding: t; -*-

;;; Commentary:
;; Unit tests for the corresponding jaunder package module.

;;; Code:

(require 'ert)
(require 'jaunder)

(defun jaunder-test--collect (org dir)
  "Collect media links from ORG with `default-directory' DIR."
  (with-temp-buffer
    (insert org)
    (org-mode)
    (setq default-directory dir)
    (jaunder--collect-media-links)))

;;; Publish-time warnings (shared idiom) ----------------------------------

(defmacro jaunder-test--capturing-warnings (&rest body)
  "Run BODY with `display-warning' captured; return the list of (TYPE MSG LEVEL).
Lets the warning tests assert on emitted warnings without touching the real
`*Warnings*' buffer."
  (declare (indent 0))
  `(let (jaunder-test--warnings)
     (cl-letf (((symbol-function 'display-warning)
                (lambda (type message &optional level &rest _)
                  (push (list type message level) jaunder-test--warnings))))
              ,@body)
     (nreverse jaunder-test--warnings)))

(ert-deftest jaunder-media-content-type-is-deterministic ()
  (dolist (case '(("a.jpg" . "image/jpeg")
                  ("a.jpeg" . "image/jpeg")
                  ("a.png" . "image/png")
                  ("a.gif" . "image/gif")
                  ("a.webp" . "image/webp")
                  ("a.svg" . "image/svg+xml")
                  ("a.mp3" . "audio/mpeg")
                  ("a.ogg" . "audio/ogg")
                  ("a.oga" . "audio/ogg")
                  ("a.flac" . "audio/flac")
                  ("a.wav" . "audio/wav")
                  ("a.mp4" . "video/mp4")
                  ("a.webm" . "video/webm")
                  ("a.pdf" . "application/pdf")
                  ("A.PDF" . "application/pdf")
                  ("a.unknown" . "application/octet-stream")
                  ("extensionless" . "application/octet-stream")))
    (should (equal (jaunder--media-content-type (car case)) (cdr case)))))

(ert-deftest jaunder-media-collect-file-links-qualify-and-resolve ()
  ;; relative, ./-relative-with-desc, and absolute file: links all qualify.
  (let ((rs (jaunder-test--collect
             (concat "#+TITLE: T\n\nSee [[file:img/a.png]] and [[./b.JPG][alt]]"
                     " and [[file:/abs/c.gif]].\n")
             "/home/u/post/")))
    (should (equal (mapcar (lambda (r) (plist-get r :raw-link)) rs)
                   '("file:img/a.png" "./b.JPG" "file:/abs/c.gif")))
    (should (equal (mapcar (lambda (r) (plist-get r :path)) rs)
                   '("/home/u/post/img/a.png" "/home/u/post/b.JPG" "/abs/c.gif")))
    (should (equal (mapcar (lambda (r) (plist-get r :content-type)) rs)
                   '("image/png" "image/jpeg" "image/gif")))))

(ert-deftest jaunder-media-collects-local-files-and-excludes-non-local-links ()
  ;; Header-region, absolute HTTP, fuzzy, and block-contained links are excluded;
  ;; an arbitrary body-local file remains a candidate.
  (let ((rs (jaunder-test--collect
             (concat "#+DESCRIPTION: [[file:cover.png]]\n"
                     "\n"
                     "abs [[https://x/y.png]] "
                     "fuzzy [[a.png]] "
                     "doc [[file:notes.txt]]\n"
                     "#+begin_src org\n[[file:code.png]]\n#+end_src\n"
                     "#+begin_example\n[[file:ex.png]]\n#+end_example\n")
             "/d/")))
    (should (equal rs
                   '((:raw-link "file:notes.txt"
                                :content-type "application/octet-stream"
                                :path "/d/notes.txt"))))))

(ert-deftest jaunder-media-collect-uses-resolved-path-for-content-type ()
  (cl-letf (((symbol-function 'jaunder--org-body-links)
             (lambda ()
               '((:type "file" :path "opaque" :raw-link "file:opaque"
                        :file "/resolved/document.pdf")))))
           (should
            (equal (jaunder--collect-media-links)
                   '((:raw-link "file:opaque"
                                :content-type "application/pdf"
                                :path "/resolved/document.pdf"))))))

(ert-deftest jaunder-localize-media-aggregates-preflight-failures-before-upload ()
  (let* ((dir (make-temp-file "jt-preflight-" t))
         (missing (expand-file-name "missing.pdf" dir))
         (directory (expand-file-name "directory.pdf" dir))
         (unreadable (expand-file-name "unreadable.pdf" dir))
         (real-file-readable-p (symbol-function 'file-readable-p))
         (upload-calls 0))
    (unwind-protect
        (progn
          (make-directory directory)
          (with-temp-file unreadable (insert "PDF"))
          (cl-letf (((symbol-function 'file-readable-p)
                     (lambda (path)
                       (and (not (equal path unreadable))
                            (funcall real-file-readable-p path))))
                    ((symbol-function 'jaunder--upload-media)
                     (lambda (&rest _)
                       (setq upload-calls (1+ upload-calls)))))
                   (with-temp-buffer
                     (insert (format "#+TITLE: T\n\n[[file:%s]] [[file:%s]] [[file:%s]]\n"
                                     missing directory unreadable))
                     (org-mode)
                     (let* ((body (jaunder-entry-body (jaunder--org->atom)))
                            (err (should-error (jaunder--localize-media body)
                                               :type 'error))
                            (message (error-message-string err)))
                       (should
                        (string-prefix-p
                         "jaunder: media file(s) missing, unreadable, or not regular: "
                         message))
                       (dolist (path (list missing directory unreadable))
                         (should (string-match-p (regexp-quote path) message)))
                       (should (= upload-calls 0))))))
      (delete-directory dir t))))

(ert-deftest jaunder-media-substitute-single-and-desc ()
  (should (equal (jaunder--substitute-media
                  "a [[file:x.png]] b [[./y.png][alt]] c"
                  '("https://h/m/x.png" "https://h/m/y.png"))
                 "a [[https://h/m/x.png]] b [[https://h/m/y.png][alt]] c")))

(ert-deftest jaunder-media-substitute-collision-is-positional ()
  ;; same raw target, different resolved URLs -> each rewritten independently
  (should (equal (jaunder--substitute-media
                  "[[attachment:p.png]] and [[attachment:p.png]]"
                  '("https://h/m/aaa/p.png" "https://h/m/bbb/p.png"))
                 "[[https://h/m/aaa/p.png]] and [[https://h/m/bbb/p.png]]")))

(ert-deftest jaunder-media-substitute-same-file-same-url ()
  ;; one file behind two links -> caller passes the same URL twice; both rewrite
  (should (equal (jaunder--substitute-media
                  "[[file:x.png]] then [[file:x.png]]"
                  '("https://h/m/x.png" "https://h/m/x.png"))
                 "[[https://h/m/x.png]] then [[https://h/m/x.png]]")))

(ert-deftest jaunder-media-substitute-no-links-is-noop ()
  (should (equal (jaunder--substitute-media
                  "plain [[https://x/y.png]] and [[fuzzy-target]] only" nil)
                 "plain [[https://x/y.png]] and [[fuzzy-target]] only")))

(ert-deftest jaunder-upload-media-errors-on-non-2xx ()
  (cl-letf (((symbol-function 'jaunder--http-request)
             (lambda (&rest _) '(:status 500 :body "boom"))))
           (let ((jaunder--active-blog '(:base-url "http://x" :username "alice")))
             (should-error (jaunder--upload-media "/tmp/x.png" "image/png") :type 'error))))

(ert-deftest jaunder-media-link-p-qualifies-file-and-attachment-types ()
  ;; Local-path link type is the eligibility boundary; media type and filesystem
  ;; state are handled after resolution.
  (should (equal
           (mapcar #'jaunder--media-link-p
                   '((:type "file" :path "document.pdf")
                     (:type "attachment" :path "recording.flac")
                     (:type "https" :path "//x/c.png")
                     (:type "file" :path "extensionless")
                     (:type "fuzzy" :path "e.png")))
           '(t t nil t nil))))

(ert-deftest jaunder-localize-media-handles-files-attachments-and-deduplication ()
  ;; Exercise the complete pure localization boundary: resolved paths determine
  ;; MIME, repeated files upload once, descriptions survive, and source stays local.
  (let* ((dir (make-temp-file "jt-localize-" t))
         (attach-dir (expand-file-name "attachments" dir))
         (pdf (expand-file-name "document.pdf" dir))
         (flac (expand-file-name "recording.flac" attach-dir))
         (jaunder-warn-untracked-media nil)
         calls)
    (unwind-protect
        (progn
          (make-directory attach-dir)
          (with-temp-file pdf (insert "PDF"))
          (with-temp-file flac (insert "FLAC"))
          (cl-letf (((symbol-function 'jaunder--upload-media)
                     (lambda (path content-type)
                       (push (list path content-type) calls)
                       (if (equal path pdf)
                           "https://h/media/document.pdf"
                         "https://h/media/recording.flac"))))
                   (with-temp-buffer
                     (setq default-directory dir)
                     (org-mode)
                     (insert
                      (format
                       (concat "#+TITLE: T\n\n"
                               "[[file:document.pdf][download]] and "
                               "[[file:document.pdf][again]]\n"
                               "* Audio\n:PROPERTIES:\n:DIR: %s\n:END:\n\n"
                               "[[attachment:recording.flac][listen]]\n")
                       attach-dir))
                     (let* ((body (jaunder-entry-body (jaunder--org->atom)))
                            (before (buffer-string))
                            (out (jaunder--localize-media body)))
                       (should (= (length calls) 2))
                       (should (member (list pdf "application/pdf") calls))
                       (should (member (list flac "audio/flac") calls))
                       (should
                        (string-match-p
                         (regexp-quote "[[https://h/media/document.pdf][download]]")
                         out))
                       (should
                        (string-match-p
                         (regexp-quote "[[https://h/media/document.pdf][again]]")
                         out))
                       (should
                        (string-match-p
                         (regexp-quote "[[https://h/media/recording.flac][listen]]")
                         out))
                       (should-not (string-match-p "file:document\\.pdf" out))
                       (should-not (string-match-p "attachment:recording\\.flac" out))
                       (should (equal (buffer-string) before))))))
      (delete-directory dir t))))

(ert-deftest jaunder-localize-media-no-candidates-is-noop ()
  (let (called)
    (cl-letf (((symbol-function 'jaunder--upload-media)
               (lambda (&rest _) (setq called t) "u")))
             (with-temp-buffer
               (insert "#+TITLE: T\n\nJust prose, [[https://x/y.png]] absolute.\n")
               (org-mode)
               (let ((body (jaunder-entry-body (jaunder--org->atom))))
                 (should (equal (jaunder--localize-media body) body))
                 (should-not called))))))

;;; #206 — untracked-media warning

(ert-deftest jaunder-warn-untracked-media-one-per-untracked ()
  ;; AC-206a: one committed + one untracked → exactly one warning, the untracked.
  (cl-letf (((symbol-function 'jaunder--git-toplevel) (lambda (_dir) "/repo"))
            ((symbol-function 'jaunder--git-tracked-p)
             (lambda (_top path) (equal path "/repo/a.png"))))
           (let ((warnings (jaunder-test--capturing-warnings
                            (jaunder--warn-untracked-media
                             (list (list :path "/repo/a.png")
                                   (list :path "/repo/b.png"))))))
             (should (= (length warnings) 1))
             (should (eq (nth 0 (car warnings)) 'jaunder))
             (should (string-prefix-p "jaunder: " (nth 1 (car warnings))))
             (should (string-match-p "/repo/b.png" (nth 1 (car warnings)))))))

(ert-deftest jaunder-warn-untracked-media-all-tracked ()
  ;; AC-206d
  (cl-letf (((symbol-function 'jaunder--git-toplevel) (lambda (_dir) "/repo"))
            ((symbol-function 'jaunder--git-tracked-p) (lambda (_top _path) t)))
           (should-not (jaunder-test--capturing-warnings
                        (jaunder--warn-untracked-media (list (list :path "/repo/a.png")))))))

(ert-deftest jaunder-warn-untracked-media-skips-non-repo ()
  ;; AC-206e: no repo (or no git) → skip entirely.
  (cl-letf (((symbol-function 'jaunder--git-toplevel) (lambda (_dir) nil)))
           (should-not (jaunder-test--capturing-warnings
                        (jaunder--warn-untracked-media (list (list :path "/x/a.png")))))))

(ert-deftest jaunder-warn-untracked-media-suppressed ()
  ;; AC-206f
  (cl-letf (((symbol-function 'jaunder--git-toplevel) (lambda (_dir) "/repo"))
            ((symbol-function 'jaunder--git-tracked-p) (lambda (_top _path) nil)))
           (let ((jaunder-warn-untracked-media nil))
             (should-not (jaunder-test--capturing-warnings
                          (jaunder--warn-untracked-media (list (list :path "/repo/a.png"))))))))

(ert-deftest jaunder-warn-untracked-media-dedups ()
  ;; AC-206g: the same untracked path referenced twice warns once.
  (cl-letf (((symbol-function 'jaunder--git-toplevel) (lambda (_dir) "/repo"))
            ((symbol-function 'jaunder--git-tracked-p) (lambda (_top _path) nil)))
           (let ((warnings (jaunder-test--capturing-warnings
                            (jaunder--warn-untracked-media
                             (list (list :path "/repo/a.png")
                                   (list :path "/repo/a.png"))))))
             (should (= (length warnings) 1)))))

(ert-deftest jaunder-git-tracked-p-real-repo ()
  ;; AC-206b/c deterministic: pin git's actual exit code for gitignored and
  ;; outside-tree paths (and toplevel resolution from a subdirectory), rather
  ;; than assuming it.  Staging (git add) suffices for `ls-files --error-unmatch',
  ;; so no commit/identity setup is needed.
  (skip-unless (executable-find "git"))
  ;; Strip inherited GIT_* vars (a pre-commit/CI hook exports GIT_DIR /
  ;; GIT_WORK_TREE) so the temp repo's git subprocesses stay hermetic and do not
  ;; resolve against the ambient work tree.
  (let* ((process-environment
          (seq-remove (lambda (v) (string-prefix-p "GIT_" v))
                      process-environment))
         (root (file-truename (make-temp-file "jaunder-git-" t)))
         (default-directory root))
    (unwind-protect
        (progn
          (should (zerop (call-process "git" nil nil nil "init")))
          (with-temp-file (expand-file-name "tracked.png" root) (insert "x"))
          (should (zerop (call-process "git" nil nil nil "add" "tracked.png")))
          (with-temp-file (expand-file-name ".gitignore" root) (insert "ignored.png\n"))
          (with-temp-file (expand-file-name "ignored.png" root) (insert "y"))
          (let ((outside (make-temp-file "jaunder-outside-" nil ".png")))
            (unwind-protect
                (progn
                  (should (jaunder--git-tracked-p
                           root (expand-file-name "tracked.png" root)))
                  (should-not (jaunder--git-tracked-p
                               root (expand-file-name "ignored.png" root)))
                  (should-not (jaunder--git-tracked-p root outside)))
              (delete-file outside)))
          (let ((sub (expand-file-name "sub/deep" root)))
            (make-directory sub t)
            (should (equal (file-truename (jaunder--git-toplevel sub))
                           (file-truename root)))))
      (delete-directory root t))))

(ert-deftest jaunder-git-toplevel-skips-on-unenterable-dir ()
  ;; Best-effort: an unenterable `default-directory' must not signal on the
  ;; publish path — the helper returns nil (skip), never errors.
  (should-not (jaunder--git-toplevel "/jaunder-no-such-dir-xyz/")))

(ert-deftest jaunder-git-media-helpers-handle-worktree-and-untracked-results ()
  "Git helper results drive the warning layer without exposing process details."
  (let (arguments)
    (cl-letf (((symbol-function 'executable-find) (lambda (_) "/git"))
              ((symbol-function 'call-process)
               (lambda (&rest args)
                 (push args arguments)
                 (if (equal (nth 4 args) "rev-parse") 0 1)))
              ((symbol-function 'buffer-string) (lambda () "/repo\n")))
             (should (equal (jaunder--git-toplevel "/repo/subdir") "/repo"))
             (should-not (jaunder--git-tracked-p "/repo" "/repo/missing.png"))
             (should (= (length arguments) 2)))))

;;; jaunder-media-test.el ends here
