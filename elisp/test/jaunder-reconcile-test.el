;;; jaunder-reconcile-test.el --- Inventory behavior tests -*- lexical-binding: t; -*-

;; Copyright (C) 2026 Jaunder contributors

;;; Commentary:
;; Focused pure contracts for AtomPub Collection inventory parsing, local discovery,
;; and conflict-safe joining.

;;; Code:

(require 'ert)
(require 'cl-lib)
(require 'jaunder)

(defun jaunder-reconcile-test--entry (id slug &optional href)
  "Return a minimal Collection Entry XML for ID, SLUG, and optional HREF."
  (format (concat "<entry><link rel=\"edit\" href=\"%s\"/>"
                  "<j:slug>%s</j:slug></entry>")
          (or href (format "https://example.test/atompub/alice/posts/%s" id))
          slug))

(defun jaunder-reconcile-test--page (entries &optional next)
  "Return Collection XML containing ENTRIES and optional NEXT URI."
  (concat "<feed xmlns=\"http://www.w3.org/2005/Atom\""
          " xmlns:j=\"https://jaunder.org/ns/atompub\">"
          (when next (format "<link rel=\"next\" href=\"%s\"/>" next))
          (mapconcat #'identity entries "")
          "</feed>"))

(defun jaunder-reconcile-test--member (id slug)
  "Return an inventory Member fixture with ID, SLUG, and a stable edit URI."
  (jaunder--make-inventory-member
   :id id :slug slug
   :edit-uri (format "https://example.test/atompub/alice/posts/%s" id)))

(defun jaunder-reconcile-test--local (path &optional id)
  "Return an inventory local fixture for PATH and optional ID."
  (jaunder--make-inventory-local :path path :id id))

(defun jaunder-reconcile-test--assert-total-partition (inventory locals members)
  "Assert INVENTORY owns every LOCALS and MEMBERS input exactly once by identity."
  (let ((owned-locals
         (append (jaunder-inventory-local-drafts inventory)
                 (jaunder-inventory-orphans inventory)
                 (mapcar #'jaunder-inventory-match-local
                         (jaunder-inventory-matched inventory))
                 (apply #'append
                        (mapcar #'jaunder-inventory-conflict-locals
                                (jaunder-inventory-conflicts inventory)))))
        (owned-members
         (append (jaunder-inventory-server-only inventory)
                 (mapcar #'jaunder-inventory-match-member
                         (jaunder-inventory-matched inventory))
                 (apply #'append
                        (mapcar #'jaunder-inventory-conflict-members
                                (jaunder-inventory-conflicts inventory))))))
    (dolist (local locals)
      (should (= (cl-count local owned-locals :test #'eq) 1)))
    (dolist (member members)
      (should (= (cl-count member owned-members :test #'eq) 1)))))

(ert-deftest jaunder-inventory-page-parses-members-and-one-next-in-wire-order ()
  ;; Collection Entry order survives parsing; a later page URI cannot change its grammar.
  (let* ((page (jaunder--parse-collection-page
                (jaunder-reconcile-test--page
                 (list (jaunder-reconcile-test--entry "7" "first")
                       (jaunder-reconcile-test--entry "8" "second"))
                 "https://example.test/page-2")
                "https://example.test/atompub/alice/posts"))
         (members (plist-get page :members)))
    (should (equal (mapcar #'jaunder-inventory-member-id members) '("7" "8")))
    (should (equal (mapcar #'jaunder-inventory-member-slug members) '("first" "second")))
    (should (equal (plist-get page :next) "https://example.test/page-2"))))

(ert-deftest jaunder-inventory-page-rejects-multiple-next-links ()
  ;; More than one continuation makes the Collection traversal ambiguous.
  (should-error
   (jaunder--parse-collection-page
    "<feed><link rel=\"next\" href=\"one\"/><link rel=\"next\" href=\"two\"/></feed>"
    "https://example.test/atompub/alice/posts")))

(ert-deftest jaunder-inventory-page-accepts-edit-path-under-base-prefix-only ()
  ;; A base URL path is part of the configured Collection Member grammar.
  (let ((collection "https://example.test/jaunder/api/atompub/alice/posts"))
    (should (equal
             (mapcar #'jaunder-inventory-member-id
                     (plist-get
                      (jaunder--parse-collection-page
                       (jaunder-reconcile-test--page
                        (list
                         (jaunder-reconcile-test--entry
                          "7" "prefixed"
                          "https://example.test/jaunder/api/atompub/alice/posts/7")))
                       collection)
                      :members))
             '("7")))
    (should-error
     (jaunder--parse-collection-page
      (jaunder-reconcile-test--page
       (list (jaunder-reconcile-test--entry
              "7" "missing-prefix" "https://example.test/atompub/alice/posts/7")))
      collection))))

(ert-deftest jaunder-inventory-page-rejects-malformed-or-multiple-edit-links ()
  ;; An edit Member is exactly the configured Collection path plus canonical ID.
  (dolist (entry
           (list "<entry><j:slug>x</j:slug></entry>"
                 "<entry><link rel=\"edit\" href=\"https://example.test/atompub/alice/posts/nope\"/><j:slug>x</j:slug></entry>"
                 "<entry><link rel=\"edit\" href=\"https://example.test/atompub/alice/posts/01\"/><j:slug>x</j:slug></entry>"
                 (concat "<entry><link rel=\"edit\" href=\"https://example.test/atompub/alice/posts/1\"/>"
                         "<link rel=\"edit\" href=\"https://example.test/atompub/alice/posts/2\"/><j:slug>x</j:slug></entry>")))
    (should-error (jaunder--parse-collection-page
                   (jaunder-reconcile-test--page (list entry))
                   "https://example.test/atompub/alice/posts"))))

(ert-deftest jaunder-inventory-page-rejects-missing-or-empty-slug ()
  ;; Every Member must expose its server-assigned target filename slug.
  (dolist (entry
           (list "<entry><link rel=\"edit\" href=\"https://example.test/atompub/alice/posts/1\"/></entry>"
                 "<entry><link rel=\"edit\" href=\"https://example.test/atompub/alice/posts/1\"/><j:slug></j:slug></entry>"))
    (should-error (jaunder--parse-collection-page
                   (jaunder-reconcile-test--page (list entry))
                   "https://example.test/atompub/alice/posts"))))

(ert-deftest jaunder-inventory-pagination-preserves-pages-and-rejects-cycles ()
  ;; Traversal follows each exact next URI once and never returns a partial cycle.
  (let ((responses (list
                    (cons "https://example.test/atompub/alice/posts"
                          (jaunder-reconcile-test--page
                           (list (jaunder-reconcile-test--entry "1" "one")) "page-2"))
                    (cons "page-2"
                          (jaunder-reconcile-test--page
                           (list (jaunder-reconcile-test--entry "2" "two")))))))
    (cl-letf (((symbol-function 'jaunder--http-request)
               (lambda (_method url &rest _)
                 (list :status 200 :body (cdr (assoc url responses))))))
             (let ((jaunder--active-blog '(:base-url "https://example.test" :username "alice")))
               (should (equal (mapcar #'jaunder-inventory-member-id
                                      (jaunder--fetch-collection-members))
                              '("1" "2")))))
    (setcdr (assoc "page-2" responses)
            (jaunder-reconcile-test--page
             (list (jaunder-reconcile-test--entry "2" "two"))
             "https://example.test/atompub/alice/posts"))
    (cl-letf (((symbol-function 'jaunder--http-request)
               (lambda (_method url &rest _)
                 (list :status 200 :body (cdr (assoc url responses))))))
             (let ((jaunder--active-blog '(:base-url "https://example.test" :username "alice")))
               (should-error (jaunder--fetch-collection-members))))))

(ert-deftest jaunder-inventory-pagination-rejects-non-2xx-without-result ()
  ;; A failing page is fatal; callers cannot accidentally join an earlier prefix.
  (cl-letf (((symbol-function 'jaunder--http-request)
             (lambda (&rest _) '(:status 503 :body "unavailable"))))
           (let ((jaunder--active-blog '(:base-url "https://example.test" :username "alice")))
             (should-error (jaunder--fetch-collection-members)))))

(ert-deftest jaunder-inventory-pagination-rejects-duplicate-server-id ()
  ;; One Post ID must appear once across all Collection pages.
  (let ((pages '("https://example.test/atompub/alice/posts" "next")))
    (cl-letf (((symbol-function 'jaunder--http-request)
               (lambda (_method url &rest _)
                 (list :status 200
                       :body (if (equal url (car pages))
                                 (jaunder-reconcile-test--page
                                  (list (jaunder-reconcile-test--entry "1" "one")) "next")
                               (jaunder-reconcile-test--page
                                (list (jaunder-reconcile-test--entry "1" "two"))))))))
             (let ((jaunder--active-blog '(:base-url "https://example.test" :username "alice")))
               (should-error (jaunder--fetch-collection-members))))))

(ert-deftest jaunder-inventory-scans-and-joins-complete-root-fixture ()
  ;; One real root proves sorted discovery, every local class, conflicts, and
  ;; nested-file exclusion across the scanner-to-join seam.
  (let* ((root (make-temp-file "jaunder-inventory-" t))
         (nested (expand-file-name "nested" root))
         (files '(("orphan.org" . "2")
                  ("match.org" . "1")
                  ("invalid.org" . "abc")
                  ("dup-b.org" . "3")
                  ("dup-a.org" . "3")
                  ("draft.org")))
         (members (list (jaunder-reconcile-test--member "1" "one")
                        (jaunder-reconcile-test--member "3" "three")
                        (jaunder-reconcile-test--member "5" "five"))))
    (unwind-protect
        (progn
          (make-directory nested)
          (dolist (file files)
            (write-region
             (if (cdr file)
                 (format "#+PROPERTY: JAUNDER_ID %s\n\nBody" (cdr file))
               "Body")
             nil (expand-file-name (car file) root) nil 'silent))
          (write-region "#+PROPERTY: JAUNDER_ID 99\n\nBody" nil
                        (expand-file-name "ignored.org" nested) nil 'silent)
          (let* ((locals (jaunder--scan-root-locals root))
                 (inventory (jaunder--join-inventory locals members)))
            (should (equal (mapcar (lambda (local)
                                     (file-name-nondirectory
                                      (jaunder-inventory-local-path local)))
                                   locals)
                           '("draft.org" "dup-a.org" "dup-b.org" "invalid.org"
                             "match.org" "orphan.org")))
            (should (equal (mapcar #'jaunder-inventory-local-path
                                   (jaunder-inventory-local-drafts inventory))
                           (list (expand-file-name "draft.org" root))))
            (should (equal (mapcar #'jaunder-inventory-local-path
                                   (jaunder-inventory-orphans inventory))
                           (list (expand-file-name "orphan.org" root))))
            (should (equal (mapcar #'jaunder-inventory-match-local
                                   (jaunder-inventory-matched inventory))
                           (list (nth 4 locals))))
            (should (equal (mapcar #'jaunder-inventory-member-id
                                   (jaunder-inventory-server-only inventory))
                           '("5")))
            (should (= (length (jaunder-inventory-conflicts inventory)) 2))
            (should-not (cl-find-if
                         (lambda (local)
                           (equal (file-name-nondirectory
                                   (jaunder-inventory-local-path local))
                                  "ignored.org"))
                         locals))
            (jaunder-reconcile-test--assert-total-partition
             inventory locals members)))
      (delete-directory root t))))

(ert-deftest jaunder-inventory-scan-preserves-empty-id-and-suppresses-org-hooks ()
  ;; Temporary metadata parsing neither mistakes an empty present ID for a draft nor runs user hooks.
  (let* ((root (make-temp-file "jaunder-inventory-" t))
         (empty (expand-file-name "empty.org" root))
         (hook-ran nil)
         (hook (lambda () (setq hook-ran t)))
         (org-mode-hook (list hook))
         (change-major-mode-hook (list hook))
         (after-change-major-mode-hook (list hook)))
    (unwind-protect
        (progn
          (write-region "#+PROPERTY: JAUNDER_ID \n\nBody" nil empty nil 'silent)
          (let ((local (car (jaunder--scan-root-locals root))))
            (should (equal (jaunder-inventory-local-id local) ""))
            (should-not hook-ran)
            (should (member 'invalid-local-id
                            (jaunder-inventory-conflict-kinds
                             (car (jaunder-inventory-conflicts
                                   (jaunder--join-inventory (list local) nil))))))))
      (delete-directory root t))))

(ert-deftest jaunder-inventory-join-partitions-every-ordinary-class ()
  ;; Drafts, unique matches, orphans, and server-only Members are disjoint.
  (let* ((draft (jaunder-reconcile-test--local "draft.org"))
         (match-local (jaunder-reconcile-test--local "match.org" "1"))
         (orphan (jaunder-reconcile-test--local "orphan.org" "2"))
         (member (jaunder-reconcile-test--member "1" "one"))
         (server-only (jaunder-reconcile-test--member "3" "three"))
         (inventory (jaunder--join-inventory
                     (list draft match-local orphan) (list member server-only))))
    (should (equal (jaunder-inventory-local-drafts inventory) (list draft)))
    (should (equal (jaunder-inventory-orphans inventory) (list orphan)))
    (should (equal (jaunder-inventory-server-only inventory) (list server-only)))
    (should (equal (mapcar #'jaunder-inventory-match-local
                           (jaunder-inventory-matched inventory))
                   (list match-local)))
    (should-not (jaunder-inventory-conflicts inventory))))

(ert-deftest jaunder-inventory-join-conflicts-invalid-and-duplicate-local-ids ()
  ;; Invalid IDs and same-ID locals are reported, never guessed into an ordinary class.
  (let* ((invalid (jaunder-reconcile-test--local "invalid.org" "abc"))
         (first (jaunder-reconcile-test--local "first.org" "7"))
         (second (jaunder-reconcile-test--local "second.org" "7"))
         (member (jaunder-reconcile-test--member "7" "seven"))
         (conflicts (jaunder-inventory-conflicts
                     (jaunder--join-inventory (list invalid first second) (list member)))))
    (should (= (length conflicts) 2))
    (should (member 'invalid-local-id (jaunder-inventory-conflict-kinds (car conflicts))))
    (should (member 'duplicate-local-id (jaunder-inventory-conflict-kinds (cadr conflicts))))
    (should (equal (jaunder-inventory-conflict-members (cadr conflicts)) (list member)))))

(ert-deftest jaunder-inventory-join-indexes-empty-and-overlapping-conflict-owners ()
  ;; Indexed ID and slug edges merge overlaps without duplicating ordinary or conflict ownership.
  (let* ((empty (jaunder-reconcile-test--local "empty.org" ""))
         (first (jaunder-reconcile-test--local "first.org" "1"))
         (second (jaunder-reconcile-test--local "second.org" "1"))
         (one (jaunder-reconcile-test--member "1" "shared"))
         (two (jaunder-reconcile-test--member "2" "shared"))
         (three (jaunder-reconcile-test--member "3" "single"))
         (locals (list empty first second))
         (members (list one two three))
         (inventory (jaunder--join-inventory locals members)))
    (should (= (length (jaunder-inventory-conflicts inventory)) 2))
    (should (equal (jaunder-inventory-server-only inventory) (list three)))
    (jaunder-reconcile-test--assert-total-partition inventory locals members)))

(ert-deftest jaunder-inventory-join-merges-overlapping-duplicate-seeds ()
  ;; A shared ID connects duplicate-local and duplicate-slug conditions into one group.
  (let* ((first (jaunder-reconcile-test--local "first.org" "1"))
         (second (jaunder-reconcile-test--local "second.org" "1"))
         (one (jaunder-reconcile-test--member "1" "same"))
         (two (jaunder-reconcile-test--member "2" "same"))
         (inventory (jaunder--join-inventory (list first second) (list one two)))
         (conflict (car (jaunder-inventory-conflicts inventory))))
    (should (= (length (jaunder-inventory-conflicts inventory)) 1))
    (should (member 'duplicate-local-id (jaunder-inventory-conflict-kinds conflict)))
    (should (member 'duplicate-target-slug (jaunder-inventory-conflict-kinds conflict)))
    (should (equal (jaunder-inventory-conflict-locals conflict) (list first second)))
    (should (equal (jaunder-inventory-conflict-members conflict) (list one two)))
    (should-not (jaunder-inventory-matched inventory))))

(ert-deftest jaunder-inventory-join-is-a-deterministic-total-partition ()
  ;; Inputs in a conflict are owned once; ordinary lists retain their source order.
  (let* ((draft (jaunder-reconcile-test--local "draft.org"))
         (invalid (jaunder-reconcile-test--local "invalid.org" "x"))
         (match (jaunder-reconcile-test--local "match.org" "1"))
         (orphan (jaunder-reconcile-test--local "orphan.org" "2"))
         (a (jaunder-reconcile-test--member "1" "a"))
         (b (jaunder-reconcile-test--member "3" "dup"))
         (c (jaunder-reconcile-test--member "4" "dup"))
         (inventory (jaunder--join-inventory (list draft invalid match orphan) (list a b c))))
    (should (equal (jaunder-inventory-local-drafts inventory) (list draft)))
    (should (equal (jaunder-inventory-orphans inventory) (list orphan)))
    (should-not (jaunder-inventory-server-only inventory))
    (should (equal (mapcar #'jaunder-inventory-match-local
                           (jaunder-inventory-matched inventory))
                   (list match)))
    (jaunder-reconcile-test--assert-total-partition
     inventory (list draft invalid match orphan) (list a b c))))


(ert-deftest jaunder-reconcile-classifies-all-matched-change-combinations ()
  "ETag and mtime changes form the four matched reconciliation states."
  (let ((match (jaunder--make-inventory-match
                :local (jaunder-reconcile-test--local "/tmp/match.org" "7")
                :member (jaunder-reconcile-test--member "7" "match"))))
    (dolist (fixture '((nil nil unchanged) (t nil server-ahead)
                       (nil t local-ahead) (t t conflict)))
      (let* ((server (nth 0 fixture))
             (local (nth 1 fixture))
             (row (jaunder--classify-match
                   match
                   (list :response (list :status 200
                                         :headers (list (cons "etag"
                                                              (if server "\"new\"" "\"old\"")))))
                   "\"old\"" "2026-08-25T12:00:00Z"
                   (if local (encode-time 3 0 12 25 8 2026 t)
                     (encode-time 2 0 12 25 8 2026 t)))))
        (should (eq (jaunder-reconcile-row-state row) (nth 2 fixture)))))))

(ert-deftest jaunder-reconcile-two-second-mtime-boundary-is-not-local-change ()
  "Only an mtime more than two seconds after sync marks a local change."
  (let* ((match (jaunder--make-inventory-match
                 :local (jaunder-reconcile-test--local "/tmp/match.org" "7")
                 :member (jaunder-reconcile-test--member "7" "match")))
         (outcome (list :response (list :status 200 :headers '(("etag" . "\"old\"")))))
         (synced "2026-08-25T12:00:00Z"))
    (should (eq (jaunder-reconcile-row-state
                 (jaunder--classify-match match outcome "\"old\"" synced
                                          (encode-time 2 0 12 25 8 2026 t)))
                'unchanged))
    (should (eq (jaunder-reconcile-row-state
                 (jaunder--classify-match match outcome "\"old\"" synced
                                          (encode-time 3 0 12 25 8 2026 t)))
                'local-ahead))))

(ert-deftest jaunder-reconcile-keeps-first-unclassifiable-reason ()
  "A failed Member prerequisite wins over later local marker failures."
  (let ((match (jaunder--make-inventory-match
                :local (jaunder-reconcile-test--local "/tmp/match.org" "7")
                :member (jaunder-reconcile-test--member "7" "match"))))
    (dolist (fixture
             (list
              (list (list :error '(error "offline")) nil nil nil 'member-transport-error)
              (list (list :response (list :status 404)) nil nil nil 'member-not-found)
              (list (list :response (list :status 500)) nil nil nil 'member-http-error)
              (list (list :response (list :status 200 :headers nil)) nil nil nil
                    'current-etag-invalid)
              (list (list :response (list :status 200 :headers '(("etag" . "unquoted"))))
                    "\"old\"" "2026-08-25T12:00:00Z" (encode-time 2 0 12 25 8 2026 t)
                    'current-etag-invalid)
              (list (list :response (list :status 200 :headers '(("etag" . "W/\"new\""))))
                    "\"old\"" "2026-08-25T12:00:00Z" (encode-time 2 0 12 25 8 2026 t)
                    'current-etag-invalid)
              (list (list :response (list :status 200 :headers '(("etag" . "\"new\""))))
                    nil nil nil 'stored-etag-invalid)
              (list (list :response (list :status 200 :headers '(("etag" . "\"new\""))))
                    "unquoted" "2026-08-25T12:00:00Z" (encode-time 2 0 12 25 8 2026 t)
                    'stored-etag-invalid)
              (list (list :response (list :status 200 :headers '(("etag" . "\"new\""))))
                    "W/\"old\"" "2026-08-25T12:00:00Z" (encode-time 2 0 12 25 8 2026 t)
                    'stored-etag-invalid)
              (list (list :response (list :status 200 :headers '(("etag" . "W/\"new\""))))
                    "unquoted" nil nil 'current-etag-invalid)
              (list (list :response (list :status 200 :headers '(("etag" . "\"new\""))))
                    "\"old\"" nil nil 'synced-at-invalid)
              (list (list :response (list :status 200 :headers '(("etag" . "\"new\""))))
                    "\"old\"" "2026-08-25T12:00:00Z" nil 'file-mtime-unreadable)))
      (pcase-let ((`(,outcome ,etag ,synced ,mtime ,reason) fixture))
        (should (eq (jaunder-reconcile-row-reason
                     (jaunder--classify-match match outcome etag synced mtime))
                    reason))))))

(ert-deftest jaunder-reconcile-rendering-is-persistent-and-guides-one-sided-states ()
  "Rendering has stable counts, reasons, and guidance in its report buffer."
  (let* ((local (jaunder-reconcile-test--local "/tmp/local.org" "7"))
         (member (jaunder-reconcile-test--member "7" "server"))
         (report (jaunder--make-reconcile-report
                  :root "/tmp"
                  :inventory (jaunder--make-inventory)
                  :rows (list
                         (jaunder--make-reconcile-row :state 'server-ahead
                                                      :local local :member member)
                         (jaunder--make-reconcile-row :state 'unclassifiable
                                                      :local local :member member
                                                      :reason 'stored-etag-invalid)))))
    (let ((rendered (with-current-buffer (jaunder--render-reconcile-report report)
                      (buffer-string))))
      (with-current-buffer (jaunder--render-reconcile-report report)
        (should (equal (buffer-string) rendered))
        (should (string-match-p "server-ahead (1)" (buffer-string)))
        (should (string-match-p "stored-etag-invalid" (buffer-string)))
        (should (string-match-p "Pull this Post manually" (buffer-string)))))))

(ert-deftest jaunder-reconcile-requires-an-active-blog-before-inventory ()
  "An unconfigured root fails before filesystem or network reconciliation."
  (let ((jaunder-blogs nil))
    (should-error (jaunder-reconcile "/tmp/jaunder-unconfigured-root/"))))

(ert-deftest jaunder-reconcile-applies-only-the-offered-server-only-preview ()
  "Cancellation pulls nothing; confirmation preserves the preview report."
  (let* ((root (make-temp-file "jaunder-reconcile-preview-" t))
         (jaunder-blogs
          (list (cons (file-name-as-directory root)
                      (list :base-url "https://example.test" :username "alice"))))
         (first (jaunder-reconcile-test--member "1" "first"))
         (second (jaunder-reconcile-test--member "2" "second"))
         (inventory (jaunder--make-inventory :server-only (list first second)))
         pulled)
    (unwind-protect
        (cl-letf (((symbol-function 'jaunder--inventory-for-root) (lambda (_) inventory))
                  ((symbol-function 'jaunder--pull-member)
                   (lambda (_ member) (push member pulled))))
                 (cl-letf (((symbol-function 'y-or-n-p) (lambda (_) nil)))
                          (jaunder-reconcile root))
                 (let ((rendered (with-current-buffer "*Jaunder Reconcile*" (buffer-string))))
                   (should-not pulled)
                   (cl-letf (((symbol-function 'y-or-n-p) (lambda (_) t)))
                            (jaunder-reconcile root))
                   (should (equal (nreverse pulled) (list first second)))
                   (with-current-buffer "*Jaunder Reconcile*"
                     (should (equal (buffer-string) rendered)))))
      (delete-directory root t))))

(ert-deftest jaunder-reconcile-keeps-valid-markers-when-mtime-is-unreadable ()
  "An mtime failure classifies a valid matched Post as file-mtime-unreadable."
  (let* ((root (make-temp-file "jaunder-reconcile-markers-" t))
         (path (expand-file-name "matched.org" root))
         (local (jaunder-reconcile-test--local path "7"))
         (match (jaunder--make-inventory-match
                 :local local :member (jaunder-reconcile-test--member "7" "matched")))
         (outcome (list :response
                        (list :status 200 :headers '(("etag" . "\"old\""))))))
    (unwind-protect
        (progn
          (with-temp-file path
            (insert "#+PROPERTY: JAUNDER_SYNCED \"old\"\n"
                    "#+PROPERTY: JAUNDER_SYNCED_AT 2026-08-25T12:00:00Z\n"))
          (cl-letf (((symbol-function 'file-attributes)
                     (lambda (&rest _) (error "unreadable mtime"))))
                   (let ((markers (jaunder--reconcile-local-markers local)))
                     (should (equal (list (nth 0 markers) (nth 1 markers))
                                    '("\"old\"" "2026-08-25T12:00:00Z")))
                     (should (eq (jaunder-reconcile-row-reason
                                  (jaunder--classify-match match outcome
                                                           (nth 0 markers)
                                                           (nth 1 markers)
                                                           (nth 2 markers)))
                                 'file-mtime-unreadable)))))
      (delete-directory root t))))

(ert-deftest jaunder-reconcile-selects-the-most-specific-configured-root ()
  "Nested reconciliation resolves its active blog and inventory root by longest prefix."
  (let* ((parent (make-temp-file "jaunder-reconcile-parent-" t))
         (child (expand-file-name "nested/" parent))
         (descendant (expand-file-name "descendant/" child))
         (jaunder-blogs
          (list (cons (file-name-as-directory parent)
                      (list :base-url "https://parent.test" :username "parent"))
                (cons child (list :base-url "https://child.test" :username "child"))))
         observed)
    (make-directory descendant t)
    (unwind-protect
        (cl-letf (((symbol-function 'jaunder--inventory-for-root)
                   (lambda (root)
                     (setq observed
                           (list root (jaunder--active-base-url)
                                 (jaunder--active-username)))
                     (jaunder--make-inventory))))
                 (jaunder-reconcile descendant)
                 (should (equal observed (list child "https://child.test" "child"))))
      (delete-directory parent t))))

(ert-deftest jaunder-reconcile-preserves-inventory-only-classes-and-conflict-details ()
  "D1 classes remain distinct reconciliation rows and groups remain intact."
  (let* ((draft (jaunder-reconcile-test--local "/tmp/draft.org"))
         (orphan (jaunder-reconcile-test--local "/tmp/orphan.org" "3"))
         (server (jaunder-reconcile-test--member "4" "server"))
         (local (jaunder-reconcile-test--local "/tmp/duplicate.org" "5"))
         (member (jaunder-reconcile-test--member "5" "duplicate"))
         (conflict (jaunder--make-inventory-conflict
                    :kinds '(duplicate-local-id duplicate-target-slug)
                    :locals (list local) :members (list member)))
         (inventory (jaunder--make-inventory :local-drafts (list draft)
                                             :orphans (list orphan)
                                             :server-only (list server)
                                             :conflicts (list conflict)))
         (report (jaunder--reconcile-build-report "/tmp" inventory)))
    (should (equal (mapcar #'jaunder-reconcile-row-state
                           (jaunder-reconcile-report-rows report))
                   '(orphan local-draft server-only inventory-conflict)))
    (with-current-buffer (jaunder--render-reconcile-report report)
      (should (string-match-p "inventory-conflict (1)" (buffer-string)))
      (should (string-match-p "duplicate-local-id, duplicate-target-slug"
                              (buffer-string)))
      (should (string-match-p "local: /tmp/duplicate.org id=5" (buffer-string)))
      (should (string-match-p "slug=duplicate" (buffer-string))))))
(provide 'jaunder-reconcile-test)
;;; jaunder-reconcile-test.el ends here
