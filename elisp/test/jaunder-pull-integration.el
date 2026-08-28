;;; jaunder-pull-integration.el --- Live AtomPub pull tests -*- lexical-binding: t; -*-

;; Copyright (C) 2026 Jaunder contributors

;;; Commentary:
;; End-to-end proof that an untitled server-only Post becomes deterministic Org
;; bytes through the real transport, and that a pre-existing destination blocks.

;;; Code:

(require 'ert)
(require 'jaunder)
(require 'jaunder-integration-helper)

(defun jaunder-pull-integration--create-server-only-member (root body)
  "Create BODY through AtomPub and return its fresh server-only Member."
  (let* ((create
          (jaunder--http-request
           "POST"
           (jaunder--build-url (jaunder--active-base-url) "atompub"
                               (jaunder--active-username) "posts")
           (jaunder--atom-entry->xml
            (jaunder--make-entry :title "" :draft t
                                 :content-type "text/org" :body body))
           "application/atom+xml"))
         (location (jaunder--response-header create "Location")))
    (should (eq (plist-get create :status) 201))
    (string-match "/\\([0-9]+\\)/?\\'" location)
    (let ((member
           (cl-find (match-string 1 location)
                    (jaunder-inventory-server-only
                     (jaunder--inventory-for-root root))
                    :key #'jaunder-inventory-member-id :test #'equal)))
      (should member)
      member)))

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
                                   ;; An occupied destination must win before both
                                   ;; authenticated Member and anonymous media I/O.
                                   (cl-letf (((symbol-function 'jaunder--http-request)
                                              (lambda (&rest _) (error "unexpected Member I/O")))
                                             ((symbol-function 'jaunder--pull-media-get)
                                              (lambda (&rest _) (error "unexpected media I/O"))))
                                            (let ((blocked (jaunder--pull-member root member)))
                                              (should (eq (jaunder-pull-result-status blocked) 'blocked))
                                              (should (equal (jaunder-pull-result-path blocked) path))
                                              (should (equal (with-temp-buffer
                                                               (insert-file-contents path)
                                                               (buffer-string))
                                                             bytes))))))))
       (delete-directory root t)))))

(ert-deftest jaunder-pull-localizes-media-retries-reuses-and-republishes ()
  "One verified public download serves retries and separate pulled Posts."
  (jaunder-test--with-live-server
   (let* ((root (make-temp-file "jaunder-pull-media-live-" t))
          (image (expand-file-name "source image.png" root))
          (source-bytes "PULL-MEDIA-BYTES")
          (jaunder-blogs
           (list (cons (file-name-as-directory root)
                       (list :base-url jaunder-test-base-url
                             :username jaunder-test-username)))))
     (unwind-protect
         (progn
           (with-temp-file image (insert source-bytes))
           (let ((real-get (symbol-function 'jaunder--pull-media-get))
                 (real-install (symbol-function 'jaunder--install-pulled-bytes))
                 (real-http (symbol-function 'jaunder--http-request))
                 (gets 0)
                 (media-upload-statuses nil)
                 (install-attempts 0))
             (cl-letf
              (((symbol-function 'jaunder--pull-media-get)
                (lambda (&rest arguments)
                  (setq gets (1+ gets))
                  (apply real-get arguments)))
               ((symbol-function 'jaunder--install-pulled-bytes)
                (lambda (path bytes)
                  (setq install-attempts (1+ install-attempts))
                  (if (= install-attempts 1)
                      (error "injected final Post install failure")
                    (funcall real-install path bytes)))))
              (jaunder--with-blog root
                                  (let* ((media-url (jaunder--upload-media image "image/png"))
                                         (first
                                          (jaunder-pull-integration--create-server-only-member
                                           root (format "[[%s]]" media-url)))
                                         (hash
                                          (progn
                                            (string-match
                                             "/media/\\(?:upload\\|cached\\)/[0-9a-f]\\{2\\}/[0-9a-f]\\{2\\}/\\([0-9a-f]\\{64\\}\\)/"
                                             media-url)
                                            (match-string 1 media-url)))
                                         (copy (expand-file-name
                                                (concat "local-media/" hash "/source image.png") root)))
                                    (should-error (jaunder--pull-member root first))
                                    (should-not
                                     (file-exists-p
                                      (expand-file-name
                                       (concat (jaunder-inventory-member-slug first) ".org") root)))
                                    (should (equal (with-temp-buffer
                                                     (insert-file-contents-literally copy)
                                                     (buffer-string))
                                                   source-bytes))
                                    (should (= gets 1))
                                    (let* ((pulled (jaunder--pull-member root first))
                                           (path (jaunder-pull-result-path pulled))
                                           (before (with-temp-buffer
                                                     (insert-file-contents-literally copy)
                                                     (buffer-string)))
                                           (native-body
                                            (concat "[[file:local-media/" hash
                                                    "/source%20image.png]]")))
                                      (should (eq (jaunder-pull-result-status pulled) 'pulled))
                                      (should (= gets 1))
                                      (should (equal
                                               (with-temp-buffer
                                                 (insert-file-contents path)
                                                 (org-mode)
                                                 (jaunder-entry-body
                                                  (jaunder--org->atom)))
                                               native-body))
                                      (let ((buffer (find-file-noselect path)))
                                        (unwind-protect
                                            (with-current-buffer buffer
                                              (cl-letf
                                               (((symbol-function 'jaunder--http-request)
                                                 (lambda (method url &rest arguments)
                                                   (let ((response
                                                          (apply real-http method url arguments)))
                                                     (when (string-match-p "/media\\'" url)
                                                       (push (plist-get response :status)
                                                             media-upload-statuses))
                                                     response))))
                                               (jaunder-publish)))
                                          (when (buffer-live-p buffer)
                                            (with-current-buffer buffer (set-buffer-modified-p nil))
                                            (kill-buffer buffer))))
                                      (should (equal media-upload-statuses '(200)))
                                      (should (equal
                                               (with-temp-buffer
                                                 (insert-file-contents path)
                                                 (org-mode)
                                                 (jaunder-entry-body
                                                  (jaunder--org->atom)))
                                               native-body))
                                      (should (equal before
                                                     (with-temp-buffer
                                                       (insert-file-contents-literally copy)
                                                       (buffer-string)))))
                                    (let ((second
                                           (jaunder-pull-integration--create-server-only-member
                                            root (format "[[%s]]" media-url))))
                                      (should (eq (jaunder-pull-result-status
                                                   (jaunder--pull-member root second))
                                                  'pulled))
                                      (should (= gets 1))))))))
       (delete-directory root t)))))

(provide 'jaunder-pull-integration)
;;; jaunder-pull-integration.el ends here
