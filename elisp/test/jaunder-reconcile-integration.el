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
                  (should (= (cl-count id server-only-ids :test #'equal) 1)))))))
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

(defun jaunder-reconcile-live--member-representation (response)
  "Return RESPONSE's exact title, content type, and native text representation."
  (let* ((fields (jaunder--harvest-response-fields (plist-get response :body)))
         (title (car (cdr (assq 'titles fields))))
         (content (car (cdr (assq 'content-nodes fields)))))
    (list :title title
          :content-type (dom-attr content 'type)
          :body (dom-inner-text content))))

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
                                      :body "Body.Local change.\n")))
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

(provide 'jaunder-reconcile-integration)
;;; jaunder-reconcile-integration.el ends here
