;;; jaunder-pull-convergence-test.el --- Pull checkpoint convergence -*- lexical-binding: t; -*-

;; Copyright (C) 2026 Jaunder contributors

;;; Commentary:
;; Controlled wire, clock and filesystem boundaries exercise real reconciliation
;; commands, staging, installation and refreshed classification.

;;; Code:

(require 'ert)
(require 'cl-lib)
(require 'jaunder)

(defun jaunder-pull-convergence-test--state ()
  "Return the single Post's current report state."
  (should (= 1 (length (jaunder-reconcile-report-rows jaunder-reconcile-report))))
  (jaunder-reconcile-row-state
   (car (jaunder-reconcile-report-rows jaunder-reconcile-report))))

(defun jaunder-pull-convergence-test--exercise (state delay &optional rename checkpoint-failure)
  "Accept STATE's remote Post after DELAY, optionally RENAME or CHECKPOINT-FAILURE.
Advance a controlled clock at a real Collection, Media or disk-write boundary
instead of sleeping.  All staging, integrity checks and installation are real."
  (let* ((root (file-name-as-directory (make-temp-file "jaunder-convergence-" t)))
         (path (expand-file-name (if rename "old.org" "post.org") root))
         (destination (expand-file-name "post.org" root))
         (jaunder-blogs (list (cons root '(:base-url "https://example.test" :username "alice"))))
         (installation-time (current-time))
         (clock (time-subtract installation-time 60))
         (transferring nil)
         (etag "\"new\"")
         (hash (secure-hash 'sha256 ""))
         (remote-body (if (eq delay 'media)
                          (format "Remote [[/media/upload/%s/%s/%s/a.bin][Media]]."
                                  (substring hash 0 2) (substring hash 2 4) hash)
                        ;; A marker-looking line in authored source must stay literal.
                        "Remote body.\n#+PROPERTY: JAUNDER_SYNCED_AT authored-body-text\n"))
         (entry (concat "<entry xmlns=\"http://www.w3.org/2005/Atom\""
                        " xmlns:app=\"http://www.w3.org/2007/app\""
                        " xmlns:j=\"https://jaunder.org/ns/atompub\">"
                        "<title>Remote title</title>"
                        "<link rel=\"edit\" href=\"https://example.test/atompub/alice/posts/7\"/>"
                        "<j:slug>post</j:slug><content type=\"text/org\">" remote-body "</content>"
                        "<app:control><app:draft>yes</app:draft></app:control></entry>"))
         (real-write (symbol-function 'write-region))
         (real-set-times (symbol-function 'set-file-times))
         before buffer visiting)
    (unwind-protect
        (cl-letf (((symbol-function 'current-time) (lambda () clock))
                  ((symbol-function 'jaunder--current-zone-name) (lambda () "UTC"))
                  ((symbol-function 'y-or-n-p) (lambda (&rest _) t))
                  ((symbol-function 'write-region)
                   (lambda (start end filename &rest args)
                     (when (and transferring (eq delay 'write)
                                (string-prefix-p ".jaunder-pull-" (file-name-nondirectory filename)))
                       (setq clock installation-time))
                     (apply real-write start end filename args)))
                  ((symbol-function 'set-file-times)
                   (lambda (filename &rest args)
                     (if (and checkpoint-failure transferring
                              (string-prefix-p ".jaunder-pull-" (file-name-nondirectory filename)))
                         (error "injected checkpoint failure")
                       (apply real-set-times filename args))))
                  ((symbol-function 'plz)
                   (lambda (_method url &rest _)
                     (should (string-suffix-p "/a.bin" url))
                     (setq clock installation-time)
                     (make-plz-response :status 200 :body "" :headers
                                        (list (cons 'etag (concat "\"sha256-" hash "\""))
                                              '(x-jaunder-instance . "12345678-1234-1234-1234-123456789abc")))))
                  ((symbol-function 'jaunder--http-request)
                   (lambda (_method url &rest _)
                     (cond
                      ((string-suffix-p "/service" url)
                       (list :status 200 :body
                             "<service xmlns=\"http://www.w3.org/2007/app\" xmlns:atom=\"http://www.w3.org/2005/Atom\"><workspace><atom:title>Blog</atom:title></workspace></service>"))
                      ((string-suffix-p "/posts" url)
                       ;; Opening the report does not consume simulated delay.
                       (when (and transferring (eq delay 'collection))
                         (setq clock installation-time))
                       (list :status 200 :body
                             (concat "<feed xmlns=\"http://www.w3.org/2005/Atom\" xmlns:j=\"https://jaunder.org/ns/atompub\">"
                                     "<entry><link rel=\"edit\" href=\"https://example.test/atompub/alice/posts/7\"/>"
                                     "<j:slug>post</j:slug><j:etag>" etag "</j:etag></entry></feed>")))
                      ((string-suffix-p "/posts/7" url)
                       (list :status 200 :headers
                             (list (cons "etag" etag)
                                   '("x-jaunder-instance" . "12345678-1234-1234-1234-123456789abc"))
                             :body entry))
                      (t (error "Unexpected fixture request: %s" url))))))
          (unless (eq state 'server-only)
            (setq before (concat "#+PROPERTY: JAUNDER_ID 7\n#+PROPERTY: JAUNDER_SLUG "
                                 (if rename "old" "post")
                                 "\n#+PROPERTY: JAUNDER_SYNCED \"old\"\n#+PROPERTY: JAUNDER_SYNCED_AT "
                                 (format-time-string "%Y-%m-%dT%H:%M:%SZ" clock t)
                                 "\n\nLocal body.\n"))
            (write-region before nil path nil 'silent)
            (set-file-times path (if (eq state 'conflict) (time-add clock 5) clock))
            (when rename (setq visiting (find-file-noselect path))))
          (jaunder-reconcile root)
          (setq buffer (get-buffer "*Jaunder Reconcile*"))
          (with-current-buffer buffer
            (should (eq (jaunder-pull-convergence-test--state) state))
            (let ((row (car (jaunder-reconcile-report-rows jaunder-reconcile-report))))
              (puthash (jaunder-reconcile-row-key row) t jaunder-reconcile-marks))
            (setq transferring t)
            (if (eq state 'conflict)
                (jaunder-reconcile-keep-remote-selected)
              (jaunder-reconcile-pull-selected))
            (let ((result (car jaunder-reconcile-last-batch-results)))
              (if checkpoint-failure
                  (progn
                    (should (eq (jaunder-reconcile-result-outcome result) 'failed))
                    (should (eq (jaunder-reconcile-result-local-effect result) 'unchanged))
                    (should (equal (with-temp-buffer (insert-file-contents path) (buffer-string)) before)))
                (should (eq (jaunder-reconcile-result-outcome result) 'success))
                (should (eq (jaunder-pull-convergence-test--state) 'unchanged))
                (with-temp-buffer
                  (insert-file-contents destination)
                  (should (equal (jaunder--inventory-buffer-property "JAUNDER_SYNCED") etag))
                  (should (equal (jaunder--inventory-buffer-property "JAUNDER_SYNCED_AT")
                                 (jaunder-reconcile-result-synced-at result)))
                  (should (string-suffix-p
                           (if (eq delay 'media)
                               (format "Remote [[file:local-media/%s/a.bin][Media]]." hash)
                             remote-body)
                           (buffer-string))))
                (when visiting
                  (should-not (file-exists-p path))
                  (should (equal (buffer-file-name visiting) destination))
                  (with-current-buffer visiting
                    (should-not (buffer-modified-p))
                    (should (equal (jaunder--inventory-buffer-property "JAUNDER_SYNCED_AT")
                                   (jaunder-reconcile-result-synced-at result)))))
                ;; Accepted ETags and install timestamps must not hide future changes.
                (setq etag "\"later\"")
                (jaunder-reconcile-refresh)
                (should (eq (jaunder-pull-convergence-test--state) 'server-ahead))
                (setq etag "\"new\"")
                (write-region "Local edit.\n" nil destination t 'silent)
                (set-file-times destination
                                (time-add (date-to-time (jaunder-reconcile-result-synced-at result)) 5))
                (jaunder-reconcile-refresh)
                (should (eq (jaunder-pull-convergence-test--state) 'local-ahead))
                (setq etag "\"later\"")
                (jaunder-reconcile-refresh)
                (should (eq (jaunder-pull-convergence-test--state) 'conflict))))))
      (when (buffer-live-p buffer) (kill-buffer buffer))
      (when (buffer-live-p visiting) (kill-buffer visiting))
      (delete-directory root t))))

(ert-deftest jaunder-reconcile-pull-converges-after-slow-collection-verification ()
  "An accepted remote Post stays unchanged after delayed verification."
  (jaunder-pull-convergence-test--exercise 'server-ahead 'collection))

(ert-deftest jaunder-reconcile-pull-converges-after-slow-media-and-canonical-rename ()
  "Verified Local Media Copies and clean buffers retain a converged checkpoint."
  (jaunder-pull-convergence-test--exercise 'server-ahead 'media t))

(ert-deftest jaunder-reconcile-server-only-pull-converges-after-slow-media ()
  "A newly installed remote Post does not appear locally edited."
  (jaunder-pull-convergence-test--exercise 'server-only 'media))

(ert-deftest jaunder-reconcile-keep-remote-converges-after-slow-media ()
  "An explicitly accepted conflict remains synchronized after staging."
  (jaunder-pull-convergence-test--exercise 'conflict 'media))

(ert-deftest jaunder-reconcile-pull-converges-after-slow-temporary-write ()
  "Disk-write latency does not manufacture a local edit."
  (jaunder-pull-convergence-test--exercise 'server-ahead 'write))

(ert-deftest jaunder-reconcile-pull-checkpoint-failure-preserves-reviewed-local-post ()
  "An uncheckpointed temporary file cannot replace the reviewed Post."
  (jaunder-pull-convergence-test--exercise 'server-ahead 'collection nil t))

(provide 'jaunder-pull-convergence-test)
;;; jaunder-pull-convergence-test.el ends here
