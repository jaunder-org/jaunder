;;; jaunder-media-boundary-debug-test.el --- Media diagnostic boundaries -*- lexical-binding: t; -*-

;;; Commentary:
;; Real publish and pulled-copy owners preserve bytes, request/work counts and
;; signal data with logging off/on.  Failure snapshots include committed copies
;; and staging leftovers, so diagnostics cannot hide partial filesystem effects.

;;; Code:

(require 'ert)
(require 'cl-lib)
(require 'jaunder)
(load (expand-file-name "jaunder-debug-boundary-fixture.el"
                        (file-name-directory (or load-file-name buffer-file-name))) nil t)

(defun jaunder-media-debug--literal (path)
  "Read PATH without decoding its bytes."
  (with-temp-buffer
    (set-buffer-multibyte nil)
    (insert-file-contents-literally path)
    (buffer-string)))

(defun jaunder-media-debug--private-text-p (text hash)
  "Recognize fixture secrets in TEXT, including HASH."
  (or (string-match-p "private\\|https://\\|sha256" text)
      (string-match-p (regexp-quote hash) text)))

(defun jaunder-media-debug--assert-pairs (text)
  "Assert each retained span in TEXT has one matching start and terminal."
  (let ((spans (make-hash-table :test #'equal)))
    (dolist (line (split-string text "\n" t))
      (should (string-match " span=\\([^ ]+\\) label=\\([^ ]+\\) phase=\\(start\\|end\\)" line))
      (let ((id (match-string 1 line))
            (event (list (match-string 2 line) (match-string 3 line))))
        (puthash id (append (gethash id spans) (list event)) spans)))
    (maphash (lambda (_ events)
               (should (= (length events) 2))
               (should (equal (caar events) (caadr events)))
               (should (equal (mapcar #'cadr events) '("start" "end"))))
             spans)))

(defun jaunder-media-debug--pull (debug scenario)
  "Run real pulled-copy SCENARIO with DEBUG and return comparable effects."
  (jaunder-debug-boundary--with-session
   (let* ((root (file-name-as-directory (make-temp-file "jaunder-media-proof-" t)))
          (bytes (string-as-unibyte "private-bytes"))
          (hash (secure-hash 'sha256 bytes))
          (instance "12345678-1234-1234-1234-123456789abc")
          (origin "https://private.example")
          (url (format "%s/media/upload/%s/%s/%s/private-a.bin"
                       origin (substring hash 0 2) (substring hash 2 4) hash))
          (body (format "private-body [[%s][private-label]] [[%s]]" url url))
          (second (concat (substring url 0 (- (length url) (length "private-a.bin"))) "private-b.bin"))
          (requests 0) (hashes 0) (renames 0) result projection
          (hash-file (symbol-function 'jaunder--pull-media-file-sha256))
          (rename (symbol-function 'rename-file)))
     (unwind-protect
         (let* ((jaunder-debug debug)
                (plan (jaunder--pull-media-plan
                       "org" (if (eq scenario 'partial) (concat body " [[" second "]]") body) origin))
                (target (jaunder--pull-media-target-path root hash "private-a.bin")))
           (when (eq scenario 'corrupt) (write-region "private-corrupt" nil target nil 'silent))
           (when (eq scenario 'unsafe)
             (delete-directory (expand-file-name "local-media" root) t)
             (make-symbolic-link "private-missing" (expand-file-name "local-media" root)))
           (cl-letf (((symbol-function 'plz)
                      (lambda (&rest _)
                        (setq requests (1+ requests))
                        (make-plz-response
                         :status 200 :body bytes
                         :headers (list (cons "x-jaunder-instance"
                                              (if (eq scenario 'trust) "private-wrong-instance" instance))
                                        (cons "etag" (concat "\"sha256-" hash "\""))))))
                     ((symbol-function 'jaunder--pull-media-file-sha256)
                      (lambda (path) (setq hashes (1+ hashes)) (funcall hash-file path)))
                     ((symbol-function 'rename-file)
                      (lambda (&rest args)
                        (setq renames (1+ renames))
                        (if (and (eq scenario 'partial) (= renames 2))
                            (signal 'file-error '(private-install-failure))
                          (apply rename args)))))
             (setq result
                   (condition-case condition
                       (progn
                         (jaunder--pull-media-materialize root instance plan)
                         (let ((before requests))
                           (jaunder--pull-media-materialize root instance plan)
                           (should (= before requests)))
                         'success)
                     (error condition))))
           (setq projection
                 (list result requests hashes renames
                       (jaunder--pull-media-apply-plan plan)
                       (mapcar (lambda (path)
                                 (list (file-relative-name path root) (jaunder-media-debug--literal path)))
                               (sort (directory-files-recursively root "\\.bin\\'") #'string-lessp))
                       (length (directory-files-recursively root "\\.jaunder-media-"))))
           (if debug
               (let ((text (jaunder-debug-boundary--text)))
                 (jaunder-media-debug--assert-pairs text)
                 (should-not (jaunder-media-debug--private-text-p text hash))
                 (should-not (string-match-p (regexp-quote root) text))
                 (should-not jaunder--debug-operation-stack)
                 (if (eq scenario 'success)
                     (progn
                       (should (string-match-p "label=media.materialize phase=start count=1" text))
                       (dolist (label '("media.plan" "media.apply" "media.path" "media.hash" "media.verify" "media.download" "media.materialize"))
                         (should (>= (jaunder-debug-boundary--label-count label text) 2))))
                   (should (string-match-p "outcome=error" text))))
             (should (= jaunder--debug-id-counter 0))
             (should-not (get-buffer jaunder--debug-buffer-name)))
           projection)
       (delete-directory root t)))))

(ert-deftest jaunder-media-debug-pull-off-on-reuse-rejection-and-partial-effects ()
  "Logging preserves real installation, reuse, rejection and partial cleanup."
  (dolist (scenario '(success trust corrupt unsafe partial))
    (let* ((off (jaunder-media-debug--pull nil scenario))
           (on (jaunder-media-debug--pull t scenario)))
      ;; Error messages containing the independently allocated root differ only
      ;; there; injected failures retain their complete condition data below.
      (if (memq scenario '(corrupt unsafe))
          (progn (should (eq (caar off) 'error))
                 (should (eq (caar on) 'error))
                 (should (equal (cdr off) (cdr on))))
        (should (equal off on)))
      (should (= (car (last on)) 0))
      (pcase scenario
        ('success (should (eq (car on) 'success)) (should (= (nth 1 on) 1)))
        ('partial (should (equal (car on) '(file-error private-install-failure)))
                  (should (= (length (nth 5 on)) 1)))
        ((or 'corrupt 'unsafe) (should (= (nth 1 on) 0)))))))

(ert-deftest jaunder-media-debug-publish-off-on-real-source-and-disabled-counters ()
  "Publish deduplicates real files; disabled logging touches no diagnostic route."
  (let (results)
    (dolist (debug '(nil t))
      (jaunder-debug-boundary--with-session
       (let* ((root (make-temp-file "jaunder-media-publish-" t))
              (source (expand-file-name "private.png" root))
              (jaunder-warn-untracked-media nil)
              (jaunder--active-blog '(:base-url "https://private.example" :username "private-user"))
              (jaunder-debug debug) (calls 0) (diagnostic-calls 0)
              (now (symbol-function 'jaunder--debug-now))
              (format-event (symbol-function 'jaunder--debug-format-event))
              (buffer (symbol-function 'jaunder--debug-buffer)))
         (unwind-protect
             (progn
               (write-region "private-bytes" nil source nil 'silent)
               (cl-letf (((symbol-function 'jaunder--debug-now)
                          (lambda () (setq diagnostic-calls (1+ diagnostic-calls)) (funcall now)))
                         ((symbol-function 'jaunder--debug-format-event)
                          (lambda (event) (setq diagnostic-calls (1+ diagnostic-calls)) (funcall format-event event)))
                         ((symbol-function 'jaunder--debug-buffer)
                          (lambda () (setq diagnostic-calls (1+ diagnostic-calls)) (funcall buffer)))
                         ((symbol-function 'jaunder--http-request)
                          (lambda (&rest _)
                            (setq calls (1+ calls))
                            (list :status 201 :body "<entry xmlns=\"http://www.w3.org/2005/Atom\"><content src=\"https://private.example/media/uploaded\"/></entry>"))))
                 (with-temp-buffer
                   (setq default-directory (file-name-as-directory root))
                   (org-mode)
                   (insert "#+TITLE: private-title\n\n[[file:private.png][one]] [[file:private.png][two]]")
                   (let* ((before (buffer-string))
                          (body (jaunder-entry-body (jaunder--org->atom)))
                          (out (jaunder--localize-media body)))
                     (should (equal before (buffer-string)))
                     (should (= calls 1))
                     (should (string-match-p "media/uploaded" out))
                     (push (list out calls (jaunder-media-debug--literal source)) results))))
               (if debug
                   (let ((text (jaunder-debug-boundary--text)))
                     (jaunder-media-debug--assert-pairs text)
                     (dolist (label '("media.plan" "media.path" "media.upload" "media.apply"))
                       (should (= 2 (jaunder-debug-boundary--label-count label text))))
                     (should-not (string-match-p "private" text))
                     (should (string-match-p "parent=" text)))
                 (should (= diagnostic-calls 0))
                 (should (= jaunder--debug-id-counter 0))
                 (should-not (get-buffer jaunder--debug-buffer-name))))
           (delete-directory root t)))))
    (should (equal (car results) (cadr results)))))

(ert-deftest jaunder-media-debug-real-owner-exact-error-and-quit-data ()
  "Network, literal-file and verification owners retain exact signaled data."
  (dolist (debug '(nil t))
    (dolist (kind '(error quit))
      (jaunder-debug-boundary--with-session
       (let* ((root (make-temp-file "jaunder-media-signals-" t))
              (path (expand-file-name "private.bin" root))
              (jaunder-debug debug)
              (jaunder--active-blog '(:base-url "https://private.example" :username "private-user"))
              (data '(private-condition-data)))
         (unwind-protect
             (progn
               (write-region "" nil path nil 'silent)
               (dolist (owner '(upload download hash verify))
                 (cl-letf (((symbol-function 'jaunder--http-request) (lambda (&rest _) (signal kind data)))
                           ((symbol-function 'plz) (lambda (&rest _) (signal kind data)))
                           ((symbol-function 'insert-file-contents-literally) (lambda (&rest _) (signal kind data))))
                   (should
                    (equal (cons kind data)
                           (condition-case condition
                               (pcase owner
                                 ('upload (jaunder--upload-media path "application/octet-stream"))
                                 ('download (jaunder--pull-media-get "https://private.example/media" path))
                                 ('hash (jaunder--pull-media-file-sha256 path))
                                 ('verify (jaunder--pull-media-require-existing-copy path (make-string 64 ?a))))
                             (error condition) (quit condition))))))
               (should-not jaunder--debug-operation-stack)
               (if debug
                   (let ((text (jaunder-debug-boundary--text)))
                     (jaunder-media-debug--assert-pairs text)
                     (should-not (string-match-p "private" text))
                     (dolist (label '("media.upload" "media.download" "media.verify"))
                       (should (= 2 (jaunder-debug-boundary--label-count label text))))
                     (should (= 4 (jaunder-debug-boundary--label-count "media.hash" text))))
                 (should (= jaunder--debug-id-counter 0))
                 (should-not (get-buffer jaunder--debug-buffer-name))))
           (delete-directory root t)))))))

;;; jaunder-media-boundary-debug-test.el ends here
