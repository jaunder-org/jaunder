;;; jaunder-pull-boundary-debug-test.el --- Pull boundary diagnostics -*- lexical-binding: t; -*-

;;; Commentary:
;; Real-file pull staging, final revalidation/preflight, and atomic installation
;; proofs, including disabled instrumentation, cancellation, and privacy.

;;; Code:

(require 'ert)
(require 'cl-lib)
(require 'jaunder)

(load (expand-file-name "jaunder-debug-boundary-fixture.el"
                        (file-name-directory (or load-file-name buffer-file-name))) nil t)

(defun jaunder-pull-boundary-debug-test--entry ()
  "Return valid draft Member XML containing privacy sentinels."
  (concat "<entry xmlns=\"http://www.w3.org/2005/Atom\""
          " xmlns:app=\"http://www.w3.org/2007/app\""
          " xmlns:j=\"https://jaunder.org/ns/atompub\">"
          "<title>private-title</title><link rel=\"edit\""
          " href=\"https://private.example/atompub/alice/posts/7\"/>"
          "<j:slug>remote</j:slug><content type=\"text/org\">private-body</content>"
          "<app:control><app:draft>yes</app:draft></app:control></entry>"))

(defun jaunder-pull-boundary-debug-test--member ()
  "Return reviewed inventory Member fixture."
  (jaunder--make-inventory-member :id "7" :slug "remote"
                                  :edit-uri "https://private.example/atompub/alice/posts/7"
                                  :etag "\"old\""))

(defun jaunder-pull-boundary-debug-test--bytes ()
  "Return exact local Post bytes for identity 7."
  (concat "#+PROPERTY: JAUNDER_STATUS draft\n#+PROPERTY: JAUNDER_FORMAT org\n"
          "#+PROPERTY: JAUNDER_SLUG old\n#+PROPERTY: JAUNDER_ID 7\n"
          "#+PROPERTY: JAUNDER_SYNCED \"old\"\n\nlocal-body\n"))

(defmacro jaunder-pull-boundary-debug-test--with-root (bindings &rest body)
  "Create ROOT and PATH and bind the active configured blog for BODY."
  (declare (indent 1) (debug t))
  (let ((root (car bindings))
        (path (cadr bindings)))
    `(let* ((,root (file-name-as-directory (make-temp-file "jaunder-pull-debug-" t)))
            (,path (expand-file-name "old.org" ,root))
            (jaunder-blogs
             (list (cons ,root '(:base-url "https://private.example" :username "private-user")))))
       (unwind-protect
           (progn ,@body)
         (delete-directory ,root t)))))

(ert-deftest jaunder-pull-boundary-debug-real-stage-revalidate-preflight-install-nest ()
  "Actual pull boundaries retain staged bytes, freshness request count, and rename."
  (jaunder-debug-boundary--with-session
   (jaunder-pull-boundary-debug-test--with-root (root path)
                                                (write-region (jaunder-pull-boundary-debug-test--bytes) nil path nil 'silent)
                                                (let* ((member (jaunder-pull-boundary-debug-test--member))
                                                       (local (jaunder--make-inventory-local :path path :id "7" :slug "old"))
                                                       (row (jaunder--make-reconcile-row
                                                             :state 'server-ahead :local local :member member :local-sha256
                                                             (jaunder--reconcile-file-sha256 path) :remote-etag "\"old\""))
                                                       (jaunder-reconcile-report (jaunder--make-reconcile-report :root root))
                                                       (requests 0))
                                                  (cl-letf (((symbol-function 'jaunder--fetch-service-document)
                                                             (lambda (_) (jaunder--parse-service-document
                                                                          "<service xmlns=\"http://www.w3.org/2007/app\" xmlns:atom=\"http://www.w3.org/2005/Atom\"><workspace><atom:title>x</atom:title></workspace></service>")))
                                                            ((symbol-function 'jaunder--http-request)
                                                             (lambda (_method _url &rest _)
                                                               (setq requests (1+ requests))
                                                               (list :status 200
                                                                     :headers '(("etag" . "\"old\"")
                                                                                ("x-jaunder-instance" . "12345678-1234-1234-1234-123456789abc"))
                                                                     :body (jaunder-pull-boundary-debug-test--entry))))
                                                            ((symbol-function 'jaunder--current-zone-name) (lambda () "UTC")))
                                                    (jaunder--call-with-blog
                                                     root
                                                     (lambda ()
                                                       (let ((jaunder-debug t))
                                                         (let ((staged (jaunder--pull-stage-member root member)))
                                                           (should (string-match-p "private-body" (plist-get staged :bytes)))
                                                           (should (plist-get (jaunder--reconcile-pull-remote-revalidation row "\"old\"") :ok))
                                                           (should-not (jaunder--reconcile-pull-preflight row staged))
                                                           (let ((result (jaunder--reconcile-pull-install-staged
                                                                          row staged '(:http-status 200) path)))
                                                             (should (eq (plist-get result :outcome) 'success))
                                                             (should (equal (plist-get result :local-effect) 'renamed))
                                                             (should (equal (plist-get staged :bytes)
                                                                            (with-temp-buffer
                                                                              (insert-file-contents (expand-file-name "remote.org" root))
                                                                              (buffer-string)))))))))
                                                    (should (= requests 2))
                                                    (should (file-exists-p (expand-file-name "remote.org" root)))
                                                    (should-not (file-exists-p path))
                                                    (let ((text (jaunder-debug-boundary--text)))
                                                      (dolist (label '("pull.stage" "pull.revalidate" "pull.install"))
                                                        (should (= 2 (jaunder-debug-boundary--label-count label text))))
                                                      ;; Direct preflight plus both install guards bracket Media finalization.
                                                      (should (= 6 (jaunder-debug-boundary--label-count "pull.preflight" text)))
                                                      (should (string-match-p "parent=" text))
                                                      (should-not (string-match-p
                                                                   "private.example\\|private-user\\|private-title\\|private-body\\|sha256" text))))))))

(ert-deftest jaunder-pull-boundary-debug-preserves-errors-quit-and-disabled-work ()
  "Actual pull boundaries preserve signals and disabled calls avoid diagnostics."
  (jaunder-debug-boundary--with-session
   (let ((jaunder-debug t))
     (should-error (jaunder--pull-stage-member "/private-root" 'not-a-member)))
   (jaunder-pull-boundary-debug-test--with-root (root path)
                                                (write-region (jaunder-pull-boundary-debug-test--bytes) nil path nil 'silent)
                                                (let* ((member (jaunder-pull-boundary-debug-test--member))
                                                       (local (jaunder--make-inventory-local :path path :id "7" :slug "old"))
                                                       (row (jaunder--make-reconcile-row :state 'server-ahead :local local :member member
                                                                                         :local-sha256 (jaunder--reconcile-file-sha256 path)))
                                                       (jaunder-reconcile-report (jaunder--make-reconcile-report :root root)))
                                                  (let ((jaunder-debug t))
                                                    (cl-letf (((symbol-function 'write-region) (lambda (&rest _) (signal 'quit '(private-quit)))))
                                                      (should (equal '(quit private-quit)
                                                                     (condition-case err
                                                                         (jaunder--reconcile-replace-pulled-file path path "private-bytes")
                                                                       (quit err))))))
                                                  (let ((before jaunder--debug-id-counter)
                                                        (jaunder-debug nil))
                                                    (cl-letf (((symbol-function 'jaunder--debug-now) (lambda () (error "clock")))
                                                              ((symbol-function 'jaunder--debug-format-event) (lambda (_) (error "format")))
                                                              ((symbol-function 'jaunder--debug-buffer) (lambda () (error "buffer")))
                                                              ((symbol-function 'jaunder--http-request)
                                                               (lambda (&rest _) (list :status 200 :headers '(("etag" . "\"old\""))))) )
                                                      (should-error (jaunder--pull-stage-member "/private-root" 'not-a-member))
                                                      (should (plist-get (jaunder--reconcile-pull-remote-revalidation row "\"old\"") :ok))
                                                      (should-not (jaunder--reconcile-pull-preflight row '(:slug "old")))
                                                      (should (equal (plist-get (jaunder--reconcile-replace-pulled-file path path "private-bytes") :local-effect)
                                                                     'replaced)))
                                                    (should (= before jaunder--debug-id-counter)))
                                                  (let ((text (jaunder-debug-boundary--text)))
                                                    (should (string-match-p "outcome=error" text))
                                                    (should (string-match-p "outcome=cancelled" text))
                                                    (should-not (string-match-p "private-root\\|private-quit\\|private-bytes" text)))))))

;;; jaunder-pull-boundary-debug-test.el ends here
