;;; jaunder-author-debug-test.el --- Authoring lifecycle diagnostics -*- lexical-binding: t; -*-

;;; Commentary:
;; Real authoring commands preserve saved Post bytes and local buffer/file
;; lifecycles with diagnostics off/on.  Only prompt, wire, clock, and fault
;; inputs are controlled; publishing, write-back, rename, and deletion are real.

;;; Code:

(require 'ert)
(require 'cl-lib)
(require 'rx)
(require 'jaunder)
(load (expand-file-name "jaunder-debug-boundary-fixture.el"
                        (file-name-directory (or load-file-name buffer-file-name))) nil t)

(defun jaunder-author-debug--literal (path)
  "Return literal bytes at PATH."
  (with-temp-buffer
    (set-buffer-multibyte nil)
    (insert-file-contents-literally path)
    (buffer-string)))

(defun jaunder-author-debug--run (enabled prefix action &optional kind)
  "Run actual ACTION with PREFIX and ENABLED; inject native KIND if requested."
  (jaunder-debug-boundary--with-session
   (let* ((root (file-name-as-directory (make-temp-file "jaunder-author-private-" t)))
          (default-directory root)
          (jaunder-blogs nil)
          (jaunder--service-doc-cache nil)
          (formatter (symbol-function 'format-time-string))
          (delete (symbol-function 'delete-file))
          (fixed (encode-time 0 2 3 7 10 2026 t))
          (failure '("private-condition" 17))
          requests created source before after result new-result command-result
          (deletes 0))
     (setq jaunder-debug enabled)
     (unwind-protect
         (save-window-excursion
           (cl-letf (((symbol-function 'format-time-string)
                      (lambda (format &optional _time zone)
                        (funcall formatter format fixed zone)))
                     ((symbol-function 'read-string)
                      (lambda (&rest _)
                        (when (and kind (eq action 'prompt)) (signal kind failure))
                        "private-title"))
                     ((symbol-function 'completing-read)
                      (lambda (prompt &rest _)
                        (if (string-prefix-p "Tag" prompt) "" "draft")))
                     ((symbol-function 'jaunder--idempotency-key) (lambda () "private-key"))
                     ((symbol-function 'delete-file)
                      (lambda (path &rest args)
                        (setq deletes (1+ deletes))
                        (when (and kind (eq action 'cancel)) (signal kind failure))
                        (apply delete path args)))
                     ((symbol-function 'jaunder--http-request)
                      (lambda (method url &optional body &rest _)
                        (push (list method url body) requests)
                        (when (and kind (equal method "POST")) (signal kind failure))
                        (if (equal method "GET")
                            (list :status 200 :body
                                  (concat "<service xmlns=\"http://www.w3.org/2007/app\""
                                          " xmlns:j=\"https://jaunder.org/ns/atompub\""
                                          " xmlns:atom=\"http://www.w3.org/2005/Atom\">"
                                          "<workspace><atom:title>private-service</atom:title>"
                                          "<j:extension version=\"1\""
                                          " features=\"audience format-media-type\"/>"
                                          "</workspace></service>"))
                          (list :status 201
                                :headers '(("location" . "https://private.example/atompub/private-user/posts/42")
                                           ("etag" . "\"private-etag\""))
                                :body
                                (concat "<entry xmlns=\"http://www.w3.org/2005/Atom\""
                                        " xmlns:j=\"https://jaunder.org/ns/atompub\">"
                                        "<j:slug>private-post</j:slug><j:audience>private</j:audience>"
                                        "<content type=\"text/org\">private-body</content></entry>"))))))
             (setq result
                   (condition-case condition
                       (progn
                         (setq new-result (jaunder-new-post prefix))
                         (setq created (current-buffer) source (buffer-file-name))
                         (insert "private-body\n")
                         (save-buffer)
                         (setq before (jaunder-author-debug--literal source))
                         (setq jaunder-blogs
                               (list (cons root '(:base-url "https://private.example" :username "private-user"))))
                         (setq command-result
                               (if (eq action 'complete)
                                   (call-interactively (key-binding (kbd "C-c C-c")))
                                 (call-interactively (key-binding (kbd "C-c C-k")))))
                         'success)
                     (error condition) (quit condition)))
             (when kind (should (equal result (cons kind failure))))
             (setq after (mapcar (lambda (path)
                                   (cons (file-name-nondirectory path)
                                         (jaunder-author-debug--literal path)))
                                 (directory-files root t (rx ".org" string-end))))
             (let ((text (when enabled (jaunder-debug-boundary--text))))
               (if enabled
                   (progn
                     (should (= 2 (jaunder-debug-boundary--label-count "author.new" text)))
                     (unless (eq action 'prompt)
                       (should (= 2 (jaunder-debug-boundary--label-count
                                     (if (eq action 'complete) "author.complete" "author.cancel") text))))
                     ;; Interactive callbacks are distinct roots; called work
                     ;; shares its callback's correlation and every span closes.
                     (let ((spans (make-hash-table :test #'equal)) correlations)
                       (dolist (line (split-string text "\n" t))
                         (should (string-match (rx " correlation=" (group (+ (not space)))) line))
                         (push (match-string 1 line) correlations)
                         (should (string-match (rx " span=" (group (+ (not space)))
                                                   (optional " parent=" (+ (not space)))
                                                   " label=" (group (+ (not space)))
                                                   " phase=" (group (or "start" "end"))) line))
                         (let ((id (match-string 1 line))
                               (event (list (match-string 2 line) (match-string 3 line))))
                           (puthash id (append (gethash id spans) (list event)) spans)))
                       (should (= (length (delete-dups correlations)) (if (eq action 'prompt) 1 2)))
                       (maphash (lambda (_ events)
                                  (should (= (length events) 2))
                                  (should (equal (caar events) (caadr events)))
                                  (should (equal (mapcar #'cadr events) '("start" "end")))) spans))
                     (when kind
                       (should (string-match-p
                                (rx-to-string `(seq " outcome=" ,(if (eq kind 'quit) "cancelled" "error") line-end))
                                text)))
                     (should-not (string-match-p (rx (or "private" "https://")) text)))
                 (should (zerop jaunder--debug-id-counter))
                 (should-not (get-buffer jaunder--debug-buffer-name)))
               (list :result result :new-result new-result :command-result command-result
                     :before before :after after
                     :input-live (and created (buffer-live-p created))
                     :input-modified (and (buffer-live-p created)
                                          (buffer-modified-p created))
                     :requests (nreverse requests) :deletes deletes))))
       (when (buffer-live-p created)
         (with-current-buffer created (set-buffer-modified-p nil))
         (kill-buffer created))
       (delete-directory root t)))))

(ert-deftest jaunder-author-debug-new-and-cancel-off-on-real-files ()
  "Both creation paths save the same bytes and abandon only their local input."
  (dolist (prefix '(nil (4)))
    (let ((off (jaunder-author-debug--run nil prefix 'cancel))
          (on (jaunder-author-debug--run t prefix 'cancel)))
      (should (equal off on))
      (should (eq (plist-get on :result) 'success))
      (should (string-match-p "private-body" (plist-get on :before)))
      (should-not (plist-get on :after))
      (should-not (plist-get on :input-live))
      (should-not (plist-get on :requests))
      (should (= 1 (plist-get on :deletes))))))

(ert-deftest jaunder-author-debug-complete-off-on-real-publish-and-buffer-exit ()
  "Completion publishes once, installs exact bytes and retains diagnostics."
  (let ((off (jaunder-author-debug--run nil '(4) 'complete))
        (on (jaunder-author-debug--run t '(4) 'complete)))
    (should (equal off on))
    (should (eq (plist-get on :result) 'success))
    (should-not (plist-get on :input-live))
    (should (equal (mapcar #'car (plist-get on :requests)) '("GET" "GET" "POST")))
    (should (equal (mapcar #'car (plist-get on :after)) '("private-post.org")))
    (let ((installed (cdar (plist-get on :after))))
      (should (string-match-p "JAUNDER_ID 42" installed))
      (should (string-match-p "private-body" installed))
      (should-not (string-match-p "JAUNDER_CREATE_KEY" installed)))))

(ert-deftest jaunder-author-debug-native-errors-and-quits-preserve-partial-effects ()
  "Prompt, publish and deletion failures keep exact native data and effects."
  (dolist (action '(prompt complete cancel))
    (dolist (kind '(error quit))
      (let ((off (jaunder-author-debug--run nil (unless (eq action 'prompt) '(4)) action kind))
            (on (jaunder-author-debug--run t (unless (eq action 'prompt) '(4)) action kind)))
        (should (equal off on))
        (if (eq action 'prompt)
            (progn (should-not (plist-get on :after)) (should-not (plist-get on :input-live)))
          (should (plist-get on :input-live))
          (should (= 1 (length (plist-get on :after))))
          (should-not (plist-get on :input-modified)))))))

(provide 'jaunder-author-debug-test)
;;; jaunder-author-debug-test.el ends here
