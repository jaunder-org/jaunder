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
         (jaunder--with-blog root
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
                                   (should (= (cl-count id server-only-ids :test #'equal) 1))))))
       (delete-directory root t)))))

(ert-deftest jaunder-reconcile-fetches-member-etag-and-pulls-preview-only ()
  "The live command fetches matched Members and applies its server-only preview."
  (jaunder-test--with-live-server
   (let* ((root (make-temp-file "jaunder-reconcile-live-" t))
          (jaunder-blogs
           (list (cons (file-name-as-directory root)
                       (list :base-url jaunder-test-base-url
                             :username jaunder-test-username))))
          (token (file-name-nondirectory (directory-file-name root))))
     (unwind-protect
         (jaunder--with-blog
          root
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
              (cl-letf (((symbol-function 'y-or-n-p) (lambda (_) t)))
                       (jaunder-reconcile root))
              (let* ((inventory (jaunder--inventory-for-root root))
                     (match (cl-find id (jaunder-inventory-matched inventory)
                                     :key (lambda (item)
                                            (jaunder-inventory-member-id
                                             (jaunder-inventory-match-member item)))
                                     :test #'equal))
                     (report (jaunder--reconcile-build-report root inventory))
                     (row (car (jaunder-reconcile-report-rows report))))
                (should match)
                (should (file-exists-p
                         (jaunder-inventory-local-path
                          (jaunder-inventory-match-local match))))
                (should (memq (jaunder-reconcile-row-state row)
                              '(unchanged local-ahead)))
                (with-current-buffer "*Jaunder Reconcile*"
                  (should (string-match-p (regexp-quote slug) (buffer-string))))))))
       (delete-directory root t)))))

(provide 'jaunder-reconcile-integration)
;;; jaunder-reconcile-integration.el ends here
