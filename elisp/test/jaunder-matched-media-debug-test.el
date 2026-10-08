;;; jaunder-matched-media-debug-test.el --- Matched Media owner diagnostics -*- lexical-binding: t; -*-

;;; Commentary:
;; Actual matched-pull consumers must time verified Media staging, installation,
;; and original-path checks without changing requests or authored destinations.

;;; Code:

(require 'ert)
(require 'cl-lib)
(require 'jaunder)
(load (expand-file-name "jaunder-debug-boundary-fixture.el"
                        (file-name-directory (or load-file-name buffer-file-name))) nil t)

(defun jaunder-matched-media-debug--check-routes (debug thunk)
  "Run THUNK and count diagnostic route work even if a failure is contained."
  (let ((clock (symbol-function 'jaunder--debug-now))
        (formatter (symbol-function 'jaunder--debug-format-event))
        (buffer (symbol-function 'jaunder--debug-buffer))
        (calls 0))
    (cl-letf (((symbol-function 'jaunder--debug-now)
               (lambda () (setq calls (1+ calls))
                 (unless debug (ert-fail "disabled diagnostic clock")) (funcall clock)))
              ((symbol-function 'jaunder--debug-format-event)
               (lambda (event) (setq calls (1+ calls))
                 (unless debug (ert-fail "disabled diagnostic fields")) (funcall formatter event)))
              ((symbol-function 'jaunder--debug-buffer)
               (lambda () (setq calls (1+ calls))
                 (unless debug (ert-fail "disabled diagnostic buffer")) (funcall buffer))))
      (funcall thunk))
    (if debug (should (> calls 0)) (should (= calls 0)))))

(defun jaunder-matched-media-debug--run (debug action stale)
  "Run actual matched ACTION with DEBUG and optionally make its original STALE."
  (jaunder-debug-boundary--with-session
   (let* ((root (file-name-as-directory (make-temp-file "jaunder-private-matched-" t)))
          (path (expand-file-name "post.org" root))
          (original (expand-file-name "assets/private-original.png" root))
          (bytes (string-as-unibyte "private verified original bytes"))
          (hash (secure-hash 'sha256 bytes))
          (instance "12345678-1234-1234-1234-123456789abc")
          (origin "https://private.example")
          (url (format "%s/media/upload/%s/%s/%s/private-remote.png"
                       origin (substring hash 0 2) (substring hash 2 4) hash))
          (local-body "[[file:assets/private-original.png][private local label]]\n")
          (remote-body (format "private remote edits [[%s#remote-fragment][remote-label]]" url))
          (xml (concat "<entry xmlns=\"http://www.w3.org/2005/Atom\""
                       " xmlns:app=\"http://www.w3.org/2007/app\""
                       " xmlns:j=\"https://jaunder.org/ns/atompub\">"
                       "<title>private-title</title><link rel=\"edit\""
                       " href=\"https://private.example/atompub/private-user/posts/7\"/>"
                       "<j:slug>post</j:slug><content type=\"text/org\">" remote-body
                       "</content><app:control><app:draft>yes</app:draft></app:control></entry>"))
          (member (jaunder--make-inventory-member
                   :id "7" :slug "post" :etag "\"old\""
                   :edit-uri "https://private.example/atompub/private-user/posts/7"))
          (jaunder--active-blog (list :base-url origin :username "private-user"))
          (member-requests 0) (media-requests 0) (inventories 0)
          result rendered projection)
     (unwind-protect
         (progn
           (make-directory (file-name-directory original))
           (with-temp-file original (set-buffer-multibyte nil) (insert bytes))
           (with-temp-file path
             (insert "#+PROPERTY: JAUNDER_STATUS draft\n#+PROPERTY: JAUNDER_FORMAT org\n"
                     "#+PROPERTY: JAUNDER_SLUG post\n#+PROPERTY: JAUNDER_ID 7\n"
                     "#+PROPERTY: JAUNDER_SYNCED \"old\"\n\n" local-body))
           (let* ((local (jaunder--make-inventory-local :path path :id "7" :slug "post"))
                  (row (jaunder--make-reconcile-row
                        :state (if (eq action 'pull) 'server-ahead 'conflict)
                        :local local :member member :remote-etag "\"old\""
                        :local-sha256 (jaunder--reconcile-file-sha256 path)))
                  (jaunder-reconcile-report
                   (jaunder--make-reconcile-report
                    :root root :inventory (jaunder--join-inventory (list local) (list member))))
                  (jaunder-debug debug))
             (cl-letf (((symbol-function 'current-time) (lambda () '(24000 0 0 0)))
                       ((symbol-function 'jaunder--current-zone-name) (lambda () "UTC"))
                       ((symbol-function 'jaunder--fetch-service-document)
                        (lambda (_)
                          (jaunder--parse-service-document
                           "<service xmlns=\"http://www.w3.org/2007/app\" xmlns:atom=\"http://www.w3.org/2005/Atom\"><workspace><atom:title>x</atom:title></workspace></service>")))
                       ((symbol-function 'jaunder--inventory-for-root)
                        (lambda (_)
                          (setq inventories (1+ inventories))
                          (jaunder--join-inventory (list local) (list member))))
                       ((symbol-function 'jaunder--http-request)
                        (lambda (method _url &rest _)
                          (should (equal method "GET"))
                          (setq member-requests (1+ member-requests))
                          (list :status 200 :body xml
                                :headers (list (cons "etag" "\"old\"")
                                               (cons "x-jaunder-instance" instance)))))
                       ((symbol-function 'plz)
                        (lambda (&rest _)
                          (setq media-requests (1+ media-requests))
                          (when stale (with-temp-file original (insert "private changed original")))
                          (make-plz-response
                           :status 200 :body bytes
                           :headers (list (cons "x-jaunder-instance" instance)
                                          (cons "etag" (concat "\"sha256-" hash "\"")))))))
               (setq result (pcase action
                              ('pull (jaunder--reconcile-pull-row row))
                              ('keep-remote (jaunder--reconcile-keep-remote-row row))
                              ('merge (jaunder--reconcile-merge-stage row))))
               (if (eq action 'merge)
                   (progn
                     (should (plist-get result :staged))
                     (setq rendered (plist-get (plist-get result :staged) :bytes)))
                 (should (eq (plist-get result :outcome) 'success))
                 (setq rendered (with-temp-buffer (insert-file-contents path) (buffer-string)))))
             (should (= media-requests 1))
             (should (= member-requests (if (eq action 'pull) 2 3)))
             (should (= inventories (if (eq action 'pull) 1 2)))
             (should (string-match-p "private remote edits" rendered))
             (should (string-match-p
                      (regexp-quote
                       (concat (if stale (format "file:local-media/%s/private-remote.png" hash)
                                 "file:assets/private-original.png")
                               "#remote-fragment][remote-label]")) rendered))
             (should (eq (file-exists-p (expand-file-name "local-media" root)) stale))
             (setq projection (list rendered member-requests media-requests inventories
                                    (with-temp-buffer (insert-file-contents original) (buffer-string))))
             (if debug
                 (let ((text (jaunder-debug-boundary--text)))
                   (jaunder-debug-boundary--assert-tree
                    text (list (if (eq action 'merge) "merge.stage" "reconcile.row")))
                   ;; One acquisition stage and one install stage; stale reuse
                   ;; requires a second install stage using already verified bytes.
                   (should (= (if stale 6 4)
                              (jaunder-debug-boundary--label-count "media.materialize" text)))
                   (should (> (jaunder-debug-boundary--label-count "media.path" text) 0))
                   (dolist (private (list root hash origin "private-user" "private-title"
                                          "private verified" "private remote" "remote-label"))
                     (should-not (string-match-p (regexp-quote private) text))))
               (should (= jaunder--debug-id-counter 0))
               (should-not (get-buffer jaunder--debug-buffer-name)))
             projection))
       (delete-directory root t)))))

(ert-deftest jaunder-matched-media-debug-consumers-preserve-reuse-fallback-and-topology ()
  "Pull, keep-remote and merge time actual reuse owners without extra requests."
  (dolist (action '(pull keep-remote merge))
    (dolist (stale '(nil t))
      (should (equal (jaunder-matched-media-debug--run nil action stale)
                     (jaunder-matched-media-debug--run t action stale))))))

(ert-deftest jaunder-matched-media-debug-path-owners-standalone-and-disabled-lazy ()
  "Original-root, original-file and fallback-target checks own standalone spans."
  (dolist (debug '(nil t))
    (jaunder-debug-boundary--with-session
     (let* ((root (make-temp-file "jaunder-private-path-" t))
            (path (expand-file-name "private.bin" root))
            (hash (make-string 64 ?a))
            (jaunder-debug debug))
       (unwind-protect
           (progn
             (with-temp-file path (insert "private bytes"))
             (jaunder-matched-media-debug--check-routes
              debug
              (lambda ()
                (should (equal root (jaunder--pull-media-original-root root)))
                (should (jaunder--pull-media-original-safe-regular-p root path))
                (should (equal (concat root "/local-media/" hash "/private.bin")
                               (jaunder--pull-media-fallback-path root hash "private.bin")))))
             (if debug
                 (let ((text (jaunder-debug-boundary--text)))
                   (jaunder-debug-boundary--assert-tree text '("media.path" "media.path" "media.path"))
                   (should (= 6 (jaunder-debug-boundary--label-count "media.path" text)))
                   (should-not (string-match-p "private\\|aaaaaaaa" text)))
               (should (= 0 jaunder--debug-id-counter))
               (should-not (get-buffer jaunder--debug-buffer-name))))
         (delete-directory root t))))))

(ert-deftest jaunder-matched-media-debug-stages-standalone-and-disabled-lazy ()
  "Acquisition and installation own separate spans, not a duplicate facade span."
  (dolist (debug '(nil t))
    (jaunder-debug-boundary--with-session
     (let* ((root (make-temp-file "jaunder-private-stage-" t))
            (plan (jaunder--make-pull-media-plan :format "org" :body "private body"))
            (jaunder-debug debug))
       (unwind-protect
           (progn
             (jaunder-matched-media-debug--check-routes
              debug
              (lambda ()
                (let ((staged (jaunder--pull-media-stage
                               root "12345678-1234-1234-1234-123456789abc" plan)))
                  (should (equal "private body" (jaunder-pull-media-plan-body
                                                 (jaunder-pull-media-staged-plan staged))))
                  (should-not (jaunder--pull-media-finalize-staged root staged)))))
             (if debug
                 (let ((text (jaunder-debug-boundary--text)))
                   (jaunder-debug-boundary--assert-tree text '("media.materialize" "media.materialize"))
                   (should (= 4 (jaunder-debug-boundary--label-count "media.materialize" text)))
                   (should (= 2 (cl-count-if (lambda (line) (string-match-p "phase=start count=0" line))
                                             (split-string text "\n" t))))
                   (should-not (string-match-p "private\\|12345678" text)))
               (should (= 0 jaunder--debug-id-counter))
               (should-not (get-buffer jaunder--debug-buffer-name))))
         (delete-directory root t))))))

(ert-deftest jaunder-matched-media-debug-new-owners-preserve-exact-native-signals ()
  "Every new owner retains the native condition-data object on error and quit."
  (dolist (debug '(nil t))
    (dolist (kind '(file-error quit))
      (dolist (owner '(root original fallback stage finalize))
        (jaunder-debug-boundary--with-session
         (let* ((root (make-temp-file "jaunder-private-signal-" t))
                (path (expand-file-name "private.bin" root))
                (hash (make-string 64 ?a))
                (data (list "private native condition"))
                (reference (jaunder--make-pull-media-reference :hash hash :leaf "private.bin"))
                (plan (jaunder--make-pull-media-plan :references (list reference)))
                (staged (jaunder--make-pull-media-staged
                         :fallbacks (list (jaunder--make-pull-media-fallback :hash hash :leaf "private.bin"))))
                (jaunder-debug debug)
                observed)
           (unwind-protect
               (progn
                 (with-temp-file path (insert "private bytes"))
                 (let* ((seam (pcase owner
                                ('root 'file-directory-p)
                                ('original 'file-attributes)
                                ('fallback 'jaunder--pull-media-safe-leaf-p)
                                ('stage 'jaunder--pull-media-original-for-hash)
                                ('finalize 'jaunder--pull-media-target-path)))
                        (native (symbol-function seam)))
                   (unwind-protect
                       (progn
                         (fset seam (lambda (&rest _) (signal kind data)))
                         (setq observed
                               (condition-case condition
                                   (pcase owner
                                     ('root (jaunder--pull-media-original-root root))
                                     ('original (jaunder--pull-media-original-safe-regular-p root path))
                                     ('fallback (jaunder--pull-media-fallback-path root hash "private.bin"))
                                     ('stage (jaunder--pull-media-stage
                                              root "12345678-1234-1234-1234-123456789abc" plan
                                              (jaunder--make-pull-media-original-proof)))
                                     ('finalize (jaunder--pull-media-finalize-staged root staged)))
                                 (error condition) (quit condition))))
                     (fset seam native)))
                 (should (eq kind (car observed)))
                 (should (eq data (cdr observed)))
                 (should-not jaunder--debug-operation-stack)
                 (if debug
                     (let* ((label (if (memq owner '(stage finalize)) "media.materialize" "media.path"))
                            (text (jaunder-debug-boundary--text)))
                       (jaunder-debug-boundary--assert-tree text (list label))
                       (should (= 2 (jaunder-debug-boundary--label-count label text)))
                       (should (string-match-p
                                (if (eq kind 'quit) "outcome=cancelled" "outcome=error") text))
                       (should-not (string-match-p "private\\|aaaaaaaa\\|12345678" text)))
                   (should (= 0 jaunder--debug-id-counter))
                   (should-not (get-buffer jaunder--debug-buffer-name))))
             (delete-directory root t))))))))

(ert-deftest jaunder-matched-media-debug-ordinary-materialization-has-two-actual-stages ()
  "The ordinary composition times acquisition and install without a facade span."
  (jaunder-debug-boundary--with-session
   (let* ((root (make-temp-file "jaunder-private-ordinary-" t))
          (plan (jaunder--make-pull-media-plan :format "org" :body "private body"))
          (jaunder-debug t))
     (unwind-protect
         (progn
           (should-not (jaunder--pull-media-materialize
                        root "12345678-1234-1234-1234-123456789abc" plan))
           (let ((text (jaunder-debug-boundary--text)))
             (jaunder-debug-boundary--assert-tree text '("media.materialize" "media.materialize"))
             (should (= 4 (jaunder-debug-boundary--label-count "media.materialize" text)))))
       (delete-directory root t)))))

;;; jaunder-matched-media-debug-test.el ends here
