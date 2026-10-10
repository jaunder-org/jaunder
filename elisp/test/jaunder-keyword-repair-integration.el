;;; jaunder-keyword-repair-integration.el --- Live add-only repair proofs -*- lexical-binding: t; -*-

;; Copyright (C) 2026 Jaunder contributors

;;; Commentary:
;; Disposable Members cross the real preview/apply and conditional HTTP boundary.
;; Compare persisted authored/lifecycle DOM, not merely the repair PUT response.

;;; Code:

(require 'ert)
(require 'cl-lib)
(require 'jaunder)
(require 'jaunder-keyword-repair)
(require 'jaunder-integration-helper)

(defmacro jaunder-keyword-repair-live--with-root (&rest body)
  "Run BODY with a disposable configured ROOT, cleaning visiting buffers."
  (declare (indent 0))
  `(jaunder-test--with-live-server
    (let* ((root (file-name-as-directory (make-temp-file "jaunder-repair-live-" t)))
           (jaunder-blogs (list (cons root (list :base-url jaunder-test-base-url
                                                 :username jaunder-test-username)))))
      (unwind-protect
          (jaunder--call-with-blog root (lambda () ,@body))
        (dolist (buffer (buffer-list))
          (when (and (buffer-file-name buffer)
                     (string-prefix-p root (buffer-file-name buffer)))
            (with-current-buffer buffer (set-buffer-modified-p nil))
            (kill-buffer buffer)))
        (delete-directory root t)))))

(defun jaunder-keyword-repair-live--create (title format &optional published draft tags)
  "Create a real Member with TITLE, FORMAT, PUBLISHED, DRAFT and TAGS."
  (let* ((response
          (jaunder--http-request
           "POST" (jaunder--collection-url)
           (jaunder--atom-entry->xml
            (jaunder--make-entry :title title :content-type format
                                 :body "Remote sentinel <xml> & \"quoted\" body.\n"
                                 :summary "Remote summary & <sentinel>"
                                 :audiences '("public") :published published :draft draft
                                 :categories (or tags '("RUST"))))
           "application/atom+xml"))
         (location (jaunder--response-header response "Location")))
    (should (= (plist-get response :status) 201))
    (should (string-match "/\\([0-9]+\\)/?\\'" location))
    (match-string 1 location)))

(defun jaunder-keyword-repair-live--read (id)
  "Read persisted Member ID through authenticated transport."
  (let ((response (jaunder--http-request "GET" (jaunder--member-url id))))
    (should (= (plist-get response :status) 200))
    response))

(defun jaunder-keyword-repair-live--dom (response)
  "Parse RESPONSE's complete persisted DOM."
  (with-temp-buffer
    (insert (plist-get response :body))
    (car (xml-parse-region (point-min) (point-max)))))

(defun jaunder-keyword-repair-live--preserved (response)
  "Project all persisted fields except categories and mutation timestamp."
  (let ((dom (jaunder-keyword-repair-live--dom response)))
    (setcdr (cdr dom)
            (cl-remove-if (lambda (child)
                            (or (stringp child)
                                (memq (jaunder--atom-local-name (car child)) '(category updated))))
                          (dom-children dom)))
    dom))

(defun jaunder-keyword-repair-live--local (root id)
  "Install older local source and sentinel markers for ID under ROOT."
  (let ((path (expand-file-name (concat id ".org") root)))
    (with-temp-file path
      (insert (format "#+TITLE: Older local title\n#+PROPERTY: JAUNDER_ID %s\n" id)
              "#+PROPERTY: JAUNDER_STATUS published\n#+PROPERTY: JAUNDER_SYNCED \"old\"\n"
              "#+PROPERTY: JAUNDER_SLUG old-local-slug\n#+KEYWORDS: rust, 100 bullets, 100-BULLETS, James S. A. Corey\n\nOlder local body.\n"))
    (find-file-noselect path)))

(defun jaunder-keyword-repair-live--snapshot (buffer)
  "Capture BUFFER's path, literal bytes, clean text and synchronization markers."
  (with-current-buffer buffer
    (should-not (buffer-modified-p))
    (list (buffer-file-name) (buffer-string)
          (with-temp-buffer
            (set-buffer-multibyte nil)
            (insert-file-contents-literally (buffer-file-name buffer))
            (buffer-string))
          (mapcar #'jaunder--buffer-property '("JAUNDER_ID" "JAUNDER_SYNCED" "JAUNDER_SLUG")))))

(defun jaunder-keyword-repair-live--apply (report &optional decline)
  "Apply row zero of REPORT, optionally DECLINE confirmation."
  (cl-letf (((symbol-function 'yes-or-no-p) (lambda (&rest _) (not decline))))
    (let ((result (jaunder-keyword-repair-apply report '(0))))
      (should (equal result jaunder-keyword-repair-last-application))
      result)))

(ert-deftest jaunder-keyword-repair-live-preserves-native-members-and-frozen-permalinks ()
  "Real category-only repair retains every authored/lifecycle Member field."
  (dolist (format '("text/org" "text/markdown" "html"))
    (dolist (published '("2024-01-02T03:04:05Z" "2099-01-02T03:04:05Z"))
      (jaunder-keyword-repair-live--with-root
       (let* ((title (concat "Repair collision " (file-name-nondirectory (directory-file-name root))))
              (first (jaunder-keyword-repair-live--create title format published))
              (id (jaunder-keyword-repair-live--create title format published))
              (before (jaunder-keyword-repair-live--read id))
              (buffer (jaunder-keyword-repair-live--local root id))
              (snapshot (jaunder-keyword-repair-live--snapshot buffer))
              (real (symbol-function 'jaunder--http-request))
              (writes 0) report)
         ;; Two actual creations with the same title allocate a collision suffix.
         (should-not (equal (cdr (assq 'slugs (jaunder--harvest-response-fields
                                               (plist-get (jaunder-keyword-repair-live--read first) :body))))
                            (cdr (assq 'slugs (jaunder--harvest-response-fields (plist-get before :body))))))
         (cl-letf (((symbol-function 'jaunder--http-request)
                    (lambda (method url &rest args)
                      (unless (equal method "GET") (setq writes (1+ writes)))
                      (apply real method url args))))
           (setq report (jaunder-keyword-repair-preview root))
           (should (= writes 0))
           (should (equal (plist-get (car (plist-get (jaunder-keyword-repair-report-data report) :rows)) :additions)
                          '("100-bullets" "james-s-a-corey")))
           (should (plist-get (jaunder-keyword-repair-live--apply report t) :cancelled))
           (should (= writes 0))
           (should (eq (plist-get (car (plist-get (jaunder-keyword-repair-live--apply report) :results)) :status) 'repaired))
           (should (= writes 1))
           (let* ((after (jaunder-keyword-repair-live--read id))
                  (fields (jaunder--harvest-response-fields (plist-get after :body))))
             (should (equal (jaunder-keyword-repair-live--preserved before)
                            (jaunder-keyword-repair-live--preserved after)))
             (should (equal (sort (copy-sequence (cdr (assq 'categories fields))) #'string<)
                            '("100-bullets" "RUST" "james-s-a-corey"))))
           (let ((fresh (jaunder-keyword-repair-preview root)))
             (should (eq (plist-get (car (plist-get (jaunder-keyword-repair-report-data fresh) :rows)) :status) 'unchanged))
             (should (eq (plist-get (car (plist-get (jaunder-keyword-repair-live--apply fresh) :results)) :status) 'unchanged)))
           (should (= writes 1)))
         (should (equal snapshot (jaunder-keyword-repair-live--snapshot buffer))))))))

(ert-deftest jaunder-keyword-repair-live-blocks-draft-and-complete-over-cap-union ()
  "Selected ineligible real Members remain byte-for-byte unchanged."
  (dolist (draft '(t nil))
    (jaunder-keyword-repair-live--with-root
     (let* ((tags (unless draft (mapcar (lambda (n) (format "tag%d" n)) (number-sequence 1 256))))
            (id (jaunder-keyword-repair-live--create "Blocked repair" "text/org" nil draft tags))
            (before (jaunder-keyword-repair-live--read id))
            (buffer (jaunder-keyword-repair-live--local root id))
            (snapshot (jaunder-keyword-repair-live--snapshot buffer))
            (report (jaunder-keyword-repair-preview root))
            (real (symbol-function 'jaunder--http-request)))
       (let ((row (car (plist-get (jaunder-keyword-repair-report-data report) :rows))))
         (should (eq (plist-get row :status) 'blocked))
         (should (eq (plist-get row :reason) (if draft 'remote-draft 'over-tag-cap))))
       (cl-letf (((symbol-function 'jaunder--http-request)
                  (lambda (method url &rest args)
                    (should (equal method "GET"))
                    (apply real method url args))))
         (should (eq (plist-get (car (plist-get (jaunder-keyword-repair-live--apply report) :results)) :status) 'blocked)))
       (should (equal (plist-get before :body) (plist-get (jaunder-keyword-repair-live--read id) :body)))
       (should (equal snapshot (jaunder-keyword-repair-live--snapshot buffer)))))))

(ert-deftest jaunder-keyword-repair-live-real-precondition-race-and-response-loss ()
  "Concurrent update wins 412; a committed but lost response is never retried."
  (dolist (mode '(stale race lost))
    (jaunder-keyword-repair-live--with-root
     (let* ((id (jaunder-keyword-repair-live--create "Repair boundary" "text/org" "2024-01-02T03:04:05Z"))
            (buffer (jaunder-keyword-repair-live--local root id))
            (snapshot (jaunder-keyword-repair-live--snapshot buffer))
            (report (jaunder-keyword-repair-preview root))
            (real (symbol-function 'jaunder--http-request))
            (writes 0) concurrent result)
       (cl-labels ((update ()
                     (let* ((current (jaunder-keyword-repair-live--read id))
                            (xml (replace-regexp-in-string "Remote sentinel" "Concurrent winner"
                                                           (plist-get current :body))))
                       (should (= (plist-get (funcall real "PUT" (jaunder--member-url id) xml
                                                      "application/atom+xml"
                                                      (list (cons "If-Match" (jaunder--response-header current "ETag")))) :status) 200))
                       (setq concurrent (jaunder-keyword-repair-live--read id)))))
         (when (eq mode 'stale) (update))
         (cl-letf (((symbol-function 'jaunder--http-request)
                    (lambda (method url &rest args)
                      (if (not (equal method "PUT")) (apply real method url args)
                        (setq writes (1+ writes))
                        (should (assoc "If-Match" (nth 2 args)))
                        ;; Race is deliberately after real apply preflight and before PUT.
                        (when (eq mode 'race) (update))
                        (let ((response (apply real method url args)))
                          (if (eq mode 'lost) (error "Synthetic lost response after commit") response))))))
           (setq result (car (plist-get (jaunder-keyword-repair-live--apply report) :results))))
         (should (= writes (if (eq mode 'stale) 0 1)))
         (if (eq mode 'lost)
             (progn
               (should (eq (plist-get result :status) 'unknown-remote-outcome))
               (should (equal (sort (copy-sequence (cdr (assq 'categories (jaunder--harvest-response-fields
                                                                           (plist-get (jaunder-keyword-repair-live--read id) :body))))) #'string<)
                              '("100-bullets" "RUST" "james-s-a-corey")))
               (should (equal (jaunder-keyword-repair-error-source (cons 'error result))
                              '(error "Synthetic lost response after commit")))
               (should (eq (plist-get (car (plist-get (jaunder-keyword-repair-report-data
                                                       (jaunder-keyword-repair-preview root)) :rows)) :status) 'unchanged)))
           (should (eq (plist-get result :status) 'blocked))
           (when (eq mode 'race) (should (= (plist-get result :http-status) 412)))
           (should (equal (plist-get concurrent :body) (plist-get (jaunder-keyword-repair-live--read id) :body))))
         ;; Recovery cannot resend the original stale intent, even after response loss.
         (cl-letf (((symbol-function 'jaunder--http-request)
                    (lambda (method url &rest args)
                      (should (equal method "GET")) (apply real method url args))))
           (should (eq (plist-get (car (plist-get (jaunder-keyword-repair-live--apply report) :results)) :status) 'blocked)))
         (should (equal snapshot (jaunder-keyword-repair-live--snapshot buffer))))))))

(ert-deftest jaunder-keyword-repair-live-preserves-reconciled-server-ahead ()
  "Repair never pushes the older body of a genuinely reconciled server-ahead Post."
  (jaunder-keyword-repair-live--with-root
   (let ((path (expand-file-name "older.org" root)) buffer)
     (with-temp-file path
       (insert "#+TITLE: Older local title\n#+PROPERTY: JAUNDER_STATUS published\n#+KEYWORDS: Rust, 100 bullets\n\nOlder local body.\n"))
     (setq buffer (find-file-noselect path))
     (with-current-buffer buffer (jaunder-publish) (save-buffer))
     (setq path (buffer-file-name buffer))
     (set-file-times path (time-subtract (current-time) (seconds-to-time 5)))
     (let* ((id (with-current-buffer buffer (jaunder--buffer-property "JAUNDER_ID")))
            (etag (with-current-buffer buffer (jaunder--buffer-property "JAUNDER_SYNCED")))
            (updated
             (jaunder--http-request
              "PUT" (jaunder--member-url id)
              (jaunder--atom-entry->xml
               (jaunder--make-entry :title "Newer server title" :body "Newer remote <xml> & body.\n"
                                    :content-type "text/markdown" :summary "Newer summary"
                                    :audiences '("private") :categories '("RUST")))
              "application/atom+xml" (list (cons "If-Match" etag)))))
       (should (= (plist-get updated :status) 200))
       (let* ((reconciliation (jaunder--reconcile-build-report root (jaunder--inventory-for-root root)))
              (row (cl-find-if (lambda (candidate) (jaunder-reconcile-row-local candidate))
                               (jaunder-reconcile-report-rows reconciliation))))
         (should (eq (jaunder-reconcile-row-state row) 'server-ahead)))
       (let ((before (jaunder-keyword-repair-live--read id))
             (snapshot (jaunder-keyword-repair-live--snapshot buffer)))
         (should (eq (plist-get (car (plist-get (jaunder-keyword-repair-live--apply
                                                 (jaunder-keyword-repair-preview root)) :results)) :status) 'repaired))
         (should (equal (jaunder-keyword-repair-live--preserved before)
                        (jaunder-keyword-repair-live--preserved (jaunder-keyword-repair-live--read id))))
         (should (equal snapshot (jaunder-keyword-repair-live--snapshot buffer))))))))

(provide 'jaunder-keyword-repair-integration)
;;; jaunder-keyword-repair-integration.el ends here
