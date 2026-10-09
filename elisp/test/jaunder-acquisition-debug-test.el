;;; jaunder-acquisition-debug-test.el --- Acquisition diagnostic boundaries -*- lexical-binding: t; -*-

;;; Commentary:
;; Real-boundary proofs for the first acquisition diagnostic slice.

;;; Code:

(require 'ert)
(require 'cl-lib)
(require 'jaunder)

(load (expand-file-name "jaunder-debug-boundary-fixture.el"
                        (file-name-directory (or load-file-name buffer-file-name))) nil t)

(defconst jaunder-acquisition-debug-test--service-document
  (concat "<service xmlns=\"http://www.w3.org/2007/app\""
          " xmlns:atom=\"http://www.w3.org/2005/Atom\">"
          "<workspace><atom:title>Posts</atom:title></workspace></service>")
  "A valid minimal AtomPub Service Document.")

(ert-deftest jaunder-acquisition-debug-service-retrieval-nests-transport-and-parse ()
  "Service retrieval owns a root with real transport and parser child spans."
  (jaunder-debug-boundary--with-session
   (let ((jaunder-debug t)
         (jaunder--active-blog '(:base-url "https://private.example" :username "private-user")))
     (cl-letf (((symbol-function 'jaunder--auth-secret) (lambda () "credential-sentinel"))
               ((symbol-function 'plz)
                (lambda (&rest _)
                  (make-plz-response
                   :status 200 :headers nil
                   :body jaunder-acquisition-debug-test--service-document))))
       (should (listp (jaunder--fetch-service-document "https://private.example"))))
     (let ((text (jaunder-debug-boundary--text)))
       (dolist (label '("service.read" "transport.request" "service.parse"))
         (should (= 2 (jaunder-debug-boundary--label-count label text))))
       (should (string-match-p "method=GET" text))
       (should (string-match-p "http-status=200" text))
       (should-not (string-match-p "credential-sentinel\\|private.example\\|private-user" text))
       (should (= 4 (cl-count-if (lambda (line) (string-match-p "parent=" line))
                                 (split-string text "\n" t))))))))

(ert-deftest jaunder-acquisition-debug-atom-boundaries-pair-as-standalone-roots ()
  "Atom parsing and serialization pair independently outside a caller span."
  (jaunder-debug-boundary--with-session
   (let ((jaunder-debug t))
     (jaunder--atom-entry->xml
      (jaunder--make-entry :title "title-sentinel" :content-type "text/org"
                           :body "body-sentinel"))
     (jaunder--harvest-response-fields
      (concat "<entry xmlns=\"http://www.w3.org/2005/Atom\">"
              "<content type=\"text/org\">content-sentinel</content></entry>"))
     (let ((text (jaunder-debug-boundary--text)))
       (dolist (label '("atom.serialize" "atom.parse"))
         (should (= 2 (jaunder-debug-boundary--label-count label text))))
       (should (= 4 (cl-count-if (lambda (line) (string-match-p "format=atom" line))
                                 (split-string text "\n" t))))
       (should-not (string-match-p "title-sentinel\\|body-sentinel\\|content-sentinel" text))
       (let ((correlations (delete-dups
                            (mapcar (lambda (line)
                                      (progn (string-match "correlation=\\([^ ]+\\)" line)
                                             (match-string 1 line)))
                                    (split-string text "\n" t)))))
         (should (= 2 (length correlations))))))))

(ert-deftest jaunder-acquisition-debug-preserves-errors-and-quit ()
  "Actual boundaries retain their conditions while their terminal outcomes differ."
  (jaunder-debug-boundary--with-session
   (let ((jaunder-debug t)
         (jaunder--active-blog '(:base-url "https://private.example" :username "private-user")))
     (cl-letf (((symbol-function 'jaunder--auth-secret) (lambda () "credential-sentinel"))
               ((symbol-function 'plz)
                (lambda (&rest _)
                  (signal 'plz-curl-error
                          (list "private-error" (make-plz-error :message "private-error"))))))
       (should-error (jaunder--http-request "PRIVATE-METHOD" "https://private.example/private-body"
                                            "body-sentinel")
                     :type 'plz-error))
     (cl-letf (((symbol-function 'xml-parse-region)
                (lambda (&rest _) (signal 'quit '(private-quit)))))
       (should (eq 'quit
                   (condition-case err
                       (jaunder--parse-service-document "content-sentinel")
                     (quit (car err))))))
     (should-error (jaunder--harvest-response-fields "<content-sentinel"))
     (let ((text (jaunder-debug-boundary--text)))
       (should (string-match-p "label=transport.request" text))
       (should (string-match-p "label=service.parse" text))
       (should (string-match-p "label=atom.parse" text))
       (should (string-match-p "outcome=error" text))
       (should (string-match-p "outcome=cancelled" text))
       (should (string-match-p "method=unknown" text))
       (should-not (string-match-p
                    "credential-sentinel\\|private.example\\|private-user\\|private-error\\|private-quit\\|body-sentinel\\|content-sentinel"
                    text))))))

(ert-deftest jaunder-acquisition-debug-disabled-skips-boundary-diagnostic-work ()
  "Disabled actual boundaries neither sample nor create diagnostic state."
  (jaunder-debug-boundary--with-session
   (let ((jaunder--active-blog '(:base-url "https://private.example" :username "private-user")))
     (cl-letf (((symbol-function 'jaunder--debug-now) (lambda () (error "clock")))
               ((symbol-function 'jaunder--debug-format-event) (lambda (_) (error "format")))
               ((symbol-function 'jaunder--debug-buffer) (lambda () (error "buffer")))
               ((symbol-function 'jaunder--auth-secret) (lambda () "credential-sentinel"))
               ((symbol-function 'plz)
                (lambda (&rest _)
                  (make-plz-response :status 200 :headers nil :body ""))))
       (should (equal "" (plist-get (jaunder--http-request "GET" "https://private.example" "body-sentinel")
                                    :body)))
       (should (stringp (jaunder--atom-entry->xml
                         (jaunder--make-entry :content-type "text/org" :body "body-sentinel"))))
       (should (listp (jaunder--parse-service-document
                       jaunder-acquisition-debug-test--service-document)))
       (should (eq 'unknown (jaunder--fetch-service-document "https://private.example")))
       (should-error (jaunder--harvest-response-fields "<content-sentinel")))
     (should (= 0 jaunder--debug-id-counter))
     (should-not jaunder--debug-operation-stack)
     (should-not (get-buffer jaunder--debug-buffer-name)))))

(ert-deftest jaunder-acquisition-debug-preserves-wire-and-representation-bytes ()
  "Enabling diagnostics preserves requests, response bodies and transformations."
  (jaunder-debug-boundary--with-session
   (let* ((jaunder--active-blog '(:base-url "https://private.example" :username "private-user"))
          (payload (concat "body-sentinel\r\n" (string 0)))
          (entry (jaunder--make-entry :title "title-sentinel" :content-type "text/org" :body payload))
          snapshots)
     (dolist (enabled '(nil t))
       (let ((jaunder-debug enabled) requests)
         (cl-letf (((symbol-function 'jaunder--auth-secret) (lambda () "credential-sentinel"))
                   ((symbol-function 'plz)
                    (lambda (&rest args)
                      (push (list args plz-curl-default-args) requests)
                      (make-plz-response :status 200 :headers '(("ETag" . "header-sentinel"))
                                         :body payload))))
           (dolist (method '("GET" "HEAD" "POST" "PUT" "DELETE"))
             (let ((response (jaunder--http-request method "https://private.example/secret"
                                                    payload "text/org" '(("If-Match" . "header-sentinel")))))
               (should (eq payload (plist-get response :body)))
               (should (equal '("etag" . "header-sentinel")
                              (car (plist-get response :headers)))))))
         (push (list requests (jaunder--atom-entry->xml entry)
                     (jaunder--harvest-response-fields
                      "<entry xmlns=\"http://www.w3.org/2005/Atom\"><title>title-sentinel</title></entry>")
                     (jaunder--parse-service-document jaunder-acquisition-debug-test--service-document))
               snapshots)))
     (should (equal (car snapshots) (cadr snapshots)))
     (should (= 5 (length (caar snapshots))))
     (should-not (string-match-p "sentinel\\|private.example\\|private-user"
                                 (jaunder-debug-boundary--text))))))

(ert-deftest jaunder-acquisition-debug-status-producers-reject-source-text ()
  "Both response-status producers reject a malformed status without leaking it."
  (jaunder-debug-boundary--with-session
   (let ((jaunder-debug t)
         (jaunder--active-blog '(:base-url "https://private.example" :username "private-user"))
         warnings)
     (cl-letf (((symbol-function 'display-warning)
                (lambda (_type text &rest _) (push text warnings)))
               ((symbol-function 'jaunder--auth-secret) (lambda () "credential-sentinel"))
               ((symbol-function 'plz)
                (lambda (&rest _)
                  (make-plz-response :status "status-sentinel" :headers nil :body "body-sentinel"))))
       (should (equal "status-sentinel"
                      (plist-get (jaunder--http-request "GET" "https://private.example") :status)))
       (should (eq 'unknown (jaunder--fetch-service-document "https://private.example"))))
     (should (= 3 (length warnings)))
     (should (cl-every (lambda (text) (equal text "jaunder: diagnostic output unavailable")) warnings))
     (should-not (string-match-p "sentinel\\|private.example\\|private-user"
                                 (jaunder-debug-boundary--text))))))

;;; jaunder-acquisition-debug-test.el ends here
