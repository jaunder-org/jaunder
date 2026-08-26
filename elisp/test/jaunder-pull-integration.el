;;; jaunder-pull-integration.el --- Live AtomPub pull tests -*- lexical-binding: t; -*-

;; Copyright (C) 2026 Jaunder contributors

;;; Commentary:
;; End-to-end proof that an untitled server-only Post becomes deterministic Org
;; bytes through the real transport, and that a pre-existing destination blocks.

;;; Code:

(require 'ert)
(require 'jaunder)
(require 'jaunder-integration-helper)

(ert-deftest jaunder-pull-untitled-member-and-block-existing-destination ()
  "Pull one unique untitled Post, then preserve its file on a blocked re-pull."
  ;; The shared server may contain unrelated Members.  Track only the unique ID
  ;; returned by this test's authenticated create, then cross D1 inventory and
  ;; D2's real GET/no-clobber seam for that Member.
  (jaunder-test--with-live-server
   (let* ((root (make-temp-file "jaunder-pull-live-" t))
          (jaunder-blogs
           (list (cons (file-name-as-directory root)
                       (list :base-url jaunder-test-base-url
                             :username jaunder-test-username))))
          (token (file-name-nondirectory (directory-file-name root)))
          (media-url (concat jaunder-test-base-url "/media/server-only.png"))
          (body (format "Unique %s\n[[%s]]" token media-url)))
     (unwind-protect
         (jaunder--with-blog root
                             (let* ((create
                                     (jaunder--http-request
                                      "POST"
                                      (jaunder--build-url (jaunder--active-base-url) "atompub"
                                                          (jaunder--active-username) "posts")
                                      (jaunder--atom-entry->xml
                                       (jaunder--make-entry
                                        :title ""
                                        :draft t
                                        :content-type "text/org"
                                        :body body))
                                      "application/atom+xml"))
                                    (location (jaunder--response-header create "Location")))
                               (should (eq (plist-get create :status) 201))
                               (should (string-match "/\\([0-9]+\\)/?\\'" location))
                               (let* ((id (match-string 1 location))
                                      (inventory (jaunder--inventory-for-root root))
                                      (member (cl-find id
                                                       (jaunder-inventory-server-only inventory)
                                                       :key #'jaunder-inventory-member-id
                                                       :test #'equal)))
                                 (should member)
                                 (let* ((member-response
                                         (jaunder--http-request
                                          "GET" (jaunder-inventory-member-edit-uri member)))
                                        (server-body
                                         (dom-text
                                          (car (cdr (assq 'content-nodes
                                                          (jaunder--harvest-response-fields
                                                           (plist-get member-response :body)))))))
                                        (pulled (jaunder--pull-member root member))
                                        (path (jaunder-pull-result-path pulled))
                                        (bytes (with-temp-buffer
                                                 (insert-file-contents path)
                                                 (buffer-string))))
                                   (should (eq (plist-get member-response :status) 200))
                                   (should (eq (jaunder-pull-result-status pulled) 'pulled))
                                   (should (equal path
                                                  (expand-file-name
                                                   (concat (jaunder-inventory-member-slug member)
                                                           ".org")
                                                   root)))
                                   (should-not (string-match-p "^#\\+TITLE:" bytes))
                                   (should (string-match-p
                                            (concat "\\`#\\+PROPERTY: JAUNDER_STATUS draft\n"
                                                    "#\\+PROPERTY: JAUNDER_FORMAT org\n"
                                                    "#\\+PROPERTY: JAUNDER_SLUG "
                                                    (regexp-quote
                                                     (jaunder-inventory-member-slug member))
                                                    "\n#\\+PROPERTY: JAUNDER_ID " id
                                                    "\n#\\+PROPERTY: JAUNDER_SYNCED \"sha256-[0-9a-f]+\""
                                                    "\n#\\+PROPERTY: JAUNDER_SYNCED_AT "
                                                    "[0-9T:-]+Z\n\n")
                                            bytes))
                                   (should (string-suffix-p server-body bytes))
                                   (should (string-match-p (regexp-quote media-url) bytes))
                                   ;; A pulled file carries the canonical Org metadata
                                   ;; block.  Re-publishing it exercises the real
                                   ;; client mapping and strict AtomPub update path.
                                   (let ((pulled-buffer (find-file-noselect path)))
                                     (unwind-protect
                                         (with-current-buffer pulled-buffer
                                           (jaunder-publish)
                                           (setq bytes (buffer-string))
                                           (should (equal
                                                    (jaunder--buffer-property "JAUNDER_ID")
                                                    id))
                                           (should (jaunder--buffer-property
                                                    "JAUNDER_SYNCED")))
                                       (when (buffer-live-p pulled-buffer)
                                         (with-current-buffer pulled-buffer
                                           (set-buffer-modified-p nil)))
                                       (when (buffer-live-p pulled-buffer)
                                         (kill-buffer pulled-buffer))))
                                   (let ((blocked (jaunder--pull-member root member)))
                                     (should (eq (jaunder-pull-result-status blocked) 'blocked))
                                     (should (equal (jaunder-pull-result-path blocked) path))
                                     (should (equal (with-temp-buffer
                                                      (insert-file-contents path)
                                                      (buffer-string))
                                                    bytes)))))))
       (delete-directory root t)))))

(provide 'jaunder-pull-integration)
;;; jaunder-pull-integration.el ends here
