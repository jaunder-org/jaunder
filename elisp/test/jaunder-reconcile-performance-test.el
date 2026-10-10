;;; jaunder-reconcile-performance-test.el --- Large Collection fetch evidence -*- lexical-binding: t; -*-

;; Copyright (C) 2026 Jaunder contributors

;;; Commentary:
;; A deterministic paginated Collection exercises the actual inventory join
;; used by selected matched-Post pulls, not a cached batch snapshot.

;;; Code:

(require 'ert)
(require 'cl-lib)
(require 'jaunder)
(load (expand-file-name "jaunder-reconcile-performance-fixture.el"
                        (file-name-directory (or load-file-name buffer-file-name))) nil t)

(ert-deftest jaunder-reconcile-selected-fetch-reuses-complete-collection-pages ()
  "Three matched Posts share one complete four-page operation Collection read."
  (let* ((root (make-temp-file "jaunder-reconcile-scale-" t))
         (jaunder-blogs
          (list (cons (file-name-as-directory root)
                      '(:base-url "https://example.test" :username "alice"))))
         (ids '(1 50 100))
         (rows
          (mapcar
           (lambda (id)
             (let* ((slug (format "post-%03d" id))
                    (path (expand-file-name (concat slug ".org") root)))
               (with-temp-file path
                 (insert (format "#+TITLE: %s\n#+PROPERTY: JAUNDER_ID %d\n#+PROPERTY: JAUNDER_SLUG %s\n\nBody.\n"
                                 slug id slug)))
               (jaunder--make-reconcile-row
                :key (format "post:%d" id) :state 'server-ahead
                :local (jaunder--make-inventory-local :path path :id (number-to-string id)
                                                      :slug slug)
                :member (jaunder--make-inventory-member
                         :id (number-to-string id) :slug slug
                         :edit-uri (format "https://example.test/atompub/alice/posts/%d" id)))))
           ids))
         (buffer (jaunder--render-reconcile-report
                  (jaunder--make-reconcile-report :root root :rows rows)))
         (real-scan (symbol-function 'jaunder--scan-root-locals))
         (real-parse (symbol-function 'jaunder--parse-collection-page))
         (pages 0) (scan-seconds 0.0) (parse-seconds 0.0)
         before-total before-scan before-parse after-total progress paints)
    (unwind-protect
        (with-current-buffer buffer
          (cl-letf (((symbol-function 'jaunder--scan-root-locals)
                     (lambda (directory)
                       (let ((start (float-time)))
                         (prog1 (funcall real-scan directory)
                           (setq scan-seconds (+ scan-seconds (- (float-time) start)))))))
                    ((symbol-function 'jaunder--parse-collection-page)
                     (lambda (xml url)
                       (let ((start (float-time)))
                         (prog1 (funcall real-parse xml url)
                           (setq parse-seconds (+ parse-seconds (- (float-time) start)))))))
                    ((symbol-function 'jaunder--http-request)
                     (lambda (method url &rest _)
                       (should (equal method "GET"))
                       (setq pages (1+ pages))
                       (list :status 200 :body (jaunder-test--collection-page url))))
                    ((symbol-function 'jaunder--reconcile-refresh-buffer)
                     (lambda (&rest _) nil))
                    ((symbol-function 'message)
                     (lambda (format-string &rest arguments)
                       (push (apply #'format format-string arguments) progress)))
                    ((symbol-function 'redisplay)
                     (lambda (&rest _) (setq paints (1+ (or paints 0))))))
            ;; The old foreground path performed the same independent fresh
            ;; verification without page notifications.  Measure both shapes
            ;; in one run so their input and host conditions match.
            (let ((start (float-time)))
              (dolist (row rows)
                (should (plist-get (jaunder--reconcile-pull-unique-match row) :ok)))
              (setq before-total (- (float-time) start)
                    before-scan scan-seconds
                    before-parse parse-seconds))
            (should (= pages 12))
            (setq pages 0 scan-seconds 0.0 parse-seconds 0.0)
            (let ((start (float-time)))
              (jaunder--call-with-blog
               root
               (lambda ()
                 (jaunder--call-with-reconcile-operation
                  root (jaunder--active-base-url) (jaunder--active-username)
                  (lambda ()
                    (dolist (row rows)
                      (should (plist-get (jaunder--reconcile-pull-unique-match row) :ok)))))))
              (setq after-total (- (float-time) start)))
            (should (= pages 4)))
          (message "Reconcile fixture 100 Members/3 Posts: before 12 pages %.3fs (local %.3fs, parse %.3fs); after 4 pages %.3fs (local %.3fs, parse %.3fs). Synthetic timings exclude network."
                   before-total before-scan before-parse
                   after-total scan-seconds parse-seconds))
      (when (buffer-live-p buffer) (kill-buffer buffer))
      (delete-directory root t))))

(ert-deftest jaunder-reconcile-pull-batch-shows-stages-and-retains-timeout-context ()
  "A stalled first Post reports its stage; the next eligible Post still runs."
  (let* ((jaunder-blogs '(("/tmp/" :base-url "https://example.test" :username "alice")))
         (rows (mapcar (lambda (id)
                         (jaunder--make-reconcile-row
                          :key (format "post:%s" id) :state 'server-ahead
                          :remote-etag "\"same\""
                          :local (jaunder--make-inventory-local
                                  :path (format "/tmp/%s.org" id) :id id)
                          :member (jaunder--make-inventory-member :id id :slug id)))
                       '("7" "8")))
         (buffer (jaunder--render-reconcile-report
                  (jaunder--make-reconcile-report :root "/tmp" :rows rows)))
         progress)
    (unwind-protect
        (with-current-buffer buffer
          (cl-letf (((symbol-function 'jaunder--pull-stage-member)
                     (lambda (_root member)
                       (if (equal (jaunder-inventory-member-id member) "7")
                           (error "read timed out")
                         '(:id "8" :slug "8" :etag "\"same\"" :bytes "post"))))
                    ((symbol-function 'jaunder--reconcile-pull-unique-match)
                     (lambda (_) '(:ok t)))
                    ((symbol-function 'jaunder--reconcile-pull-remote-revalidation)
                     (lambda (&rest _) '(:ok t)))
                    ((symbol-function 'jaunder--reconcile-pull-install-staged)
                     (lambda (&rest _) '(:outcome success :local-effect replaced)))
                    ((symbol-function 'jaunder--reconcile-refresh-buffer)
                     (lambda (&rest _) nil))
                    ((symbol-function 'message)
                     (lambda (format-string &rest args)
                       (push (apply #'format format-string args) progress))))
            (should (eq (jaunder--call-with-blog
                         "/tmp" (lambda ()
                                  (jaunder--reconcile-execute-batch
                                   buffer rows 'pull #'jaunder--reconcile-pull-row)))
                        'completed))
            (let ((results jaunder-reconcile-last-batch-results)
                  (events (nreverse progress)))
              (should (equal (mapcar #'jaunder-reconcile-result-outcome results)
                             '(failed success)))
              (should (string-match-p "staging Member: .*read timed out"
                                      (jaunder-reconcile-result-detail (car results))))
              (dolist (stage '("1/2 Post 7 — staging Member"
                               "2/2 Post 8 — staging Member"
                               "2/2 Post 8 — verifying fresh Collection"
                               "2/2 Post 8 — revalidating Member"
                               "2/2 Post 8 — installing local Post"
                               "refreshing report after 2/2"
                               "batch complete after 2/2"))
                (should (cl-some (lambda (event) (string-match-p (regexp-quote stage) event))
                                 events))))))
      (kill-buffer buffer))))

(provide 'jaunder-reconcile-performance-test)
;;; jaunder-reconcile-performance-test.el ends here
