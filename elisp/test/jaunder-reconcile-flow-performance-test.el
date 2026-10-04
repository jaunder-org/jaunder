;;; jaunder-reconcile-flow-performance-test.el --- Measured selected pull -*- lexical-binding: t; -*-

;; Copyright (C) 2026 Jaunder contributors

;;; Commentary:
;; Exercise the actual selected server-ahead path with 100 paginated remote
;; Members.  Only the HTTP service/Member/Collection and public Media transfer
;; are deterministic doubles; staging, joining, verification, installation,
;; and the final report refresh remain real.

;;; Code:

(require 'ert)
(require 'cl-lib)
(require 'jaunder)

(ert-deftest jaunder-reconcile-selected-pull-measures-complete-paginated-flow ()
  "Three real matched pulls retain fresh checks through staging and refresh."
  (let* ((root (make-temp-file "jaunder-pull-flow-" t))
         (jaunder-blogs
          (list (cons (file-name-as-directory root)
                      '(:base-url "https://example.test" :username "alice"))))
         (ids '(1 50 100))
         (instance "123e4567-e89b-12d3-a456-426614174000")
         (media-bytes "fixture-image")
         (hash (secure-hash 'sha256 media-bytes))
         (media-url (format "https://example.test/media/upload/%s/%s/%s/asset.png"
                            (substring hash 0 2) (substring hash 2 4) hash))
         (etag "\"sha256-test\"")
         (source (lambda (id slug)
                   (format "#+TITLE: Before\n#+PROPERTY: JAUNDER_ID %d\n#+PROPERTY: JAUNDER_SLUG %s\n#+PROPERTY: JAUNDER_SYNCED \"old\"\n\nBefore.\n"
                           id slug)))
         (rows
          (mapcar
           (lambda (id)
             (let* ((slug (format "post-%03d" id))
                    (path (expand-file-name (concat slug ".org") root)))
               (with-temp-file path (insert (funcall source id slug)))
               (jaunder--make-reconcile-row
                :key (format "post:%d" id) :state 'server-ahead
                :remote-etag etag :local-sha256 (jaunder--reconcile-file-sha256 path)
                :local (jaunder--make-inventory-local
                        :path path :id (number-to-string id) :slug slug)
                :member (jaunder--make-inventory-member
                         :id (number-to-string id) :slug slug
                         :edit-uri (format "https://example.test/atompub/alice/posts/%d" id)))))
           ids))
         (buffer (jaunder--render-reconcile-report
                  (jaunder--make-reconcile-report :root root :rows rows)))
         (page-reads 0) (member-reads 0) (service-reads 0) (media-reads 0)
         (times (make-hash-table :test #'eq))
         (real-stage (symbol-function 'jaunder--pull-stage-member))
         (real-inventory (symbol-function 'jaunder--inventory-for-root))
         (real-revalidate (symbol-function 'jaunder--reconcile-pull-remote-revalidation))
         (real-install (symbol-function 'jaunder--reconcile-pull-install-staged))
         (real-refresh (symbol-function 'jaunder--reconcile-refresh-buffer))
         before-pages before-members before-service before-media before-times)
    (unwind-protect
        (with-current-buffer buffer
          (cl-letf (((symbol-function 'jaunder--http-request)
                     (lambda (method url &rest _)
                       (should (equal method "GET"))
                       (cond
                        ((string-suffix-p "/atompub/service" url)
                         (setq service-reads (1+ service-reads))
                         (list :status 200 :body
                               "<service xmlns=\"http://www.w3.org/2007/app\" xmlns:atom=\"http://www.w3.org/2005/Atom\"><workspace><atom:title>Blog</atom:title></workspace></service>"))
                        ((string-match "/atompub/alice/posts/\\([0-9]+\\)\\'" url)
                         (let* ((id (string-to-number (match-string 1 url)))
                                (slug (format "post-%03d" id))
                                (body (if (= id 1) (format "[[%s]]" media-url) "Remote body.")))
                           (setq member-reads (1+ member-reads))
                           (list :status 200
                                 :headers (list (cons "etag" etag)
                                                (cons "x-jaunder-instance" instance))
                                 :body (format
                                        "<entry xmlns=\"http://www.w3.org/2005/Atom\" xmlns:j=\"https://jaunder.org/ns/atompub\" xmlns:app=\"http://www.w3.org/2007/app\"><title>After</title><link rel=\"edit\" href=\"%s\"/><j:slug>%s</j:slug><content type=\"text/org\">%s</content><app:control><app:draft>yes</app:draft></app:control></entry>"
                                        url slug body))))
                        (t
                         (let* ((page (if (string-match "/page-\\([2-4]\\)\\'" url)
                                          (string-to-number (match-string 1 url)) 1))
                                (start (1+ (* 25 (1- page))))
                                (next (when (< page 4)
                                        (format "https://example.test/page-%d" (1+ page)))))
                           (setq page-reads (1+ page-reads))
                           (list :status 200
                                 :body (concat
                                        "<feed xmlns=\"http://www.w3.org/2005/Atom\" xmlns:j=\"https://jaunder.org/ns/atompub\">"
                                        (when next (format "<link rel=\"next\" href=\"%s\"/>" next))
                                        (mapconcat
                                         (lambda (id)
                                           (format "<entry><link rel=\"edit\" href=\"https://example.test/atompub/alice/posts/%d\"/><j:slug>post-%03d</j:slug><j:etag>&quot;sha256-test&quot;</j:etag></entry>" id id))
                                         (number-sequence start (+ start 24)) "")
                                        "</feed>")))))))
                    ((symbol-function 'jaunder--pull-media-get)
                     (lambda (_url destination)
                       (setq media-reads (1+ media-reads))
                       (let ((coding-system-for-write 'no-conversion))
                         (write-region media-bytes nil destination nil 'silent))
                       (list :status 200 :headers
                             (list (cons "x-jaunder-instance" instance)
                                   (cons "etag" (format "\"sha256-%s\"" hash))))))
                    ((symbol-function 'jaunder--pull-stage-member)
                     (lambda (directory member)
                       (let ((start (float-time)))
                         (prog1 (funcall real-stage directory member)
                           (puthash 'staging (+ (gethash 'staging times 0.0)
                                                (- (float-time) start)) times)))))
                    ((symbol-function 'jaunder--inventory-for-root)
                     (lambda (directory)
                       (let ((start (float-time)))
                         (prog1 (funcall real-inventory directory)
                           (puthash 'inventory (+ (gethash 'inventory times 0.0)
                                                  (- (float-time) start)) times)))))
                    ((symbol-function 'jaunder--reconcile-pull-remote-revalidation)
                     (lambda (row reviewed)
                       (let ((start (float-time)))
                         (prog1 (funcall real-revalidate row reviewed)
                           (puthash 'revalidation (+ (gethash 'revalidation times 0.0)
                                                     (- (float-time) start)) times)))))
                    ((symbol-function 'jaunder--reconcile-pull-install-staged)
                     (lambda (row staged remote path)
                       (let ((start (float-time)))
                         (prog1 (funcall real-install row staged remote path)
                           (puthash 'installation (+ (gethash 'installation times 0.0)
                                                     (- (float-time) start)) times)))))
                    ((symbol-function 'jaunder--reconcile-refresh-buffer)
                     (lambda (target)
                       (let ((start (float-time)))
                         (prog1 (funcall real-refresh target)
                           (puthash 'refresh (- (float-time) start) times))))))
            (setf (jaunder-reconcile-report-inventory jaunder-reconcile-report)
                  (jaunder--inventory-for-root root))
            (setq page-reads 0)
            (clrhash times)
            ;; Baseline: the same selected path without progress messages,
            ;; with the same final report refresh and all real safety checks.
            (jaunder--call-with-blog
             root (lambda ()
                    (dolist (row rows)
                      (should (eq (plist-get (jaunder--reconcile-pull-row row) :outcome)
                                  'success)))
                    (jaunder--reconcile-refresh-buffer buffer)))
            (setq before-pages page-reads before-members member-reads
                  before-service service-reads before-media media-reads
                  before-times (copy-hash-table times))
            (should (= before-pages 16))
            (should (= before-members 6))
            (should (= before-service 3))
            (should (= before-media 1))
            ;; A clean equivalent initial state makes the measured progress arm
            ;; repeat the same Media acquisition and local replacement.
            (dolist (id ids)
              (let ((slug (format "post-%03d" id)))
                (with-temp-file (expand-file-name (concat slug ".org") root)
                  (insert (funcall source id slug)))))
            (delete-directory (expand-file-name "local-media" root) t)
            (jaunder--render-reconcile-report
             (jaunder--make-reconcile-report
              :root root :rows rows :inventory (jaunder--inventory-for-root root)) buffer)
            (setq page-reads 0 member-reads 0 service-reads 0 media-reads 0)
            (clrhash times)
            (should (eq (jaunder--call-with-blog
                         root (lambda ()
                                (jaunder--reconcile-execute-batch
                                 buffer rows 'pull #'jaunder--reconcile-pull-row)))
                        'completed))
            (should (equal (mapcar #'jaunder-reconcile-result-outcome
                                   jaunder-reconcile-last-batch-results)
                           '(success success success)))
            (should (= page-reads 16))
            (should (= member-reads 6))
            (should (= service-reads 3))
            (should (= media-reads 1))
            (dolist (stage '(staging inventory revalidation installation refresh))
              (should (numberp (gethash stage times)))))
          (message "Selected pull 100/3 baseline: %d Collection, %d Member, %d service, %d Media GETs; staging %.3fs, inventory %.3fs, revalidation %.3fs, install %.3fs, refresh %.3fs."
                   before-pages before-members before-service before-media
                   (gethash 'staging before-times) (gethash 'inventory before-times)
                   (gethash 'revalidation before-times) (gethash 'installation before-times)
                   (gethash 'refresh before-times))
          (message "Selected pull 100/3 progress: %d Collection, %d Member, %d service, %d Media GETs; staging %.3fs, inventory %.3fs, revalidation %.3fs, install %.3fs, refresh %.3fs. Synthetic HTTP excludes network."
                   page-reads member-reads service-reads media-reads
                   (gethash 'staging times) (gethash 'inventory times)
                   (gethash 'revalidation times) (gethash 'installation times)
                   (gethash 'refresh times)))
      (when (buffer-live-p buffer) (kill-buffer buffer))
      (delete-directory root t))))

(provide 'jaunder-reconcile-flow-performance-test)
;;; jaunder-reconcile-flow-performance-test.el ends here
