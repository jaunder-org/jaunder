;;; jaunder-reconcile-integration.el --- live reconcile inventory tests -*- lexical-binding: t; -*-

;; Copyright (C) 2026 Jaunder contributors

;; This program is free software: you can redistribute it and/or modify
;; it under the terms of the GNU General Public License as published by
;; the Free Software Foundation, either version 3 of the License, or
;; (at your option) any later version.
;;
;; This program is distributed in the hope that it will be useful,
;; but WITHOUT ANY WARRANTY; without even the implied warranty of
;; MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
;; GNU General Public License for more details.
;;
;; You should have received a copy of the GNU General Public License
;; along with this program.  If not, see <https://www.gnu.org/licenses/>.

;;; Commentary:
;; Live reconciliation inventory coverage.

;;; Code:

(require 'cl-lib)
(require 'ert)
(require 'jaunder)
(require 'jaunder-integration-helper)


(ert-deftest jaunder-reconcile-inventory-exhausts-collection-pagination ()
  "Inventory finds each newly-created Member beyond the first Collection page."
  ;; Real authenticated transport crosses the server's 25-Member page boundary.
  ;; Assert only this test's returned IDs, so the shared live server's pre-existing
  ;; Members and any title/slug collisions outside this temporary root are tolerated.
  (jaunder-test--with-live-server
   (let* ((root (make-temp-file "jaunder-reconcile-inventory-" t))
          (jaunder-blogs
           (list (cons (file-name-as-directory root)
                       (list :base-url jaunder-test-base-url
                             :username jaunder-test-username))))
          (token (file-name-nondirectory (directory-file-name root))))
     (unwind-protect
         (jaunder--call-with-blog
          root
          (lambda ()
            (let ((created-ids
                   (mapcar
                    (lambda (index)
                      (let* ((response
                              (jaunder--http-request
                               "POST"
                               (jaunder--build-url (jaunder--active-base-url)
                                                   "atompub"
                                                   (jaunder--active-username)
                                                   "posts")
                               (jaunder--atom-entry->xml
                                (jaunder--make-entry
                                 :title (format "inventory-pagination-%s-%d" token index)
                                 :content-type "text/org"
                                 :body (format "Inventory pagination body %d." index)))
                               "application/atom+xml"))
                             (location (jaunder--response-header response "Location")))
                        (should (eq (plist-get response :status) 201))
                        (should (string-match "/\\([0-9]+\\)/?\\'" location))
                        (match-string 1 location)))
                    (number-sequence 1 26))))
              (let* ((inventory (jaunder--inventory-for-root root))
                     (server-only-ids
                      (mapcar #'jaunder-inventory-member-id
                              (jaunder-inventory-server-only inventory))))
                (dolist (id created-ids)
                  (should (= (cl-count id server-only-ids :test #'equal) 1)))
                ;; Make two Posts on separate Collection pages local matches. The
                ;; real server's Entry validators must remove the Member GET fanout.
                (dolist (id (list (car created-ids) (car (last created-ids))))
                  (let ((member (cl-find id (jaunder-inventory-server-only inventory)
                                         :key #'jaunder-inventory-member-id :test #'equal)))
                    (should (jaunder--strong-etag-p (jaunder-inventory-member-etag member)))
                    (jaunder-reconcile-live--write-local
                     root (concat (jaunder-inventory-member-slug member) ".org")
                     "Matched" id)))
                (let ((real-request (symbol-function 'jaunder--http-request))
                      (member-reads 0))
                  (cl-letf (((symbol-function 'jaunder--http-request)
                             (lambda (method url &rest args)
                               (when (and (equal method "GET")
                                          (string-match-p "/posts/[0-9]+\\'" url))
                                 (setq member-reads (1+ member-reads)))
                               (apply real-request method url args))))
                    (let* ((report (jaunder--reconcile-build-report
                                    root (jaunder--inventory-for-root root)))
                           (rows (jaunder-reconcile-report-rows report)))
                      (should (= (length (jaunder-inventory-matched
                                          (jaunder-reconcile-report-inventory report))) 2))
                      (should (= (length (cl-remove-if-not
                                          #'jaunder-reconcile-row-local rows)) 2))))
                  (should (= member-reads 0)))))))
       (delete-directory root t)))))

(ert-deftest jaunder-reconcile-inventory-remains-a-server-only-preview ()
  "The live inventory leaves a server-only Member untouched until an explicit action."
  (jaunder-test--with-live-server
   (let* ((root (make-temp-file "jaunder-reconcile-live-" t))
          (jaunder-blogs
           (list (cons (file-name-as-directory root)
                       (list :base-url jaunder-test-base-url
                             :username jaunder-test-username))))
          (token (file-name-nondirectory (directory-file-name root))))
     (unwind-protect
         (jaunder--call-with-blog
          root
          (lambda ()
            (let* ((created
                    (jaunder--http-request
                     "POST" (jaunder--collection-url)
                     (jaunder--atom-entry->xml
                      (jaunder--make-entry :title (concat "reconcile-" token)
                                           :content-type "text/org"
                                           :body "Server-only reconciliation fixture."))
                     "application/atom+xml"))
                   (location (jaunder--response-header created "Location")))
              (should (eq (plist-get created :status) 201))
              (should (string-match "/\\([0-9]+\\)/?\\'" location))
              (let* ((id (match-string 1 location))
                     (preview-inventory (jaunder--inventory-for-root root))
                     (preview-member
                      (cl-find id (jaunder-inventory-server-only preview-inventory)
                               :key #'jaunder-inventory-member-id :test #'equal))
                     (slug (and preview-member
                                (jaunder-inventory-member-slug preview-member))))
                (should preview-member)
                (cl-letf (((symbol-function 'y-or-n-p)
                           (lambda (&rest _) (error "inventory must not prompt"))))
                  (jaunder-reconcile root))
                (let ((inventory (jaunder--inventory-for-root root)))
                  (should (cl-find id (jaunder-inventory-server-only inventory)
                                   :key #'jaunder-inventory-member-id :test #'equal))
                  (should-not (directory-files root nil "\\.org\\'"))
                  (with-current-buffer "*Jaunder Reconcile*"
                    (should (string-match-p (regexp-quote slug) (buffer-string)))))))))
       (delete-directory root t)))))

(defun jaunder-reconcile-live--write-local (root name title &optional id)
  "Write one direct-root Org Post under ROOT and return its path."
  (let ((path (expand-file-name name root)))
    (with-temp-file path
      (insert (format "#+TITLE: %s\n#+PROPERTY: JAUNDER_STATUS published\n" title)
              (or (when id (format "#+PROPERTY: JAUNDER_ID %s\n" id)) "")
              "\nBody.\n"))
    path))

(defun jaunder-reconcile-live--create-pagination-member (title body &optional draft)
  "Create TITLE/BODY remotely and return its Location Post ID.
When DRAFT is non-nil, create a draft Member."
  (let* ((response
          (jaunder--http-request
           "POST" (jaunder--collection-url)
           (jaunder--atom-entry->xml
            (jaunder--make-entry :title title :draft draft :content-type "text/org" :body body))
           "application/atom+xml"))
         (location (jaunder--response-header response "Location")))
    (should (eq (plist-get response :status) 201))
    (should (string-match "/\\([0-9]+\\)/?\\'" location))
    (match-string 1 location)))

(defun jaunder-reconcile-live--member-representation (response)
  "Return RESPONSE's exact title, content type, and native text representation."
  (let* ((fields (jaunder--harvest-response-fields (plist-get response :body)))
         (title (car (cdr (assq 'titles fields))))
         (content (car (cdr (assq 'content-nodes fields)))))
    (list :title title
          :content-type (dom-attr content 'type)
          :body (dom-inner-text content))))

(ert-deftest jaunder-reconcile-detects-audience-only-remote-change ()
  "Audience-only ETag movement produces server-ahead, then conflict with local edits."
  (jaunder-test--with-live-server
   (let* ((root (file-name-as-directory (make-temp-file "jaunder-audience-reconcile-" t)))
          (path (expand-file-name "audience.org" root))
          (jaunder-blogs (list (cons root (list :base-url jaunder-test-base-url
                                                :username jaunder-test-username))))
          buffer)
     (unwind-protect
         (progn
           (with-temp-file path
             (insert (concat "#+TITLE: Audience reconcile\n"
                             "#+PROPERTY: JAUNDER_STATUS published\n"
                             "#+PROPERTY: JAUNDER_AUDIENCE public\n\nBody.\n")))
           (setq buffer (find-file-noselect path))
           (with-current-buffer buffer (jaunder-publish) (save-buffer))
           (setq path (buffer-file-name buffer))
           (set-file-times path (time-subtract (current-time) (seconds-to-time 5)))
           (jaunder--call-with-blog
            root
            (lambda ()
              (let* ((id (with-current-buffer buffer
                           (jaunder--buffer-property "JAUNDER_ID")))
                     (old-etag (with-current-buffer buffer
                                 (jaunder--buffer-property "JAUNDER_SYNCED")))
                     (updated
                      (jaunder--http-request
                       "PUT" (jaunder--member-url id)
                       (jaunder--atom-entry->xml
                        (jaunder--make-entry
                         :title "Audience reconcile" :audiences '("private")
                         :content-type "text/org" :body "Body."))
                       "application/atom+xml"
                       (list (cons "If-Match" old-etag))))
                     (new-etag (jaunder--response-header updated "ETag")))
                (should (eq (plist-get updated :status) 200))
                (should-not (equal old-etag new-etag))
                (let* ((report
                        (jaunder--reconcile-build-report
                         root (jaunder--inventory-for-root root)))
                       (row
                        (cl-find
                         id (jaunder-reconcile-report-rows report)
                         :key (lambda (candidate)
                                (let ((member (jaunder-reconcile-row-member candidate)))
                                  (and member (jaunder-inventory-member-id member))))
                         :test #'equal)))
                  (should (eq (jaunder-reconcile-row-state row) 'server-ahead)))
                (with-current-buffer buffer
                  (goto-char (point-max))
                  (insert "Local change.\n")
                  (save-buffer))
                (set-file-times path (time-add (current-time) (seconds-to-time 5)))
                (let* ((report
                        (jaunder--reconcile-build-report
                         root (jaunder--inventory-for-root root)))
                       (row
                        (cl-find
                         id (jaunder-reconcile-report-rows report)
                         :key (lambda (candidate)
                                (let ((member (jaunder-reconcile-row-member candidate)))
                                  (and member (jaunder-inventory-member-id member))))
                         :test #'equal)))
                  (should (eq (jaunder-reconcile-row-state row) 'conflict)))))))
       (when (buffer-live-p buffer)
         (with-current-buffer buffer (set-buffer-modified-p nil))
         (kill-buffer buffer))
       (delete-directory root t)))))

(defun jaunder-reconcile-live--run-selected (root keys command)
  "Build ROOT's report, mark KEYS, confirm COMMAND, and return its results/prompt."
  (jaunder-reconcile root)
  (with-current-buffer "*Jaunder Reconcile*"
    (dolist (key keys) (puthash key t jaunder-reconcile-marks))
    (let (prompt)
      (cl-letf (((symbol-function 'y-or-n-p)
                 (lambda (text) (setq prompt text) t)))
        (funcall command))
      (list :prompt prompt :results jaunder-reconcile-last-batch-results))))

(ert-deftest jaunder-reconcile-live-selected-pull-crosses-pagination-and-isolates-failure ()
  "Selected pull handles a paginated server-only Post, a server-ahead Post, and failure."
  (jaunder-test--with-live-server
   (let* ((root (file-name-as-directory (make-temp-file "jaunder-reconcile-pull-live-" t)))
          (nested (expand-file-name "staging/" root))
          (token (file-name-nondirectory (directory-file-name root)))
          (jaunder-blogs (list (cons root (list :base-url jaunder-test-base-url
                                                :username jaunder-test-username))))
          (local (jaunder-reconcile-live--write-local root
                                                      (concat "matched-" token ".org")
                                                      (concat "matched-" token)))
          local-buffer shadow ids)
     (unwind-protect
         (jaunder--call-with-blog
          root
          (lambda ()
            ;; Publish once, then update through a nested second working copy so
            ;; the direct-root original is genuinely server-ahead.
            (setq local-buffer (find-file-noselect local))
            (with-current-buffer local-buffer (jaunder-publish) (save-buffer))
            (setq local (buffer-file-name local-buffer))
            (make-directory nested)
            (setq shadow (expand-file-name "remote-editor.org" nested))
            (copy-file local shadow)
            (with-current-buffer (find-file-noselect shadow)
              (jaunder--replace-audience-properties '("public" "subscribers"))
              (goto-char (point-max)) (insert "Remote change.\n")
              (jaunder-publish) (save-buffer) (set-buffer-modified-p nil))
            ;; 26 creates force at least two 25-Member Collection pages.
            (setq ids (mapcar (lambda (index)
                                (jaunder-reconcile-live--create-pagination-member
                                 (format "pull-page-%s-%d" token index) "Server-only body."))
                              (number-sequence 1 26)))
            (let* ((inventory (jaunder--inventory-for-root root))
                   (server-id (car (last ids)))
                   (server-member (cl-find server-id (jaunder-inventory-server-only inventory)
                                           :key #'jaunder-inventory-member-id :test #'equal))
                   (matched-id (with-current-buffer local-buffer
                                 (jaunder--buffer-property "JAUNDER_ID")))
                   (matched-member (cl-find matched-id (jaunder-inventory-matched inventory)
                                            :key (lambda (match)
                                                   (jaunder-inventory-member-id
                                                    (jaunder-inventory-match-member match)))
                                            :test #'equal)))
              (should server-member)
              (dolist (id ids)
                (should (cl-find id (jaunder-inventory-server-only inventory)
                                 :key #'jaunder-inventory-member-id :test #'equal)))
              ;; An occupied server-only target is an independent first failure;
              ;; the following selected server-only and matched pulls continue.
              (with-temp-file (expand-file-name
                               (concat (jaunder-inventory-member-slug server-member) ".org") root)
                (insert "occupied"))
              (let* ((success-id (nth 24 ids))
                     (outcome (jaunder-reconcile-live--run-selected
                               root (list (format "post:%s" server-id)
                                          (format "post:%s" success-id)
                                          (format "post:%s" matched-id))
                               #'jaunder-reconcile-pull-selected))
                     (results (plist-get outcome :results)))
                (should (string-match-p "Pull 3 selected" (plist-get outcome :prompt)))
                (should (equal (mapcar #'jaunder-reconcile-result-outcome results)
                               '(success blocked success)))
                (should (equal (mapcar #'jaunder-reconcile-result-post-id results)
                               (list matched-id server-id success-id)))
                (let ((matched-result (car results))
                      (server-result (nth 2 results))
                      (success-member
                       (cl-find success-id (jaunder-inventory-server-only inventory)
                                :key #'jaunder-inventory-member-id :test #'equal)))
                  (should (equal (jaunder-reconcile-result-post-id matched-result) matched-id))
                  (should (equal (jaunder-reconcile-result-slug matched-result)
                                 (jaunder-inventory-member-slug
                                  (jaunder-inventory-match-member matched-member))))
                  (should (jaunder--strong-etag-p (jaunder-reconcile-result-etag matched-result)))
                  (should (stringp (jaunder-reconcile-result-synced-at matched-result)))
                  (should (= (jaunder-reconcile-result-http-status matched-result) 200))
                  (should (eq (jaunder-reconcile-result-local-effect matched-result) 'replaced))
                  (should (equal (jaunder-reconcile-result-post-id server-result) success-id))
                  (should (equal (jaunder-reconcile-result-slug server-result)
                                 (jaunder-inventory-member-slug success-member)))
                  (should (jaunder--strong-etag-p (jaunder-reconcile-result-etag server-result)))
                  (should (stringp (jaunder-reconcile-result-synced-at server-result)))
                  (should (= (jaunder-reconcile-result-http-status server-result) 200))
                  (should (eq (jaunder-reconcile-result-local-effect server-result) 'created)))
                (let ((pulled-path
                       (expand-file-name
                        (concat (jaunder-inventory-member-slug
                                 (cl-find success-id (jaunder-inventory-server-only inventory)
                                          :key #'jaunder-inventory-member-id :test #'equal))
                                ".org") root)))
                  (should (file-exists-p pulled-path))
                  (should (string-match-p
                           "^#\\+PROPERTY: JAUNDER_AUDIENCE private$"
                           (with-temp-buffer
                             (insert-file-contents pulled-path) (buffer-string)))))
                (with-current-buffer local-buffer
                  (should (string-match-p "Remote change" (buffer-string)))
                  (should (string-match-p
                           (regexp-quote
                            (concat "#+PROPERTY: JAUNDER_AUDIENCE public\n"
                                    "#+PROPERTY: JAUNDER_AUDIENCE subscribers\n"))
                           (buffer-string)))
                  (should-not (string-match-p "JAUNDER_AUDIENCE private"
                                              (buffer-string)))
                  (should-not (buffer-modified-p))))))))
     (dolist (buffer (list local-buffer (get-file-buffer shadow)))
       (when (buffer-live-p buffer)
         (with-current-buffer buffer (set-buffer-modified-p nil))
         (kill-buffer buffer)))
     (delete-directory root t))))

(ert-deftest jaunder-reconcile-live-selected-pull-stale-etag-preserves-matched-post ()
  "A final live ETag race leaves the selected matched file and visiting buffer intact."
  (jaunder-test--with-live-server
   (let* ((root (file-name-as-directory (make-temp-file "jaunder-pull-stale-live-" t)))
          (nested (expand-file-name "remote/" root))
          (token (file-name-nondirectory (directory-file-name root)))
          (title (concat "stale-" token))
          (jaunder-blogs (list (cons root (list :base-url jaunder-test-base-url
                                                :username jaunder-test-username))))
          (path (jaunder-reconcile-live--write-local root (concat title ".org") title))
          local remote)
     (unwind-protect
         (jaunder--call-with-blog
          root
          (lambda ()
            (setq local (find-file-noselect path))
            (with-current-buffer local (jaunder-publish) (save-buffer))
            (make-directory nested)
            (setq path (buffer-file-name local)
                  remote (expand-file-name "remote.org" nested))
            (copy-file path remote)
            ;; Establish the reviewed server-ahead state with a separate working copy.
            (with-current-buffer (find-file-noselect remote)
              (goto-char (point-max)) (insert "First remote change.\n")
              (jaunder-publish) (save-buffer) (set-buffer-modified-p nil)
              (setq remote (buffer-file-name)))
            (jaunder-reconcile root)
            (with-current-buffer "*Jaunder Reconcile*"
              (let* ((id (with-current-buffer local (jaunder--buffer-property "JAUNDER_ID")))
                     (before (with-temp-buffer (insert-file-contents-literally path) (buffer-string)))
                     (real-http (symbol-function 'jaunder--http-request))
                     (member-gets 0)
                     racing)
                (puthash (format "post:%s" id) t jaunder-reconcile-marks)
                ;; Inject at the real final Member HTTP boundary.  Staging has
                ;; already fetched the Member; this is the second Member GET.
                (cl-letf (((symbol-function 'jaunder--http-request)
                           (lambda (method url &rest arguments)
                             (when (and (not racing) (equal method "GET")
                                        (equal url (jaunder--member-url id)))
                               (setq member-gets (1+ member-gets))
                               (when (= member-gets 2)
                                 (let ((racing t))
                                   (with-current-buffer (find-file-noselect remote)
                                     (goto-char (point-max))
                                     (insert "Final remote race.\n")
                                     (jaunder-publish)
                                     (save-buffer)
                                     (set-buffer-modified-p nil)))))
                             (apply real-http method url arguments)))
                          ((symbol-function 'y-or-n-p) (lambda (_) t)))
                  (jaunder-reconcile-pull-selected))
                (let ((result (car jaunder-reconcile-last-batch-results)))
                  (should (eq (jaunder-reconcile-result-outcome result) 'blocked))
                  (should (eq (jaunder-reconcile-result-reason result) 'etag-stale))
                  (should (integerp (jaunder-reconcile-result-http-status result)))
                  (should (jaunder-reconcile-result-etag result)))
                (should (equal (with-temp-buffer (insert-file-contents-literally path) (buffer-string))
                               before))
                (with-current-buffer local
                  (should-not (buffer-modified-p))
                  (should (equal (buffer-file-name) path)))
                (let ((remote-member (jaunder--http-request "GET" (jaunder--member-url id))))
                  (should (eq (plist-get remote-member :status) 200))
                  (should (string-match-p "Final remote race" (plist-get remote-member :body))))))))
       (dolist (buffer (list local (get-file-buffer remote)))
         (when (buffer-live-p buffer) (with-current-buffer buffer (set-buffer-modified-p nil))
               (kill-buffer buffer)))
       (delete-directory root t)))))

(ert-deftest jaunder-reconcile-live-selected-pull-continues-after-staging-failure ()
  "A media trust failure retains safe partial Local Media Copies and continues."
  (jaunder-test--with-live-server
   (let* ((root (file-name-as-directory (make-temp-file "jaunder-pull-failure-live-" t)))
          (first-source (expand-file-name "first.png" root))
          (failed-source (expand-file-name "failed.png" root))
          (success-source (expand-file-name "success.png" root))
          (jaunder-blogs (list (cons root (list :base-url jaunder-test-base-url
                                                :username jaunder-test-username))))
          failed-id success-id first-url failed-url success-url)
     (unwind-protect
         (jaunder--call-with-blog
          root
          (lambda ()
            (with-temp-file first-source (insert "verified partial media"))
            (with-temp-file failed-source (insert "rejected media"))
            (with-temp-file success-source (insert "successful media"))
            (setq first-url (jaunder--upload-media first-source "image/png")
                  failed-url (jaunder--upload-media failed-source "image/png")
                  success-url (jaunder--upload-media success-source "image/png"))
            (should-not (equal first-url failed-url))
            (should-not (equal failed-url success-url))
            (should (= (length (jaunder-pull-media-plan-references
                                (jaunder--pull-media-plan
                                 "org" (format "[[%s]]\n[[%s]]" first-url failed-url)
                                 (jaunder--active-base-url))))
                       2))
            ;; Collection order is newest first, so create the successful Member
            ;; first and the Member whose real media response will fail.
            (setq success-id
                  (jaunder-inventory-member-id
                   (jaunder-pull-integration--create-server-only-member
                    root (format "[[%s]]" success-url)))
                  failed-id
                  (jaunder-inventory-member-id
                   (jaunder-pull-integration--create-server-only-member
                    root (format "[[%s]]\n[[%s]]" first-url failed-url))))
            (jaunder-reconcile root)
            (with-current-buffer "*Jaunder Reconcile*"
              (let ((real-media-get (symbol-function 'jaunder--pull-media-get))
                    (media-gets 0)
                    (failed-response-seen nil))
                (puthash (format "post:%s" failed-id) t jaunder-reconcile-marks)
                (puthash (format "post:%s" success-id) t jaunder-reconcile-marks)
                ;; Stage the actual Member and Media.  Only the actual media
                ;; response boundary for FAILED-URL is made untrustworthy.
                (cl-letf (((symbol-function 'jaunder--pull-media-get)
                           (lambda (url destination)
                             (let ((response (funcall real-media-get url destination)))
                               (setq media-gets (1+ media-gets))
                               (if (equal url failed-url)
                                   (progn
                                     (setq failed-response-seen t)
                                     (plist-put response :status 503))
                                 response))))
                          ((symbol-function 'y-or-n-p) (lambda (_) t)))
                  (jaunder-reconcile-pull-selected))
                (let ((results jaunder-reconcile-last-batch-results))
                  (should failed-response-seen)
                  (should (equal (mapcar #'jaunder-reconcile-result-post-id results)
                                 (list failed-id success-id)))
                  (should (equal (mapcar #'jaunder-reconcile-result-outcome results)
                                 '(failed success)))
                  (should (eq (jaunder-reconcile-result-reason (car results)) 'pull-failed))
                  (should (file-exists-p
                           (expand-file-name
                            (concat (jaunder-reconcile-result-slug (cadr results)) ".org")
                            root)))
                  ;; The later successful item retains its verified Local Media
                  ;; Copy despite the earlier item's failed staging.
                  (should (= (length (directory-files-recursively
                                      (expand-file-name "local-media" root) "."))
                             1))))))))
     (delete-directory root t))))

(ert-deftest jaunder-reconcile-live-selected-push-mixed-batch ()
  "Selected push creates, updates, no-ops, blocks, and keeps ordered results."
  (jaunder-test--with-live-server
   (let* ((root (make-temp-file "jaunder-reconcile-push-live-" t))
          (token (file-name-nondirectory (directory-file-name root)))
          (jaunder-blogs (list (cons (file-name-as-directory root)
                                     (list :base-url jaunder-test-base-url
                                           :username jaunder-test-username))))
          (draft-title (concat "draft-" token))
          (matched-title (concat "matched-" token))
          (unchanged-title (concat "unchanged-" token))
          (draft (jaunder-reconcile-live--write-local root (concat draft-title ".org") draft-title))
          (draft-buffer (find-file-noselect draft))
          (matched (jaunder-reconcile-live--write-local root (concat matched-title ".org") matched-title))
          (unchanged (jaunder-reconcile-live--write-local
                      root (concat unchanged-title ".org") unchanged-title))
          (matched-buffer (find-file-noselect matched))
          (unchanged-buffer (find-file-noselect unchanged)))
     (unwind-protect
         (progn
           (dolist (buffer (list matched-buffer unchanged-buffer))
             (with-current-buffer buffer (jaunder-publish)))
           (with-current-buffer matched-buffer
             (goto-char (point-max)) (insert "Local change.\n")
             (jaunder--set-property "JAUNDER_LOCAL_AHEAD" "true") (save-buffer))
           (let* ((orphan (jaunder-reconcile-live--write-local root "orphan.org" "orphan" "999999"))
                  (unchanged-before
                   (jaunder-reconcile-live--member-representation
                    (jaunder--http-request
                     "GET" (jaunder--member-url
                            (with-current-buffer unchanged-buffer
                              (jaunder--buffer-property "JAUNDER_ID"))))))
                  (outcome (jaunder-reconcile-live--run-selected
                            root (list (format "local:%s" draft)
                                       (format "post:%s" (with-current-buffer matched-buffer
                                                           (jaunder--buffer-property "JAUNDER_ID")))
                                       (format "post:%s" (with-current-buffer unchanged-buffer
                                                           (jaunder--buffer-property "JAUNDER_ID")))
                                       (format "local:%s" orphan))
                            #'jaunder-reconcile-push-selected))
                  (results (plist-get outcome :results)))
             (should (string-match-p "Push 4 selected" (plist-get outcome :prompt)))
             (should (equal (mapcar #'jaunder-reconcile-result-outcome results)
                            '(no-op success blocked success)))
             (should (equal (mapcar #'jaunder-reconcile-result-post-id results)
                            (list (with-current-buffer unchanged-buffer
                                    (jaunder--buffer-property "JAUNDER_ID"))
                                  (with-current-buffer matched-buffer
                                    (jaunder--buffer-property "JAUNDER_ID"))
                                  nil
                                  (with-current-buffer draft-buffer
                                    (jaunder--buffer-property "JAUNDER_ID")))))
             (should (equal (mapcar #'jaunder-reconcile-result-http-status results)
                            '(nil 200 nil 201)))
             (should (equal (mapcar #'jaunder-reconcile-result-local-effect results)
                            '(unchanged updated unchanged created)))
             (let ((unchanged-id (jaunder-reconcile-result-post-id (nth 0 results)))
                   (matched-id (jaunder-reconcile-result-post-id (nth 1 results)))
                   (draft-id (jaunder-reconcile-result-post-id (nth 3 results))))
               (let ((draft-response (jaunder--http-request "GET" (jaunder--member-url draft-id)))
                     (matched-response (jaunder--http-request "GET" (jaunder--member-url matched-id)))
                     (unchanged-response (jaunder--http-request "GET" (jaunder--member-url unchanged-id))))
                 (should (eq (plist-get draft-response :status) 200))
                 (should (equal (jaunder-reconcile-live--member-representation draft-response)
                                (list :title draft-title :content-type "text/org" :body "Body.\n")))
                 (should (eq (plist-get matched-response :status) 200))
                 (should (equal (jaunder-reconcile-live--member-representation matched-response)
                                (list :title matched-title :content-type "text/org"
                                      :body "Body.\nLocal change.\n")))
                 (should (eq (plist-get unchanged-response :status) 200))
                 (should (equal (jaunder-reconcile-live--member-representation unchanged-response)
                                unchanged-before)))
               (should (eq (plist-get (jaunder--http-request "GET" (jaunder--member-url "999999"))
                                      :status)
                           404)))))
       (delete-directory root t)))))

(defun jaunder-reconcile-live--create-server-only ()
  "Create a live server-only Post and return its decimal ID."
  (let* ((response (jaunder--http-request
                    "POST" (jaunder--collection-url)
                    (jaunder--atom-entry->xml
                     (jaunder--make-entry :title "server" :content-type "text/org" :body "Body."))
                    "application/atom+xml"))
         (location (jaunder--response-header response "Location")))
    (should (eq (plist-get response :status) 201))
    (string-match "/\\([0-9]+\\)/?\\'" location)
    (match-string 1 location)))

(defun jaunder-reconcile-live--advance-member (id)
  "Make a real concurrent Member change to ID so a reviewed ETag becomes stale."
  (let* ((url (jaunder--member-url id))
         (current (jaunder--http-request "GET" url))
         (xml (replace-regexp-in-string "Body\\." "Changed." (plist-get current :body))))
    (should (eq (plist-get current :status) 200))
    (should (eq (plist-get
                 (jaunder--http-request "PUT" url xml "application/atom+xml"
                                        (list (cons "If-Match"
                                                    (jaunder--response-header current "ETag"))))
                 :status)
                200))))

(ert-deftest jaunder-reconcile-live-selected-delete-stale-and-continues ()
  "A stale selected delete yields 412 while the later selected delete succeeds."
  (jaunder-test--with-live-server
   (let* ((root (make-temp-file "jaunder-reconcile-stale-live-" t))
          (jaunder-blogs (list (cons (file-name-as-directory root)
                                     (list :base-url jaunder-test-base-url
                                           :username jaunder-test-username))))
          (local (jaunder-reconcile-live--write-local root "matched.org" "matched")))
     (unwind-protect
         (jaunder--call-with-blog root
                                  (lambda ()
                                    (with-current-buffer (find-file-noselect local) (jaunder-publish))
                                    (let* ((matched-id (with-current-buffer (find-file-noselect local)
                                                         (jaunder--buffer-property "JAUNDER_ID")))
                                           (later-id (jaunder-reconcile-live--create-server-only)))
                                      (jaunder-reconcile root)
                                      (with-current-buffer "*Jaunder Reconcile*"
                                        (puthash (format "post:%s" matched-id) t jaunder-reconcile-marks)
                                        (puthash (format "post:%s" later-id) t jaunder-reconcile-marks)
                                        (cl-letf (((symbol-function 'y-or-n-p)
                                                   (lambda (_) (jaunder-reconcile-live--advance-member matched-id) t)))
                                          (jaunder-reconcile-delete-selected))
                                        (let ((results jaunder-reconcile-last-batch-results))
                                          (should (equal (mapcar #'jaunder-reconcile-result-outcome results)
                                                         '(failed success)))
                                          (should (= (jaunder-reconcile-result-http-status (car results)) 412))
                                          (let ((stale-member
                                                 (jaunder--http-request
                                                  "GET" (jaunder--member-url matched-id))))
                                            (should (eq (plist-get stale-member :status) 200))
                                            (should (string-match-p "Changed\\."
                                                                    (plist-get stale-member :body))))
                                          (should (file-exists-p local)))))))
       (delete-directory root t)))))

(ert-deftest jaunder-reconcile-live-selected-delete-server-and-matched ()
  "Selected delete soft-deletes server-only and removes matched file after 204."
  (jaunder-test--with-live-server
   (let* ((root (make-temp-file "jaunder-reconcile-delete-live-" t))
          (token (file-name-nondirectory (directory-file-name root)))
          (jaunder-blogs (list (cons (file-name-as-directory root)
                                     (list :base-url jaunder-test-base-url
                                           :username jaunder-test-username))))
          (title (concat "matched-" token))
          (local (jaunder-reconcile-live--write-local root (concat title ".org") title))
          (local-buffer (find-file-noselect local)))
     (unwind-protect
         (jaunder--call-with-blog root
                                  (lambda ()
                                    (with-current-buffer local-buffer (jaunder-publish))
                                    (let* ((matched-id (with-current-buffer local-buffer
                                                         (jaunder--buffer-property "JAUNDER_ID")))
                                           (created (jaunder--http-request
                                                     "POST" (jaunder--collection-url)
                                                     (jaunder--atom-entry->xml
                                                      (jaunder--make-entry :title "server" :content-type "text/org" :body "Body."))
                                                     "application/atom+xml"))
                                           (location (jaunder--response-header created "Location")))
                                      (string-match "/\\([0-9]+\\)/?\\'" location)
                                      (let* ((server-id (match-string 1 location))
                                             (outcome (jaunder-reconcile-live--run-selected
                                                       root (list (format "post:%s" server-id)
                                                                  (format "post:%s" matched-id))
                                                       #'jaunder-reconcile-delete-selected))
                                             (results (plist-get outcome :results)))
                                        (should (string-match-p "SOFT-DELETE" (plist-get outcome :prompt)))
                                        (should (string-match-p "retains server tombstones" (plist-get outcome :prompt)))
                                        (should (= (length results) 2))
                                        (should (equal (mapcar #'jaunder-reconcile-result-outcome results)
                                                       '(success success)))
                                        (should (equal (mapcar #'jaunder-reconcile-result-post-id results)
                                                       (list matched-id server-id)))
                                        (should (equal (mapcar #'jaunder-reconcile-result-http-status results)
                                                       '(204 204)))
                                        (should (equal (mapcar #'jaunder-reconcile-result-local-effect results)
                                                       '(removed unchanged)))
                                        (should-not (file-exists-p local))
                                        (should (eq (plist-get
                                                     (jaunder--http-request "GET" (jaunder--member-url server-id))
                                                     :status)
                                                    404))
                                        (should (eq (plist-get
                                                     (jaunder--http-request "GET" (jaunder--member-url matched-id))
                                                     :status)
                                                    404))))))
       (delete-directory root t)))))

(ert-deftest jaunder-reconcile-live-selected-push-replays-response-lost-create ()
  "Selected push replays a durable create intent and recovers its original ID."
  (jaunder-test--with-live-server
   (let* ((root (make-temp-file "jaunder-reconcile-replay-live-" t))
          (jaunder-blogs (list (cons (file-name-as-directory root)
                                     (list :base-url jaunder-test-base-url
                                           :username jaunder-test-username))))
          (path (jaunder-reconcile-live--write-local root "draft.org" "replay"))
          (buffer (find-file-noselect path)))
     (unwind-protect
         (jaunder--call-with-blog
          root
          (lambda ()
            (let* ((baseline-ids (mapcar #'jaunder-inventory-member-id
                                         (jaunder--fetch-collection-members)))
                   (entry (with-current-buffer buffer (jaunder--org->atom)))
                   (xml (jaunder--atom-entry->xml entry))
                   (intent (with-current-buffer buffer (jaunder--create-intent xml)))
                   (initial (jaunder--http-request
                             "POST" (jaunder--collection-url) xml jaunder--entry-content-type
                             (list (cons "Idempotency-Key" (plist-get intent :key)))))
                   (location (jaunder--response-header initial "Location")))
              ;; The remote create committed, but simulate losing its response before
              ;; the ordinary publish path can write the returned identity locally.
              (should (eq (plist-get initial :status) 201))
              (should (string-match "/\\([0-9]+\\)/?\\'" location))
              (let ((original-id (match-string 1 location)))
                (with-current-buffer buffer
                  (should-not (jaunder--buffer-property "JAUNDER_ID"))
                  (should (jaunder--buffer-property "JAUNDER_CREATE_KEY")))
                (let* ((outcome (jaunder-reconcile-live--run-selected
                                 root (list (format "local:%s" path))
                                 #'jaunder-reconcile-push-selected))
                       (result (car (plist-get outcome :results)))
                       (members (jaunder--fetch-collection-members)))
                  (should (eq (jaunder-reconcile-result-outcome result) 'success))
                  (should (equal (jaunder-reconcile-result-post-id result) original-id))
                  (should (= (jaunder-reconcile-result-http-status result) 200))
                  (should (eq (jaunder-reconcile-result-local-effect result) 'created))
                  (should (equal (sort (mapcar #'jaunder-inventory-member-id members) #'string<)
                                 (sort (cons original-id baseline-ids) #'string<)))
                  (should (eq (plist-get
                               (jaunder--http-request "GET" (jaunder--member-url original-id))
                               :status)
                              200))
                  (with-current-buffer buffer
                    (should (equal (jaunder--buffer-property "JAUNDER_ID") original-id))
                    (should-not (jaunder--buffer-property "JAUNDER_CREATE_KEY"))))))))
       (when (buffer-live-p buffer)
         (with-current-buffer buffer (set-buffer-modified-p nil))
         (kill-buffer buffer))
       (delete-directory root t)))))

(defun jaunder-reconcile-live--assert-rendered-last-batch (results)
  "Assert RESULTS appear in the report's exact ordered Last batch rendering."
  (let ((expected
         (concat "Last batch\n"
                 (mapconcat
                  (lambda (result)
                    (with-temp-buffer
                      (jaunder--reconcile-render-result result)
                      (buffer-string)))
                  results "")
                 "\n")))
    (should (string-match-p (regexp-quote expected) (buffer-string)))))

(defun jaunder-reconcile-live--kill-root-buffers (root)
  "Kill every visiting buffer rooted at temporary test ROOT."
  (dolist (buffer (buffer-list))
    (with-current-buffer buffer
      (when (and buffer-file-name (file-in-directory-p buffer-file-name root))
        (set-buffer-modified-p nil)
        (kill-buffer buffer)))))

(ert-deftest jaunder-reconcile-live-mixed-multipage-batches-refresh-and-cancel ()
  "One live multipage report retains mixed command results through cancellation."
  (jaunder-test--with-live-server
   (let* ((root (file-name-as-directory (make-temp-file "jaunder-reconcile-mixed-live-" t)))
          (token (file-name-nondirectory (directory-file-name root)))
          (jaunder-blogs (list (cons root (list :base-url jaunder-test-base-url
                                                :username jaunder-test-username))))
          (matched (jaunder-reconcile-live--write-local
                    root (concat "matched-" token ".org") (concat "matched-" token)))
          (first-draft (jaunder-reconcile-live--write-local
                        root (concat "first-draft-" token ".org") "first draft"))
          (cancelled-draft (jaunder-reconcile-live--write-local
                            root (concat "cancelled-draft-" token ".org") "cancelled draft"))
          (untouched-draft (jaunder-reconcile-live--write-local
                            root (concat "untouched-draft-" token ".org") "untouched draft"))
          matched-buffer ids)
     (unwind-protect
         (jaunder--call-with-blog
          root
          (lambda ()
            ;; Establish a local-ahead row, then make enough server-only Members
            ;; to require the real Collection pagination boundary.
            (setq matched-buffer (find-file-noselect matched))
            (with-current-buffer matched-buffer
              (jaunder-publish)
              (goto-char (point-max))
              (insert "Local batch change.\n")
              (jaunder--set-property "JAUNDER_LOCAL_AHEAD" "true")
              (save-buffer))
            (setq ids (mapcar (lambda (index)
                                (jaunder-reconcile-live--create-pagination-member
                                 (format "mixed-page-%s-%d" token index) "Remote batch body."))
                              (number-sequence 1 26)))
            (jaunder-reconcile root)
            (with-current-buffer "*Jaunder Reconcile*"
              (let* ((matched-id (with-current-buffer matched-buffer
                                   (jaunder--buffer-property "JAUNDER_ID")))
                     ;; Collection order is newest first, making this oldest
                     ;; fixture the first item on the second 25-Member page.
                     (pull-id (car ids))
                     (delete-id (car (last ids))))
                (let ((server-only-ids
                       (mapcar #'jaunder-inventory-member-id
                               (jaunder-inventory-server-only
                                (jaunder-reconcile-report-inventory
                                 jaunder-reconcile-report)))))
                  (should (>= (cl-position pull-id server-only-ids :test #'equal) 25)))
                ;; Explicit push updates the local-ahead Member and creates the draft.
                (dolist (key (list (format "post:%s" matched-id)
                                   (format "local:%s" first-draft)))
                  (puthash key t jaunder-reconcile-marks))
                (cl-letf (((symbol-function 'y-or-n-p) (lambda (_) t)))
                  (jaunder-reconcile-push-selected))
                (should (equal (mapcar #'jaunder-reconcile-result-action
                                       jaunder-reconcile-last-batch-results)
                               '(push push)))
                (should (equal (mapcar #'jaunder-reconcile-result-outcome
                                       jaunder-reconcile-last-batch-results)
                               '(success success)))
                (jaunder-reconcile-live--assert-rendered-last-batch
                 jaunder-reconcile-last-batch-results)
                (let ((draft-result
                       (cl-find (format "local:%s" first-draft)
                                jaunder-reconcile-last-batch-results
                                :key #'jaunder-reconcile-result-row-key :test #'equal)))
                  (should (jaunder-reconcile-result-post-id draft-result))
                  (should (eq (jaunder-reconcile-result-local-effect draft-result) 'created))
                  (let ((response
                         (jaunder--http-request
                          "GET" (jaunder--member-url
                                 (jaunder-reconcile-result-post-id draft-result)))))
                    (should (eq (plist-get response :status) 200))
                    (should (equal (jaunder-reconcile-live--member-representation response)
                                   (list :title "first draft" :content-type "text/org"
                                         :body "Body.\n"))))
                  (should (eq (jaunder-reconcile-row-state
                               (cl-find (jaunder-reconcile-result-post-id draft-result)
                                        (jaunder-reconcile-report-rows
                                         jaunder-reconcile-report)
                                        :key #'jaunder--reconcile-row-post-id :test #'equal))
                              'unchanged)))
                (let ((response (jaunder--http-request "GET" (jaunder--member-url matched-id))))
                  (should (eq (plist-get response :status) 200))
                  (should (equal (jaunder-reconcile-live--member-representation response)
                                 (list :title (concat "matched-" token) :content-type "text/org"
                                       :body "Body.\nLocal batch change.\n"))))
                (should (eq (jaunder-reconcile-row-state
                             (cl-find matched-id (jaunder-reconcile-report-rows
                                                  jaunder-reconcile-report)
                                      :key #'jaunder--reconcile-row-post-id :test #'equal))
                            'unchanged))
                ;; Start each explicit command with its own selection.
                (clrhash jaunder-reconcile-marks)
                ;; The last created Member proves the report traversed page two.
                (puthash (format "post:%s" pull-id) t jaunder-reconcile-marks)
                (cl-letf (((symbol-function 'y-or-n-p) (lambda (_) t)))
                  (jaunder-reconcile-pull-selected))
                (should (equal (mapcar #'jaunder-reconcile-result-post-id
                                       jaunder-reconcile-last-batch-results)
                               (list pull-id)))
                (jaunder-reconcile-live--assert-rendered-last-batch
                 jaunder-reconcile-last-batch-results)
                (should (eq (jaunder-reconcile-result-local-effect
                             (car jaunder-reconcile-last-batch-results))
                            'created))
                (should (file-exists-p (expand-file-name
                                        (concat (jaunder-reconcile-result-slug
                                                 (car jaunder-reconcile-last-batch-results)) ".org")
                                        root)))
                (let* ((pulled (car jaunder-reconcile-last-batch-results))
                       (path (expand-file-name
                              (concat (jaunder-reconcile-result-slug pulled) ".org") root))
                       (bytes (with-temp-buffer
                                (insert-file-contents path)
                                (buffer-string))))
                  (should (= (jaunder-reconcile-result-http-status pulled) 200))
                  (should (string-match-p
                           (concat "^#\\+TITLE: mixed-page-" (regexp-quote token) "-1$") bytes))
                  (should (string-match-p
                           (concat "^#\\+PROPERTY: JAUNDER_ID " (regexp-quote pull-id) "$") bytes))
                  (should (equal (with-temp-buffer
                                   (insert bytes)
                                   (org-mode)
                                   (buffer-substring-no-properties
                                    (jaunder--body-start) (point-max)))
                                 "Remote batch body.\n"))
                  (let ((response (jaunder--http-request "GET" (jaunder--member-url pull-id))))
                    (should (eq (plist-get response :status) 200))
                    (should (equal (jaunder-reconcile-live--member-representation response)
                                   (list :title (format "mixed-page-%s-1" token)
                                         :content-type "text/org" :body "Remote batch body.\n"))))
                  (should (eq (jaunder-reconcile-row-state
                               (cl-find pull-id (jaunder-reconcile-report-rows
                                                 jaunder-reconcile-report)
                                        :key #'jaunder--reconcile-row-post-id :test #'equal))
                              'unchanged)))
                ;; Delete remains a separately confirmed, remote soft-delete action.
                (clrhash jaunder-reconcile-marks)
                (puthash (format "post:%s" delete-id) t jaunder-reconcile-marks)
                (let (prompt)
                  (cl-letf (((symbol-function 'y-or-n-p)
                             (lambda (text) (setq prompt text) t)))
                    (jaunder-reconcile-delete-selected))
                  (should (string-match-p "SOFT-DELETE" prompt)))
                (should (equal (mapcar #'jaunder-reconcile-result-outcome
                                       jaunder-reconcile-last-batch-results)
                               '(success)))
                (jaunder-reconcile-live--assert-rendered-last-batch
                 jaunder-reconcile-last-batch-results)
                (should (= (jaunder-reconcile-result-http-status
                            (car jaunder-reconcile-last-batch-results)) 204))
                (should (eq (plist-get (jaunder--http-request
                                        "GET" (jaunder--member-url delete-id)) :status)
                            404))
                (should-not (cl-find delete-id (jaunder-reconcile-report-rows
                                                jaunder-reconcile-report)
                                     :key #'jaunder--reconcile-row-post-id :test #'equal))
                ;; The public command confirms marked rows.  Its executor is
                ;; wrapped only to supply the production between-item predicate.
                (let ((real-execute (symbol-function 'jaunder--reconcile-execute-batch))
                      (completed 0) prompt)
                  (clrhash jaunder-reconcile-marks)
                  (dolist (path (list cancelled-draft untouched-draft))
                    (puthash (format "local:%s" path) t jaunder-reconcile-marks))
                  (cl-letf (((symbol-function 'jaunder--reconcile-execute-batch)
                             (lambda (buffer rows action operation &optional _cancelled-p)
                               (funcall real-execute buffer rows action
                                        (lambda (row)
                                          (setq completed (1+ completed))
                                          (funcall operation row))
                                        (lambda () (= completed 1)))))
                            ((symbol-function 'y-or-n-p)
                             (lambda (text) (setq prompt text) t)))
                    (jaunder-reconcile-push-selected))
                  (should (string-match-p "Push 2 selected" prompt))
                  (should (= completed 1))
                  (should (= (length jaunder-reconcile-last-batch-results) 1))
                  (jaunder-reconcile-live--assert-rendered-last-batch
                   jaunder-reconcile-last-batch-results)
                  (let* ((result (car jaunder-reconcile-last-batch-results))
                         (completed-key (jaunder-reconcile-result-row-key result))
                         (completed-id (jaunder-reconcile-result-post-id result)))
                    (should (member completed-key
                                    (list (format "local:%s" cancelled-draft)
                                          (format "local:%s" untouched-draft))))
                    (should (eq (jaunder-reconcile-result-outcome result) 'success))
                    (should (eq (jaunder-reconcile-result-local-effect result) 'created))
                    (should (eq (plist-get (jaunder--http-request
                                            "GET" (jaunder--member-url completed-id)) :status)
                                200))
                    (should (eq (jaunder-reconcile-row-state
                                 (cl-find completed-id (jaunder-reconcile-report-rows
                                                        jaunder-reconcile-report)
                                          :key #'jaunder--reconcile-row-post-id :test #'equal))
                                'unchanged))
                    (should (eq (jaunder-reconcile-row-state
                                 (cl-find-if
                                  (lambda (row)
                                    (and (eq (jaunder-reconcile-row-state row) 'local-draft)
                                         (not (equal (jaunder--reconcile-stable-row-key row)
                                                     completed-key))))
                                  (jaunder-reconcile-report-rows jaunder-reconcile-report)))
                                'local-draft))
                    (should-not (jaunder--read-local-id
                                 (if (equal completed-key (format "local:%s" cancelled-draft))
                                     untouched-draft cancelled-draft))))
                  (should (cl-find matched-id (jaunder-reconcile-report-rows
                                               jaunder-reconcile-report)
                                   :key (lambda (row)
                                          (jaunder--reconcile-row-post-id row))
                                   :test #'equal)))))))
       (jaunder-reconcile-live--kill-root-buffers root)
       (delete-directory root t)))))

(provide 'jaunder-reconcile-integration)
;;; jaunder-reconcile-integration.el ends here
