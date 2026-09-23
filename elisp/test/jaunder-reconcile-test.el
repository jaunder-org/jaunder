;;; jaunder-reconcile-test.el --- Inventory behavior tests -*- lexical-binding: t; -*-

;; Copyright (C) 2026 Jaunder contributors

;;; Commentary:
;; Focused pure contracts for AtomPub Collection inventory parsing, local discovery,
;; and conflict-safe joining.

;;; Code:

(require 'ert)
(require 'cl-lib)
(require 'jaunder)

(defun jaunder-reconcile-test--legacy-service-document (&rest _)
  "Return valid legacy capability evidence for independent reconcile tests."
  (jaunder--parse-service-document
   "<service xmlns=\"http://www.w3.org/2007/app\"><workspace/></service>"))

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

(defun jaunder-reconcile-test--member-entry (alternates)
  "Return one valid Member Entry XML with ALTERNATES as alternate hrefs."
  (concat "<entry xmlns=\"http://www.w3.org/2005/Atom\" xmlns:j=\"https://jaunder.org/ns/atompub\"><link rel=\"edit\" href=\"https://example.test/atompub/alice/posts/7\"/>"
          (mapconcat (lambda (href)
                       (format "<link rel=\"alternate\" href=\"%s\"/>" href))
                     alternates "")
          "<j:slug>post</j:slug></entry>"))

(defun jaunder-reconcile-test--local (path &optional id)
  "Return an inventory local fixture for PATH and optional ID.
The current filename supplies the local slug evidence used by matched-pull tests."
  (jaunder--make-inventory-local
   :path path :id id :slug (file-name-base path)))

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

(ert-deftest jaunder-inventory-page-reads-only-one-valid-member-etag ()
  "Member validators are expanded-name XML data, not textual-prefix matches."
  (let ((entry (jaunder-reconcile-test--entry "7" "first")))
    (dolist (fixture '(("<j:etag>&quot;current&quot;</j:etag>" . "\"current\"")
                       ("<v:etag xmlns:v=\"https://jaunder.org/ns/atompub\">&quot;current&quot;</v:etag>" . "\"current\"")
                       ("<j:etag>W/&quot;current&quot;</j:etag>" . nil)
                       ("<j:etag> &quot;current&quot;</j:etag>" . nil)
                       ("<j:etag extra=\"1\">&quot;current&quot;</j:etag>" . nil)
                       ("<j:etag><j:inner/>&quot;current&quot;</j:etag>" . nil)
                       ("<v:etag xmlns:v=\"urn:foreign\">&quot;current&quot;</v:etag>" . nil)
                       ("<j:etag>&quot;current&quot;</j:etag><j:etag>&quot;current&quot;</j:etag>" . nil)))
      (let* ((xml (replace-regexp-in-string
                   "</entry>" (concat (car fixture) "</entry>") entry t t))
             (page (jaunder--parse-collection-page
                    (jaunder-reconcile-test--page (list xml))
                    "https://example.test/atompub/alice/posts"))
             (member (car (plist-get page :members))))
        (should (equal (jaunder-inventory-member-etag member) (cdr fixture)))))))

(ert-deftest jaunder-inventory-page-rejects-multiple-next-links ()
  ;; More than one continuation makes the Collection traversal ambiguous.
  (should-error
   (jaunder--parse-collection-page
    "<feed><link rel=\"next\" href=\"one\"/><link rel=\"next\" href=\"two\"/></feed>"
    "https://example.test/atompub/alice/posts")))

(ert-deftest jaunder-inventory-page-rejects-root-and-namespace-parse-drift ()
  "Both parser views must identify one feed and the same Member count."
  (should-error
   (jaunder--parse-collection-page
    "<entry/>" "https://example.test/atompub/alice/posts"))
  (cl-letf (((symbol-function 'jaunder--parse-collection-xml-namespaced)
             (lambda (_) '(feed nil (entry nil)))))
    (should-error
     (jaunder--parse-collection-page
      "<feed/>" "https://example.test/atompub/alice/posts"))))

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
         messages
         (hook (lambda () (setq hook-ran t)))
         (org-mode-hook (list hook))
         (change-major-mode-hook (list hook))
         (after-change-major-mode-hook (list hook)))
    (unwind-protect
        (progn
          (write-region "#+PROPERTY: JAUNDER_ID \n\nBody" nil empty nil 'silent)
          (cl-letf (((symbol-function 'message)
                     (lambda (format-string &rest args)
                       (push (apply #'format-message format-string args) messages))))
            (let ((local (car (jaunder--scan-root-locals root))))
              (should (equal (jaunder-inventory-local-id local) ""))
              (should-not hook-ran)
              (should-not
               (cl-find-if
                (lambda (text)
                  (string-match-p "Making change-major-mode-hook buffer-local" text))
                messages))
              (should (member 'invalid-local-id
                              (jaunder-inventory-conflict-kinds
                               (car (jaunder-inventory-conflicts
                                     (jaunder--join-inventory (list local) nil)))))))))
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

(ert-deftest jaunder-inventory-join-matches-repaired-unique-target-slugs-by-id ()
  ;; Migration keeps the newest base filename and gives older Posts unique suffixes;
  ;; stable IDs then join every local file without weakening duplicate detection.
  (let* ((oldest (jaunder-reconcile-test--local "shared-1.org" "1"))
         (middle (jaunder-reconcile-test--local "shared-2.org" "2"))
         (newest (jaunder-reconcile-test--local "shared.org" "3"))
         (oldest-member (jaunder-reconcile-test--member "1" "shared-1"))
         (middle-member (jaunder-reconcile-test--member "2" "shared-2"))
         (newest-member (jaunder-reconcile-test--member "3" "shared"))
         (inventory
          (jaunder--join-inventory
           (list oldest middle newest)
           (list oldest-member middle-member newest-member))))
    (should (= (length (jaunder-inventory-matched inventory)) 3))
    (should-not (jaunder-inventory-conflicts inventory))
    (should-not (jaunder-inventory-server-only inventory))
    (should-not (jaunder-inventory-orphans inventory))))

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


(ert-deftest jaunder-inventory-member-retains-alternate-outcomes ()
  "Alternate failures stay on their Member with stable, typed reasons."
  (let ((collection "https://example.test/atompub/alice/posts"))
    (dolist (fixture
             '((() alternate-missing)
               (("not a URL") alternate-malformed)
               (("https://example.test/posts/bad path") alternate-malformed)
               (("https://example.test/posts/%ZZ") alternate-malformed)
               (("https://example.test:/posts/post") alternate-malformed)
               (("https://example.test:abc/posts/post") alternate-malformed)
               (("https://example.test:70000/posts/post") alternate-malformed)
               (("https://user@example.test/posts/post") alternate-user-info)
               (("https://other.test/posts/post") alternate-cross-origin)
               (("https://[::1]/posts/post") alternate-cross-origin)
               (("https://example.test/posts/exact?query")
                alternate-query-or-fragment)
               (("https://example.test/posts/exact#fragment")
                alternate-query-or-fragment)
               (("https://example.test/posts/post" "https://example.test/posts/post")
                alternate-duplicate)))
      (let ((member (car (plist-get
                          (jaunder--parse-collection-page
                           (jaunder-reconcile-test--page
                            (list (jaunder-reconcile-test--member-entry (car fixture))))
                           collection)
                          :members))))
        (should (eq (jaunder-inventory-member-alternate-invalid-reason member)
                    (cadr fixture)))
        (should-not (jaunder-inventory-member-alternate-href member))))
    (let ((member (car (plist-get
                        (jaunder--parse-collection-page
                         (jaunder-reconcile-test--page
                          (list (jaunder-reconcile-test--member-entry
                                 '("https://example.test/posts/exact"))))
                         collection)
                        :members))))
      (should (equal (jaunder-inventory-member-alternate-href member)
                     "https://example.test/posts/exact"))
      (should-not (jaunder-inventory-member-alternate-invalid-reason member)))
    ;; A direct parse falls back to ENTRY when no namespace-preserving peer is supplied.
    (let* ((entry (jaunder--parse-collection-xml
                   (jaunder-reconcile-test--member-entry
                    '("https://example.test/posts/direct"))))
           (member (jaunder--parse-collection-member entry collection)))
      (should-not (jaunder-inventory-member-alternate-href member))
      (should (eq (jaunder-inventory-member-alternate-invalid-reason member)
                  'alternate-missing)))))

(ert-deftest jaunder-inventory-local-retains-id-slug-and-filename-evidence ()
  "Local inventory preserves evidence without treating filename as identity."
  (let* ((root (make-temp-file "jaunder-inventory-" t))
         (path (expand-file-name "wrong-name.org" root)))
    (unwind-protect
        (progn
          (write-region (concat "#+PROPERTY: JAUNDER_ID 7\n"
                                "#+PROPERTY: JAUNDER_SLUG expected-name\n\nBody")
                        nil path nil 'silent)
          (let* ((local (car (jaunder--scan-root-locals root)))
                 (member (jaunder-reconcile-test--member "7" "expected-name"))
                 (expected-path (expand-file-name "expected-name.org" root)))
            (should (equal (jaunder-inventory-local-id local) "7"))
            (should (equal (jaunder-inventory-local-slug local) "expected-name"))
            (should (eq (jaunder--inventory-local-member-evidence-reason local member)
                        'local-filename-mismatch))
            (should (eq
                     (jaunder--inventory-local-member-evidence-reason
                      (jaunder--make-inventory-local
                       :path expected-path :id "8" :slug "expected-name")
                      member)
                     'local-id-mismatch))
            (should (eq
                     (jaunder--inventory-local-member-evidence-reason
                      (jaunder--make-inventory-local
                       :path expected-path :id "7" :slug "other")
                      member)
                     'local-slug-mismatch))
            (should-not
             (jaunder--inventory-local-member-evidence-reason
              (jaunder--make-inventory-local
               :path expected-path :id "7" :slug "expected-name")
              member))))
      (delete-directory root t))))

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

(ert-deftest jaunder-reconcile-matched-preview-uses-page-etags-and-falls-back-per-row ()
  "Page validators avoid matched reads; old servers still expose read failures."
  (let* ((first (jaunder--make-inventory-member
                 :id "7" :slug "one" :etag "\"new\""
                 :edit-uri "https://example.test/atompub/alice/posts/7"))
         (second (jaunder--make-inventory-member
                  :id "8" :slug "two" :etag "\"old\""
                  :edit-uri "https://example.test/atompub/alice/posts/8"))
         (older (jaunder-reconcile-test--member "9" "three"))
         (inventory (jaunder--make-inventory
                     :matched (cl-loop for member in (list first second older)
                                       collect (jaunder--make-inventory-match
                                                :local (jaunder-reconcile-test--local
                                                        (format "/tmp/%s.org"
                                                                (jaunder-inventory-member-slug member))
                                                        (jaunder-inventory-member-id member))
                                                :member member))))
         requests)
    (cl-letf (((symbol-function 'jaunder--reconcile-local-markers)
               (lambda (_) (list "\"old\"" "2026-08-25T12:00:00Z" nil
                                 (encode-time 2 0 12 25 8 2026 t))))
              ((symbol-function 'jaunder--http-request)
               (lambda (_method url)
                 (push url requests)
                 (error "offline"))))
      (let ((rows (jaunder-reconcile-report-rows
                   (jaunder--reconcile-build-report "/tmp" inventory))))
        (should (equal (mapcar #'jaunder-reconcile-row-state rows)
                       '(server-ahead unchanged unclassifiable)))
        (should (eq (jaunder-reconcile-row-reason (nth 2 rows))
                    'member-transport-error))
        (should (equal requests (list (jaunder-inventory-member-edit-uri older))))))))

(ert-deftest jaunder-reconcile-persisted-local-ahead-beats-mtime-tolerance ()
  "Recovery's explicit marker survives a within-tolerance write-back."
  (let* ((match (jaunder--make-inventory-match
                 :local (jaunder-reconcile-test--local "/tmp/match.org" "7")
                 :member (jaunder-reconcile-test--member "7" "match")))
         (outcome (list :response (list :status 200 :headers '(("etag" . "\"old\"")))))
         (synced "2026-08-25T12:00:00Z"))
    (should (eq (jaunder-reconcile-row-state
                 (jaunder--classify-match
                  match outcome "\"old\"" synced
                  (encode-time 1 0 12 25 8 2026 t) "true"))
                'local-ahead))))

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
                                                      :reason 'stored-etag-invalid)
                         (jaunder--make-reconcile-row :state 'unclassifiable
                                                      :local local :member member
                                                      :reason 'member-http-error :detail 503)))))
    (let ((rendered (with-current-buffer (jaunder--render-reconcile-report report)
                      (buffer-string))))
      (with-current-buffer (jaunder--render-reconcile-report report)
        (should (equal (buffer-string) rendered))
        (should (string-match-p "server-ahead (1)" (buffer-string)))
        (should (string-match-p "stored-etag-invalid" (buffer-string)))
        (should (string-match-p "member-http-error (503)" (buffer-string)))))))

(ert-deftest jaunder-reconcile-requires-an-active-blog-before-inventory ()
  "An unconfigured root fails before filesystem or network reconciliation."
  (let ((jaunder-blogs nil))
    (should-error (jaunder-reconcile "/tmp/jaunder-unconfigured-root/"))))

(ert-deftest jaunder-reconcile-displays-server-only-rows-without-an-operation-binding ()
  "Reconciliation does not prompt for or perform a transfer on its own."
  (let* ((root (make-temp-file "jaunder-reconcile-preview-" t))
         (jaunder-blogs
          (list (cons (file-name-as-directory root)
                      (list :base-url "https://example.test" :username "alice"))))
         (member (jaunder-reconcile-test--member "1" "first"))
         (inventory (jaunder--make-inventory :server-only (list member))))
    (unwind-protect
        (cl-letf (((symbol-function 'jaunder--inventory-for-root) (lambda (_) inventory))
                  ((symbol-function 'y-or-n-p)
                   (lambda (&rest _) (error "must not prompt"))))
          (jaunder-reconcile root)
          (with-current-buffer "*Jaunder Reconcile*"
            (should (string-match-p "server-only (1)" (buffer-string)))))
      (delete-directory root t))))

(ert-deftest jaunder-reconcile-initial-and-manual-refresh-show-synchronous-progress ()
  "Both interactive paths display work before I/O and finish truthfully."
  (let* ((root (file-name-as-directory (make-temp-file "jaunder-progress-" t)))
         (jaunder-blogs (list (cons root '(:base-url "https://example.test"
                                                     :username "alice"))))
         (inventory (jaunder--make-inventory))
         events fail)
    (unwind-protect
        (cl-letf (((symbol-function 'message)
                   (lambda (format-string &rest args)
                     (push (apply #'format format-string args) events)))
                  ((symbol-function 'redisplay) (lambda (&rest _) (push 'paint events)))
                  ((symbol-function 'jaunder--inventory-for-root)
                   (lambda (_) (push 'inventory events)
                     (pcase fail
                       ('error (error "offline"))
                       ('quit (signal 'quit nil))
                       (_ inventory))))
                  ((symbol-function 'display-buffer) (lambda (&rest _) nil)))
          (jaunder-reconcile root)
          (should (equal (nreverse events)
                         '("Jaunder reconcile: fetching and classifying Posts..."
                           paint inventory "Jaunder reconcile: report ready")))
          (setq events nil)
          (with-current-buffer "*Jaunder Reconcile*"
            (jaunder-reconcile-refresh))
          (should (equal (nreverse events)
                         '("Jaunder reconcile: fetching and classifying Posts..."
                           paint inventory "Jaunder reconcile: report ready")))
          (setq events nil fail 'error)
          (with-current-buffer "*Jaunder Reconcile*"
            (let ((old-report jaunder-reconcile-report)
                  (old-text (buffer-string)))
              (should-error (jaunder-reconcile-refresh))
              (should (eq jaunder-reconcile-report old-report))
              (should (equal (buffer-string) old-text))))
          (should (equal (nreverse events)
                         '("Jaunder reconcile: fetching and classifying Posts..."
                           paint inventory "Jaunder reconcile: report refresh failed")))
          (setq events nil fail 'quit)
          (with-current-buffer "*Jaunder Reconcile*"
            (let ((old-report jaunder-reconcile-report)
                  (old-text (buffer-string)))
              (should (eq (condition-case nil
                              (jaunder-reconcile-refresh)
                            (quit 'cancelled))
                          'cancelled))
              (should (eq jaunder-reconcile-report old-report))
              (should (equal (buffer-string) old-text))))
          (should (equal (nreverse events)
                         '("Jaunder reconcile: fetching and classifying Posts..."
                           paint inventory "Jaunder reconcile: report refresh failed"))))
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
                                                    (nth 3 markers)
                                                    (nth 2 markers)))
                          'file-mtime-unreadable)))))
      (delete-directory root t))))

(ert-deftest jaunder-reconcile-missing-local-file-clears-all-marker-inputs ()
  "A vanished local file yields no saved marker or mtime state."
  (let ((local (jaunder-reconcile-test--local
                "/definitely-missing/jaunder-post.org" "7")))
    (should (equal (jaunder--reconcile-local-markers local)
                   '(nil nil nil nil)))))

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
(ert-deftest jaunder-reconcile-rejects-malformed-page-and-retains-fetch-errors ()
  "Malformed collection wire data and one Member fetch failure stay explicit."
  (cl-letf (((symbol-function 'libxml-parse-xml-region)
             (lambda (&rest _) (error "malformed"))))
    (should-error (jaunder--parse-collection-xml "<feed>")))
  (should-error (jaunder--parse-collection-page "<not xml" "https://h/posts"))
  (should-error
   (jaunder--parse-collection-page
    "<feed><link rel=\"next\" href=\"\"/></feed>" "https://h/posts"))
  (let ((member (jaunder-reconcile-test--member "1" "one")))
    (cl-letf (((symbol-function 'jaunder--http-request)
               (lambda (&rest _) (error "offline"))))
      (should (plist-get (jaunder--reconcile-member-outcome member) :error)))))

(ert-deftest jaunder-reconcile-interactive-uses-default-directory ()
  "Interactive reconciliation resolves and returns the default root's report."
  (let* ((root (file-name-as-directory (make-temp-file "jaunder-reconcile-" t)))
         (default-directory root)
         (inventory (jaunder--make-inventory))
         (report (jaunder--make-reconcile-report :root root :rows nil))
         (jaunder-blogs
          (list (cons root '(:base-url "https://h" :username "alice")))))
    (unwind-protect
        (cl-letf (((symbol-function 'jaunder--inventory-for-root)
                   (lambda (configured-root)
                     (should (equal configured-root root))
                     inventory))
                  ((symbol-function 'jaunder--reconcile-build-report)
                   (lambda (&rest _) report))
                  ((symbol-function 'jaunder--render-reconcile-report)
                   (lambda (&rest _) (get-buffer-create " *jr-interactive*")))
                  ((symbol-function 'display-buffer) (lambda (&rest _) nil)))
          (should (eq (call-interactively #'jaunder-reconcile) report)))
      (delete-directory root t))))
(ert-deftest jaunder-reconcile-selection-resolves-marks-and-region-in-display-order ()
  "Marked rows and a contiguous region resolve without changing row state."
  (let* ((first (jaunder--make-reconcile-row :state 'local-draft
                                             :key "local:/one.org"))
         (second (jaunder--make-reconcile-row :state 'conflict
                                              :key "post:2"))
         (third (jaunder--make-reconcile-row :state 'server-only
                                             :key "post:3"))
         (report (jaunder--make-reconcile-report :root "/tmp"
                                                 :rows (list first second third)))
         (marks (make-hash-table :test #'equal)))
    (puthash "post:3" t marks)
    (puthash "local:/one.org" t marks)
    (should (equal (jaunder--reconcile-resolve-selection report marks nil)
                   (list first third)))
    (should (equal (jaunder--reconcile-resolve-selection report marks
                                                         '(1 . 2))
                   (list first third)))
    (should (eq (jaunder-reconcile-row-state second) 'conflict))))

(ert-deftest jaunder-reconcile-batch-records-progress-complete-results-and-order ()
  "The shared executor records one complete terminal result per displayed row."
  (let* ((rows (list (jaunder--make-reconcile-row :state 'local-draft :key "one")
                     (jaunder--make-reconcile-row :state 'local-ahead :key "two")))
         (report (jaunder--make-reconcile-report :root "/tmp" :rows rows))
         (buffer (jaunder--render-reconcile-report report))
         progress)
    (unwind-protect
        (cl-letf (((symbol-function 'jaunder--reconcile-refresh-buffer)
                   (lambda (&rest _) nil))
                  ((symbol-function 'message)
                   (lambda (format-string &rest arguments)
                     (push (apply #'format format-string arguments) progress))))
          (jaunder--reconcile-execute-batch
           buffer rows 'push
           (lambda (row)
             (list :outcome 'success
                   :post-id (jaunder-reconcile-row-key row)
                   :slug (concat "slug-" (jaunder-reconcile-row-key row))
                   :etag "\"current\"" :synced-at "2026-09-17T12:00:00Z"
                   :http-status 200 :local-effect 'updated)))
          (let ((results (with-current-buffer buffer jaunder-reconcile-last-batch-results)))
            (should (equal (mapcar #'jaunder-reconcile-result-row-key results)
                           '("two" "one")))
            (should (equal (mapcar #'jaunder-reconcile-result-action results)
                           '(push push)))
            (should (equal (mapcar #'jaunder-reconcile-result-outcome results)
                           '(success success)))
            (dolist (result results)
              (should (jaunder-reconcile-result-post-id result))
              (should (jaunder-reconcile-result-slug result))
              (should (jaunder-reconcile-result-etag result))
              (should (jaunder-reconcile-result-synced-at result))
              (should (integerp (jaunder-reconcile-result-http-status result)))))
          (should (equal (nreverse progress)
                         '("Jaunder push: 1/2" "Jaunder push: 2/2"))))
      (kill-buffer buffer))))

(ert-deftest jaunder-reconcile-batch-renders-failure-after-refresh-and-omits-delete-sync ()
  "A refreshed report retains failed terminal results and delete has no sync time."
  (let* ((row (jaunder--make-reconcile-row :state 'server-only :key "post:7"))
         (report (jaunder--make-reconcile-report :root "/tmp" :rows (list row)))
         (buffer (jaunder--render-reconcile-report report)))
    (unwind-protect
        (cl-letf (((symbol-function 'jaunder--reconcile-refresh-buffer)
                   (lambda (target)
                     (jaunder--render-reconcile-report
                      (jaunder--make-reconcile-report :root "/tmp" :rows nil) target))))
          (jaunder--reconcile-execute-batch
           buffer (list row) 'delete
           (lambda (_)
             (list :outcome 'failed :post-id "7" :slug "gone" :etag "\"old\""
                   :synced-at "must-not-appear" :http-status 412
                   :local-effect 'unchanged :reason 'etag-stale :detail "refresh")))
          (with-current-buffer buffer
            (let ((result (car jaunder-reconcile-last-batch-results)))
              (should-not (jaunder-reconcile-result-synced-at result))
              (should (string-match-p "Last batch" (buffer-string)))
              (should (string-match-p "etag-stale (refresh)" (buffer-string))))))
      (kill-buffer buffer))))

(ert-deftest jaunder-reconcile-batch-cancels-between-items-and-refreshes ()
  "Cancellation retains completed results and does not invoke the next operation."
  (let* ((rows (cl-loop for index below 3
                        collect (jaunder--make-reconcile-row
                                 :state 'local-draft :key (number-to-string index))))
         (buffer (jaunder--render-reconcile-report
                  (jaunder--make-reconcile-report :root "/tmp" :rows rows)))
         invoked refreshes)
    (unwind-protect
        (cl-letf (((symbol-function 'jaunder--reconcile-refresh-buffer)
                   (lambda (&rest _) (setq refreshes (1+ (or refreshes 0))))))
          (should (eq (jaunder--reconcile-execute-batch
                       buffer rows 'push
                       (lambda (row) (push (jaunder-reconcile-row-key row) invoked)
                         (list :outcome 'success :local-effect 'created))
                       (lambda () (= (length invoked) 1)))
                      'cancelled))
          (should (equal (nreverse invoked) '("0")))
          (with-current-buffer buffer
            (should (= (length jaunder-reconcile-last-batch-results) 1)))
          (should (= refreshes 1)))
      (kill-buffer buffer))))

(ert-deftest jaunder-reconcile-batch-executes-one-thousand-items-sequentially ()
  "A large batch stays ordered, single-flight, and continues after one failure."
  (let* ((rows (cl-loop for index below 1000
                        collect (jaunder--make-reconcile-row
                                 :state 'local-draft :key (format "post:%04d" index))))
         (buffer (jaunder--render-reconcile-report
                  (jaunder--make-reconcile-report :root "/tmp" :rows rows)))
         (in-flight 0) (maximum-in-flight 0) invoked)
    (unwind-protect
        (cl-letf (((symbol-function 'jaunder--reconcile-refresh-buffer)
                   (lambda (&rest _) nil)))
          (should (eq (jaunder--reconcile-execute-batch
                       buffer rows 'push
                       (lambda (row)
                         (setq in-flight (1+ in-flight)
                               maximum-in-flight (max maximum-in-flight in-flight))
                         (unwind-protect
                             (progn
                               (push (jaunder-reconcile-row-key row) invoked)
                               (if (equal (jaunder-reconcile-row-key row) "post:0500")
                                   (error "injected independent failure")
                                 (list :outcome 'success :local-effect 'created)))
                           (setq in-flight (1- in-flight)))))
                      'completed))
          (with-current-buffer buffer
            (should (= (length jaunder-reconcile-last-batch-results) 1000))
            (should (equal (mapcar #'jaunder-reconcile-result-row-key
                                   jaunder-reconcile-last-batch-results)
                           (mapcar #'jaunder-reconcile-row-key rows)))
            (should (eq (jaunder-reconcile-result-outcome
                         (nth 500 jaunder-reconcile-last-batch-results))
                        'failed))
            (should (= (cl-loop for result in jaunder-reconcile-last-batch-results
                                count (eq (jaunder-reconcile-result-outcome result)
                                          'success))
                       999)))
          (should (= maximum-in-flight 1))
          (should (equal (nreverse invoked) (mapcar #'jaunder-reconcile-row-key rows))))
      (kill-buffer buffer))))

(ert-deftest jaunder-reconcile-interactive-marking-preserves-point-for-multiple-rows ()
  "Toggling a mark keeps point on its row so another row can be marked."
  (let* ((first (jaunder--make-reconcile-row :state 'local-draft :key "first"))
         (second (jaunder--make-reconcile-row :state 'server-only :key "second"))
         (report (jaunder--make-reconcile-report :root "/tmp" :rows (list first second)))
         (buffer (jaunder--render-reconcile-report report)))
    (unwind-protect
        (with-current-buffer buffer
          (goto-char (jaunder--reconcile-row-key-position "first"))
          (jaunder-reconcile-toggle-mark)
          (should (equal (get-text-property (point) 'jaunder-reconcile-row-key) "first"))
          (goto-char (jaunder--reconcile-row-key-position "second"))
          (jaunder-reconcile-toggle-mark)
          (should (equal (jaunder-reconcile-selected-rows) (list first second))))
      (kill-buffer buffer))))

(ert-deftest jaunder-reconcile-rendering-and-selection-use-one-displayed-order ()
  "Interleaved inventory rows resolve and execute in the rendered section order."
  (let* ((local-ahead (jaunder--make-reconcile-row :state 'local-ahead :key "ahead"))
         (server-only (jaunder--make-reconcile-row :state 'server-only :key "server"))
         (conflict (jaunder--make-reconcile-row :state 'conflict :key "conflict"))
         (draft (jaunder--make-reconcile-row :state 'local-draft :key "draft"))
         (rows (list local-ahead server-only conflict draft))
         (report (jaunder--make-reconcile-report :root "/tmp" :rows rows))
         (marks (make-hash-table :test #'equal))
         (buffer (jaunder--render-reconcile-report report))
         rendered invoked)
    (unwind-protect
        (progn
          (with-current-buffer buffer
            (let ((position (point-min)))
              (while (< position (point-max))
                (let ((row (get-text-property position 'jaunder-reconcile-row))
                      (next (next-single-property-change
                             position 'jaunder-reconcile-row nil (point-max))))
                  (when row (push (jaunder-reconcile-row-key row) rendered))
                  (setq position next)))))
          (puthash "draft" t marks)
          (puthash "server" t marks)
          (should (equal (nreverse rendered) '("ahead" "conflict" "draft" "server")))
          (should (equal (mapcar #'jaunder-reconcile-row-key
                                 (jaunder--reconcile-resolve-selection report marks nil))
                         '("draft" "server")))
          (should (equal (mapcar #'jaunder-reconcile-row-key
                                 (jaunder--reconcile-resolve-selection report marks '(1 . 2)))
                         '("conflict" "draft")))
          (cl-letf (((symbol-function 'jaunder--reconcile-refresh-buffer)
                     (lambda (&rest _) nil)))
            (jaunder--reconcile-execute-batch
             buffer rows 'push
             (lambda (row)
               (push (jaunder-reconcile-row-key row) invoked)
               (list :outcome 'success :local-effect 'created))))
          (should (equal (nreverse invoked) '("ahead" "conflict" "draft" "server"))))
      (kill-buffer buffer))))

(ert-deftest jaunder-reconcile-active-heading-region-does-not-fall-back-to-marks ()
  "An active region that touches no row selects no marked rows."
  (let* ((row (jaunder--make-reconcile-row :state 'server-only :key "server"))
         (buffer (jaunder--render-reconcile-report
                  (jaunder--make-reconcile-report :root "/tmp" :rows (list row)))))
    (unwind-protect
        (with-current-buffer buffer
          (puthash "server" t jaunder-reconcile-marks)
          (goto-char (point-min))
          (search-forward "unchanged (0)")
          (set-mark (line-beginning-position))
          (goto-char (line-end-position))
          (setq mark-active t transient-mark-mode t)
          (should-not (jaunder-reconcile-selected-rows)))
      (kill-buffer buffer))))

(ert-deftest jaunder-reconcile-batch-cancellation-is-sticky-and-clears-quit-before-refresh ()
  "One-shot cancellation and a final-operation quit stop after the completed row."
  (let* ((rows (list (jaunder--make-reconcile-row :state 'local-draft :key "one")
                     (jaunder--make-reconcile-row :state 'local-draft :key "two")))
         (buffer (jaunder--render-reconcile-report
                  (jaunder--make-reconcile-report :root "/tmp" :rows rows)))
         invoked refresh-saw-quit checks)
    (unwind-protect
        (cl-letf (((symbol-function 'jaunder--reconcile-refresh-buffer)
                   (lambda (&rest _) (setq refresh-saw-quit quit-flag))))
          (should (eq (jaunder--reconcile-execute-batch
                       buffer rows 'push
                       (lambda (row)
                         (push (jaunder-reconcile-row-key row) invoked)
                         (list :outcome 'success :local-effect 'created))
                       (lambda () (= (setq checks (1+ (or checks 0))) 2)))
                      'cancelled))
          (should (equal (nreverse invoked) '("one")))
          (should-not refresh-saw-quit)
          (setq invoked nil)
          (should (eq (jaunder--reconcile-execute-batch
                       buffer (list (car rows)) 'push
                       (lambda (row)
                         (push (jaunder-reconcile-row-key row) invoked)
                         (setq quit-flag t)
                         (list :outcome 'success :local-effect 'created)))
                      'cancelled))
          (should (equal invoked '("one")))
          (should-not refresh-saw-quit))
      (setq quit-flag nil)
      (kill-buffer buffer))))

(ert-deftest jaunder-reconcile-batches-replace-summaries-and-append-before-next-item ()
  "The next batch replaces its summary and each next operation sees prior results."
  (let* ((first (jaunder--make-reconcile-row :state 'local-draft :key "one"))
         (second (jaunder--make-reconcile-row :state 'local-draft :key "two"))
         (report (jaunder--make-reconcile-report :root "/tmp" :rows (list first second)))
         (buffer (jaunder--render-reconcile-report report))
         prior-visible)
    (unwind-protect
        (cl-letf (((symbol-function 'jaunder--reconcile-refresh-buffer)
                   (lambda (target) (jaunder--render-reconcile-report report target))))
          (jaunder--reconcile-execute-batch
           buffer (list first second) 'push
           (lambda (row)
             (when (equal (jaunder-reconcile-row-key row) "two")
               (setq prior-visible
                     (with-current-buffer buffer
                       (equal (mapcar #'jaunder-reconcile-result-row-key
                                      jaunder-reconcile-last-batch-results)
                              '("one")))))
             (list :outcome 'success :local-effect 'created)))
          (should prior-visible)
          (jaunder--reconcile-execute-batch
           buffer (list second) 'delete
           (lambda (_) (list :outcome 'success :local-effect 'unchanged)))
          (with-current-buffer buffer
            (should (equal (mapcar #'jaunder-reconcile-result-action
                                   jaunder-reconcile-last-batch-results)
                           '(delete)))
            (should-not (string-match-p "push one" (buffer-string)))))
      (kill-buffer buffer))))

(ert-deftest jaunder-reconcile-report-mode-binds-refresh-and-explicit-transfer-commands ()
  "Report mode reserves `g' for refresh and `f' for selected fetch."
  (let (bindings)
    (map-keymap (lambda (_event binding) (push binding bindings))
                jaunder-reconcile-report-mode-map)
    (should (eq (lookup-key jaunder-reconcile-report-mode-map (kbd "g"))
                #'jaunder-reconcile-refresh))
    (should (eq (lookup-key jaunder-reconcile-report-mode-map (kbd "f"))
                #'jaunder-reconcile-pull-selected))
    (should (memq #'jaunder-reconcile-toggle-mark bindings))
    (should (memq #'jaunder-reconcile-push-selected bindings))
    (should (memq #'jaunder-reconcile-delete-selected bindings))
    (should-not (memq #'jaunder--pull-member bindings))))

(ert-deftest jaunder-reconcile-refresh-reclassifies-from-a-fresh-inventory ()
  "Refresh rebuilds visible classifications rather than repainting stale rows."
  (let* ((old-row (jaunder--make-reconcile-row :state 'local-draft :key "post:7"))
         (old-report (jaunder--make-reconcile-report :root "/tmp" :rows (list old-row)))
         (member (jaunder-reconcile-test--member "7" "fresh"))
         (inventory (jaunder--make-inventory :server-only (list member)))
         (buffer (jaunder--render-reconcile-report old-report)))
    (unwind-protect
        (with-current-buffer buffer
          (cl-letf (((symbol-function 'jaunder--call-with-blog) (lambda (_ thunk) (funcall thunk)))
                    ((symbol-function 'jaunder--inventory-for-root) (lambda (_) inventory)))
            (call-interactively (key-binding (kbd "g")))
            (should (eq (jaunder-reconcile-row-state
                         (car (jaunder-reconcile-report-rows jaunder-reconcile-report)))
                        'server-only))
            (should (string-match-p "server-only (1)" (buffer-string)))))
      (kill-buffer buffer))))

(ert-deftest jaunder-reconcile-refresh-restores-state-or-falls-back-at-buffer-start ()
  "Refresh preserves, clamps, or falls back for point while retaining report state."
  (let* ((old-row (jaunder--make-reconcile-row :state 'server-only :key "post:7"
                                               :member (jaunder-reconcile-test--member
                                                        "7" "界-longer-slug")))
         (old-report (jaunder--make-reconcile-report :root "/tmp" :rows (list old-row)))
         (same-row (jaunder--make-inventory
                    :server-only
                    (list (jaunder-reconcile-test--member "7" "界-longer-slug"))))
         (shorter-row (jaunder--make-inventory
                       :server-only (list (jaunder-reconcile-test--member "7" "short"))))
         (missing (jaunder--make-inventory
                   :server-only (list (jaunder-reconcile-test--member "8" "other"))))
         (first-result (jaunder--make-reconcile-result :action 'pull :row-key "post:7"
                                                       :outcome 'success))
         (second-result (jaunder--make-reconcile-result :action 'delete :row-key "post:8"
                                                        :outcome 'blocked))
         (results (list first-result second-result))
         (buffer (jaunder--render-reconcile-report old-report)))
    (unwind-protect
        (with-current-buffer buffer
          (puthash "post:7" t jaunder-reconcile-marks)
          (puthash "post:gone" t jaunder-reconcile-marks)
          (setq-local jaunder-reconcile-last-batch-results results)
          (goto-char (jaunder--reconcile-row-key-position "post:7"))
          (forward-char 3)
          (cl-letf (((symbol-function 'jaunder--call-with-blog) (lambda (_ thunk) (funcall thunk)))
                    ((symbol-function 'jaunder--inventory-for-root) (lambda (_) same-row)))
            (call-interactively (key-binding (kbd "g")))
            (should (= (current-column) 4)))
          (goto-char (jaunder--reconcile-row-key-position "post:7"))
          (end-of-line)
          (let ((original-column (current-column)))
            (cl-letf (((symbol-function 'jaunder--call-with-blog) (lambda (_ thunk) (funcall thunk)))
                      ((symbol-function 'jaunder--inventory-for-root) (lambda (_) shorter-row)))
              (call-interactively (key-binding (kbd "g")))
              (should (< (current-column) original-column))
              (should (= (current-column) (save-excursion
                                            (end-of-line)
                                            (current-column))))
              (should (gethash "post:7" jaunder-reconcile-marks))
              (should-not (gethash "post:gone" jaunder-reconcile-marks))
              (should (equal jaunder-reconcile-last-batch-results results))
              (should (string-match-p "Last batch" (buffer-string)))
              (let ((first-position (string-match "- pull post:7: success" (buffer-string)))
                    (second-position (string-match "- delete post:8: blocked" (buffer-string))))
                (should first-position)
                (should second-position)
                (should (< first-position second-position)))))
          (cl-letf (((symbol-function 'jaunder--call-with-blog) (lambda (_ thunk) (funcall thunk)))
                    ((symbol-function 'jaunder--inventory-for-root) (lambda (_) missing)))
            (call-interactively (key-binding (kbd "g")))
            (should (= (point) (point-min)))))
      (kill-buffer buffer))))

(ert-deftest jaunder-reconcile-refresh-renders-row-local-member-failure ()
  "Refresh keeps a Member fetch failure as a visible unclassifiable row."
  (let* ((local (jaunder-reconcile-test--local "/tmp/matched.org" "7"))
         (member (jaunder-reconcile-test--member "7" "remote"))
         (inventory (jaunder--make-inventory
                     :matched (list (jaunder--make-inventory-match
                                     :local local :member member))))
         (buffer (jaunder--render-reconcile-report
                  (jaunder--make-reconcile-report :root "/tmp" :rows nil))))
    (unwind-protect
        (with-current-buffer buffer
          (cl-letf (((symbol-function 'jaunder--call-with-blog) (lambda (_ thunk) (funcall thunk)))
                    ((symbol-function 'jaunder--inventory-for-root) (lambda (_) inventory))
                    ((symbol-function 'jaunder--http-request)
                     (lambda (&rest _) (list :status 503 :headers nil))))
            (call-interactively (key-binding (kbd "g")))
            (should (string-match-p "unclassifiable (1)" (buffer-string)))
            (should (string-match-p "member-http-error (503)" (buffer-string)))))
      (kill-buffer buffer))))

(ert-deftest jaunder-reconcile-refresh-failure-preserves-the-existing-report ()
  "Refresh re-signals inventory failure without replacing usable report state."
  (let* ((row (jaunder--make-reconcile-row :state 'server-only :key "post:7"))
         (report (jaunder--make-reconcile-report :root "/tmp" :rows (list row)))
         (result (jaunder--make-reconcile-result :action 'pull :row-key "post:7"
                                                 :outcome 'success))
         (buffer (jaunder--render-reconcile-report report)))
    (unwind-protect
        (with-current-buffer buffer
          (puthash "post:7" t jaunder-reconcile-marks)
          (setq-local jaunder-reconcile-last-batch-results (list result))
          (jaunder--render-reconcile-report report buffer)
          (goto-char (jaunder--reconcile-row-key-position "post:7"))
          (let ((text (buffer-string)) (point (point)) (marks jaunder-reconcile-marks))
            (cl-letf (((symbol-function 'jaunder--call-with-blog) (lambda (_ thunk) (funcall thunk)))
                      ((symbol-function 'jaunder--inventory-for-root)
                       (lambda (_) (error "offline"))))
              (let ((error (should-error
                            (call-interactively (key-binding (kbd "g")))
                            :type 'error)))
                (should (equal (error-message-string error) "offline"))))
            (should (equal (buffer-string) text))
            (should (eq jaunder-reconcile-report report))
            (should (= (point) point))
            (should (eq jaunder-reconcile-marks marks))
            (should (gethash "post:7" jaunder-reconcile-marks))
            (should (equal jaunder-reconcile-last-batch-results (list result)))))
      (kill-buffer buffer))))

(ert-deftest jaunder-reconcile-refresh-render-failure-preserves-the-existing-report ()
  "Refresh rolls back a render-stage failure after it has replaced live content."
  (let* ((row (jaunder--make-reconcile-row :state 'server-only :key "post:7"))
         (report (jaunder--make-reconcile-report :root "/tmp" :rows (list row)))
         (result (jaunder--make-reconcile-result :action 'pull :row-key "post:7"
                                                 :outcome 'success))
         (inventory (jaunder--make-inventory
                     :server-only (list (jaunder-reconcile-test--member "8" "fresh"))))
         (buffer (jaunder--render-reconcile-report report)))
    (unwind-protect
        (with-current-buffer buffer
          (puthash "post:7" t jaunder-reconcile-marks)
          (setq-local jaunder-reconcile-last-batch-results (list result))
          (jaunder--render-reconcile-report report buffer)
          (goto-char (jaunder--reconcile-row-key-position "post:7"))
          (let* ((text (buffer-string))
                 (point (point))
                 (marks jaunder-reconcile-marks)
                 (row-property (get-text-property point 'jaunder-reconcile-row)))
            (cl-letf (((symbol-function 'jaunder--call-with-blog)
                       (lambda (_ thunk) (funcall thunk)))
                      ((symbol-function 'jaunder--inventory-for-root) (lambda (_) inventory))
                      ((symbol-function 'jaunder--reconcile-row-label)
                       (lambda (_) (error "render offline"))))
              (let ((error (should-error
                            (call-interactively (key-binding (kbd "g")))
                            :type 'error)))
                (should (equal (error-message-string error) "render offline"))))
            (should (equal (buffer-string) text))
            (should (eq jaunder-reconcile-report report))
            (should (= (point) point))
            (should (eq jaunder-reconcile-marks marks))
            (should (gethash "post:7" jaunder-reconcile-marks))
            (should (equal jaunder-reconcile-last-batch-results (list result)))
            (should (eq (get-text-property point 'jaunder-reconcile-row) row-property))))
      (kill-buffer buffer))))

(ert-deftest jaunder-reconcile-push-state-matrix-delegates-only-safe-rows ()
  "Push accepts drafts/local-ahead, no-ops unchanged, and blocks every other state."
  (dolist (state '(local-draft local-ahead unchanged server-ahead conflict
                               unclassifiable orphan server-only inventory-conflict))
    (let* ((path (make-temp-file "jaunder-reconcile-push-" nil ".org"))
           (local (jaunder-reconcile-test--local path "7"))
           (member (jaunder-reconcile-test--member "7" "post"))
           (row (jaunder--make-reconcile-row :state state :key (symbol-name state)
                                             :local local :member member))
           published)
      (unwind-protect
          (progn
            (when (eq state 'local-ahead)
              (with-temp-file path (insert "#+PROPERTY: JAUNDER_ID 7\n")))
            (cl-letf (((symbol-function 'jaunder-publish)
                       (lambda () (setq published t)))
                      ((symbol-function 'jaunder--reconcile-local-mutation-safety-reason)
                       (lambda (_) nil))
                      ((symbol-function 'jaunder--buffer-property)
                       (lambda (name)
                         (cdr (assoc name '(("JAUNDER_ID" . "7")
                                            ("JAUNDER_SLUG" . "post")
                                            ("JAUNDER_SYNCED" . "\"etag\"")
                                            ("JAUNDER_SYNCED_AT" . "2026-09-17T00:00:00Z")))))))
              (let ((result (jaunder--reconcile-push-row row)))
                (should (eq (not (null published))
                            (not (null (memq state '(local-draft local-ahead))))))
                (should (eq (plist-get result :outcome)
                            (cond ((eq state 'unchanged) 'no-op)
                                  ((memq state '(local-draft local-ahead)) 'success)
                                  (t 'blocked))))
                (when (memq state '(local-draft local-ahead))
                  (should-not (get-file-buffer path)))))
            (delete-file path))))))

(ert-deftest jaunder-reconcile-push-preserves-a-preexisting-source-buffer ()
  (let* ((path (make-temp-file "jaunder-reconcile-open-" nil ".org"))
         (buffer (find-file-noselect path))
         (row (jaunder--make-reconcile-row
               :state 'local-draft :key "draft"
               :local (jaunder-reconcile-test--local path nil))))
    (unwind-protect
        (cl-letf (((symbol-function 'jaunder-publish) (lambda () nil))
                  ((symbol-function 'jaunder--reconcile-local-mutation-safety-reason)
                   (lambda (_) nil))
                  ((symbol-function 'jaunder--buffer-property) (lambda (_) nil)))
          (jaunder--reconcile-push-row row)
          (should (buffer-live-p buffer))
          (should (eq (get-file-buffer path) buffer)))
      (when (buffer-live-p buffer) (kill-buffer buffer))
      (delete-file path))))

(ert-deftest jaunder-reconcile-push-reports-a-canonical-source-rename ()
  (let* ((root (make-temp-file "jaunder-reconcile-rename-" t))
         (path (expand-file-name "65.org" root))
         (destination (expand-file-name "canonical-post.org" root))
         (row (jaunder--make-reconcile-row
               :state 'local-draft :key "draft"
               :local (jaunder-reconcile-test--local path nil))))
    (unwind-protect
        (progn
          (write-region "Body\n" nil path nil 'silent)
          (cl-letf (((symbol-function 'jaunder-publish)
                     (lambda ()
                       (jaunder--rename-to-slug "canonical-post")
                       '(:http-status 201)))
                    ((symbol-function 'jaunder--reconcile-local-mutation-safety-reason)
                     (lambda (_) nil))
                    ((symbol-function 'jaunder--buffer-property)
                     (lambda (key)
                       (cdr (assoc key '(("JAUNDER_ID" . "7")
                                         ("JAUNDER_SLUG" . "canonical-post")
                                         ("JAUNDER_SYNCED" . "\"etag\"")
                                         ("JAUNDER_SYNCED_AT" . "2026-09-17T00:00:00Z")))))))
            (let ((result (jaunder--reconcile-push-row row)))
              (should (equal (plist-get result :detail)
                             (format "renamed %s -> %s" path destination)))
              (should-not (get-file-buffer destination))
              (should-not (file-exists-p path))
              (should (file-exists-p destination)))))
      (delete-directory root t))))

(ert-deftest jaunder-reconcile-selected-commands-preview-once-and-never-delete-implicitly ()
  "Push and delete each prompt once; only delete's confirmed executor can DELETE."
  (let* ((row (jaunder--make-reconcile-row :state 'server-only :key "post:7"
                                           :member (jaunder-reconcile-test--member "7" "gone")))
         (buffer (jaunder--render-reconcile-report
                  (jaunder--make-reconcile-report :root "/tmp" :rows (list row))))
         prompts executions deletes)
    (unwind-protect
        (with-current-buffer buffer
          (puthash "post:7" t jaunder-reconcile-marks)
          (cl-letf (((symbol-function 'y-or-n-p)
                     (lambda (prompt) (push prompt prompts) nil))
                    ((symbol-function 'jaunder--http-request)
                     (lambda (method &rest _)
                       (when (equal method "DELETE") (setq deletes (1+ (or deletes 0))))
                       '(:status 200 :headers (("etag" . "\"fresh\"")))))
                    ((symbol-function 'jaunder--reconcile-execute-batch)
                     (lambda (&rest _) (setq executions (1+ (or executions 0)))))
                    ((symbol-function 'jaunder--call-with-blog)
                     (lambda (_root thunk) (funcall thunk))))
            (jaunder-reconcile-push-selected)
            (jaunder-reconcile-delete-selected)
            (should (= (length prompts) 2))
            (should (string-match-p "Push 1 selected" (car (last prompts))))
            (should (string-match-p "SOFT-DELETE 1 selected" (car prompts)))
            (should (string-match-p "ETag=\"fresh\"" (car prompts)))
            (should-not executions)
            (should-not deletes)))
      (kill-buffer buffer))))

(ert-deftest jaunder-reconcile-delete-state-matrix-and-etag-guard ()
  "Delete permits only the four unambiguous remote states and sends reviewed ETag."
  (dolist (state '(server-only unchanged local-ahead server-ahead local-draft conflict
                               unclassifiable orphan inventory-conflict))
    (let* ((member (jaunder-reconcile-test--member "7" "gone"))
           (row (jaunder--make-reconcile-row :state state :key (symbol-name state)
                                             :member member))
           methods headers)
      (let ((jaunder--active-blog '(:base-url "https://example.test" :username "alice")))
        (cl-letf (((symbol-function 'jaunder--http-request)
                   (lambda (method _url &rest args)
                     (push method methods)
                     (when (equal method "DELETE") (setq headers (nth 2 args)))
                     (if (equal method "GET")
                         '(:status 200 :headers (("etag" . "\"fresh\"")))
                       '(:status 204)))))
          (let ((review (if (memq state '(server-only unchanged local-ahead server-ahead))
                            (jaunder--reconcile-delete-etag row)
                          (jaunder--reconcile-blocked row 'delete-ineligible state))))
            (let ((result (jaunder--reconcile-delete-row row review)))
              (should (eq (plist-get result :outcome)
                          (if (memq state '(server-only unchanged local-ahead server-ahead))
                              'success 'blocked)))
              (should (equal methods (if (memq state '(server-only unchanged local-ahead server-ahead))
                                         '("DELETE" "GET") nil)))
              (when headers
                (should (equal headers '(("If-Match" . "\"fresh\""))))))))))))

(ert-deftest jaunder-reconcile-delete-stale-etag-preserves-matched-local-file ()
  "A 412 retains the reviewed identity and leaves the matched file untouched."
  (let* ((path (make-temp-file "jaunder-reconcile-delete-" nil ".org"))
         (local (jaunder-reconcile-test--local path "7"))
         (member (jaunder-reconcile-test--member "7" "gone"))
         (row (jaunder--make-reconcile-row :state 'unchanged :key "post:7"
                                           :local local :member member))
         (reviewed '(:post-id "7" :slug "gone" :etag "\"fresh\"" :http-status 200))
         (jaunder--active-blog '(:base-url "https://example.test" :username "alice")))
    (unwind-protect
        (cl-letf (((symbol-function 'jaunder--http-request)
                   (lambda (&rest _) '(:status 412)))
                  ((symbol-function 'jaunder--reconcile-delete-preflight)
                   (lambda (_) nil)))
          (let ((result (jaunder--reconcile-delete-row row reviewed)))
            (should (eq (plist-get result :outcome) 'failed))
            (should (eq (plist-get result :reason) 'etag-stale))
            (should (equal (jaunder-reconcile-result-post-id
                            (jaunder--reconcile-terminal-result 'delete row result))
                           "7"))
            (should (equal (plist-get result :etag) "\"fresh\""))
            (should (= (plist-get result :http-status) 412))
            (should (eq (plist-get result :local-effect) 'unchanged))
            (should (file-exists-p path))))
      (delete-file path))))

(ert-deftest jaunder-reconcile-delete-preflight-blocks-modified-buffer-and-preserves-it ()
  "A modified visiting buffer blocks deletion before any transport mutation."
  (let* ((path (make-temp-file "jaunder-reconcile-modified-" nil ".org"))
         (row (jaunder--make-reconcile-row
               :state 'unchanged :key "post:7"
               :local (jaunder-reconcile-test--local path "7")
               :member (jaunder-reconcile-test--member "7" "gone")))
         (buffer (find-file-noselect path)))
    (unwind-protect
        (with-current-buffer buffer
          (insert "edited")
          (cl-letf (((symbol-function 'jaunder--reconcile-current-local-id)
                     (lambda (_) "7")))
            (should (eq (jaunder--reconcile-delete-preflight row) 'local-buffer-modified)))
          (should (buffer-modified-p buffer)))
      (when (buffer-live-p buffer) (kill-buffer buffer))
      (delete-file path))))

(ert-deftest jaunder-reconcile-delete-transport-error-keeps-reviewed-metadata ()
  "A DELETE transport error retains review identity, ETag, and local state."
  (let ((row (jaunder--make-reconcile-row :state 'server-only :key "post:7"
                                          :member (jaunder-reconcile-test--member "7" "gone")))
        (jaunder--active-blog '(:base-url "https://example.test" :username "alice")))
    (cl-letf (((symbol-function 'jaunder--http-request) (lambda (&rest _) (error "offline"))))
      (let ((result (jaunder--reconcile-delete-row
                     row '(:post-id "7" :slug "gone" :etag "\"fresh\""))))
        (should (eq (plist-get result :reason) 'delete-transport-error))
        (should (equal (plist-get result :post-id) "7"))
        (should (equal (plist-get result :slug) "gone"))
        (should (equal (plist-get result :etag) "\"fresh\""))
        (should (eq (plist-get result :local-effect) 'unchanged))))))

(ert-deftest jaunder-reconcile-selected-operations-recheck-confirmation-time-identity-before-mutation ()
  "A clean identity change after review blocks selected push and delete I/O."
  (let* ((root (file-name-as-directory (make-temp-file "jaunder-reconcile-confirm-" t)))
         (push-path (expand-file-name "draft.org" root))
         (delete-path (expand-file-name "matched.org" root))
         (jaunder-blogs (list (cons root '(:base-url "https://example.test" :username "alice"))))
         push-buffer delete-buffer methods published)
    (unwind-protect
        (progn
          (with-temp-file push-path
            (insert "#+TITLE: Draft\n#+PROPERTY: JAUNDER_STATUS published\n\nBody.\n"))
          (with-temp-file delete-path
            (insert (concat "#+TITLE: Matched\n#+PROPERTY: JAUNDER_STATUS published\n"
                            "#+PROPERTY: JAUNDER_ID 7\n\nBody.\n")))
          (setq push-buffer (find-file-noselect push-path)
                delete-buffer (find-file-noselect delete-path))
          (let ((push-row (jaunder--make-reconcile-row
                           :state 'local-draft :key "local:draft"
                           :local (jaunder-reconcile-test--local push-path nil)))
                (delete-row (jaunder--make-reconcile-row
                             :state 'unchanged :key "post:7"
                             :local (jaunder-reconcile-test--local delete-path "7")
                             :member (jaunder-reconcile-test--member "7" "matched"))))
            (cl-letf (((symbol-function 'jaunder--reconcile-refresh-buffer)
                       (lambda (&rest _) nil))
                      ((symbol-function 'jaunder-publish)
                       (lambda () (setq published t)))
                      ((symbol-function 'jaunder--http-request)
                       (lambda (method &rest _)
                         (push method methods)
                         (if (equal method "GET")
                             '(:status 200 :headers (("etag" . "\"fresh\"")))
                           (error "unexpected remote mutation: %s" method)))))
              (let ((buffer (jaunder--render-reconcile-report
                             (jaunder--make-reconcile-report :root root :rows (list push-row)))))
                (unwind-protect
                    (with-current-buffer buffer
                      (puthash "local:draft" t jaunder-reconcile-marks)
                      (cl-letf (((symbol-function 'y-or-n-p)
                                 (lambda (_)
                                   (with-current-buffer push-buffer
                                     (jaunder--set-property "JAUNDER_ID" "8")
                                     (save-buffer))
                                   t)))
                        (jaunder-reconcile-push-selected))
                      (let ((result (car jaunder-reconcile-last-batch-results)))
                        (should (eq (jaunder-reconcile-result-outcome result) 'blocked))
                        (should (eq (jaunder-reconcile-result-reason result)
                                    'draft-identity-changed))))
                  (kill-buffer buffer)))
              (let ((buffer (jaunder--render-reconcile-report
                             (jaunder--make-reconcile-report :root root :rows (list delete-row)))))
                (unwind-protect
                    (with-current-buffer buffer
                      (puthash "post:7" t jaunder-reconcile-marks)
                      (cl-letf (((symbol-function 'y-or-n-p)
                                 (lambda (_)
                                   (with-current-buffer delete-buffer
                                     (jaunder--set-property "JAUNDER_ID" "8")
                                     (save-buffer))
                                   t)))
                        (jaunder-reconcile-delete-selected))
                      (let ((result (car jaunder-reconcile-last-batch-results)))
                        (should (eq (jaunder-reconcile-result-outcome result) 'blocked))
                        (should (eq (jaunder-reconcile-result-reason result)
                                    'matched-identity-changed))))
                  (kill-buffer buffer)))))
          (should-not published)
          (should (equal methods '("GET"))))
      (dolist (buffer (list push-buffer delete-buffer))
        (when (buffer-live-p buffer)
          (with-current-buffer buffer (set-buffer-modified-p nil))
          (kill-buffer buffer)))
      (delete-directory root t))))

(defun jaunder-reconcile-test--pulled-bytes (id slug etag)
  "Return minimal pulled Org bytes for ID, SLUG, and strong ETAG."
  (format (concat "#+PROPERTY: JAUNDER_STATUS draft\n"
                  "#+PROPERTY: JAUNDER_FORMAT org\n"
                  "#+PROPERTY: JAUNDER_SLUG %s\n"
                  "#+PROPERTY: JAUNDER_ID %s\n"
                  "#+PROPERTY: JAUNDER_SYNCED %s\n"
                  "#+PROPERTY: JAUNDER_SYNCED_AT 2026-09-17T00:00:00Z\n\nBody\n")
          slug id etag))

(defun jaunder-reconcile-test--matched-pull-row (path slug &optional state)
  "Return a reviewed matched pull row for PATH, SLUG, and optional STATE."
  (let ((id "7"))
    (jaunder--make-reconcile-row
     :state (or state 'server-ahead) :key (format "post:%s" id)
     :local (jaunder-reconcile-test--local path id)
     :member (jaunder-reconcile-test--member id slug)
     :local-sha256 (jaunder--reconcile-file-sha256 path) :remote-etag "\"old\"")))

(ert-deftest jaunder-reconcile-pull-state-matrix-returns-complete-task-two-results ()
  "Pull accepts exactly server-only/server-ahead, no-ops unchanged, and blocks six states."
  (dolist (state '(server-only server-ahead unchanged local-ahead conflict
                               unclassifiable orphan local-draft inventory-conflict))
    (let* ((member (jaunder-reconcile-test--member "7" "remote"))
           (row (jaunder--make-reconcile-row :state state :key (symbol-name state)
                                             :member member))
           (jaunder-reconcile-report (jaunder--make-reconcile-report :root "/tmp"))
           called)
      (cl-letf (((symbol-function 'jaunder--pull-member)
                 (lambda (&rest _) (setq called 'server-only)
                   (jaunder--make-pull-result
                    :status 'pulled :id "7" :slug "remote" :etag "\"current\""
                    :synced-at "2026-09-17T00:00:00Z" :http-status 200
                    :local-effect 'created)))
                ((symbol-function 'jaunder--reconcile-pull-server-ahead-row)
                 (lambda (_) (setq called 'server-ahead)
                   (list :outcome 'success :post-id "7" :slug "remote"
                         :etag "\"current\"" :synced-at "2026-09-17T00:00:00Z"
                         :http-status 200 :local-effect 'replaced))))
        (let* ((value (jaunder--reconcile-pull-row row))
               (result (jaunder--reconcile-terminal-result 'pull row value)))
          (should (eq (jaunder-reconcile-result-outcome result)
                      (cond ((eq state 'unchanged) 'no-op)
                            ((memq state '(server-only server-ahead)) 'success)
                            (t 'blocked))))
          (should (equal (jaunder-reconcile-result-post-id result) "7"))
          (should (equal (jaunder-reconcile-result-slug result) "remote"))
          (should (eq (jaunder-reconcile-result-local-effect result)
                      (if (eq state 'server-only) 'created
                        (if (memq state '(server-ahead unchanged))
                            (if (eq state 'server-ahead) 'replaced 'unchanged)
                          'unchanged))))
          (when (memq state '(server-only server-ahead))
            (should (equal (jaunder-reconcile-result-etag result) "\"current\""))
            (should (equal (jaunder-reconcile-result-synced-at result)
                           "2026-09-17T00:00:00Z"))
            (should (= (jaunder-reconcile-result-http-status result) 200)))
          (should (eq called (and (memq state '(server-only server-ahead)) state))))))))

(ert-deftest jaunder-reconcile-pull-selected-confirms-the-full-count-once ()
  "Selected pull previews both operations and prompts exactly once before executing."
  (let* ((rows (list (jaunder--make-reconcile-row :state 'server-only :key "post:1"
                                                  :member (jaunder-reconcile-test--member "1" "one"))
                     (jaunder--make-reconcile-row :state 'unchanged :key "post:2"
                                                  :member (jaunder-reconcile-test--member "2" "two"))))
         (buffer (jaunder--render-reconcile-report
                  (jaunder--make-reconcile-report :root "/tmp" :rows rows)))
         prompts executed)
    (unwind-protect
        (with-current-buffer buffer
          (dolist (row rows) (puthash (jaunder-reconcile-row-key row) t jaunder-reconcile-marks))
          (cl-letf (((symbol-function 'y-or-n-p)
                     (lambda (prompt) (push prompt prompts) t))
                    ((symbol-function 'jaunder--call-with-blog)
                     (lambda (_root thunk) (funcall thunk)))
                    ((symbol-function 'jaunder--reconcile-execute-batch)
                     (lambda (_buffer selected action _operation)
                       (setq executed (list selected action)))))
            (jaunder-reconcile-pull-selected)
            (should (equal prompts '("Pull 2 selected Post(s)? ")))
            (should (equal (mapcar #'jaunder-reconcile-row-key (car executed))
                           '("post:2" "post:1")))
            (should (eq (cadr executed) 'pull))))
      (kill-buffer buffer))))

(ert-deftest jaunder-reconcile-matched-pull-restores-proven-local-post-link ()
  "The complete matched server-ahead path installs a reversed canonical link."
  (let* ((root (file-name-as-directory (make-temp-file "jaunder-pull-link-" t)))
         (source-path (expand-file-name "source.org" root))
         (target-path (expand-file-name "target.org" root))
         (source-bytes (jaunder-reconcile-test--pulled-bytes "7" "source" "\"old\""))
         (source-local (jaunder-reconcile-test--local source-path "7"))
         (target-local (jaunder-reconcile-test--local target-path "8"))
         (source-member (jaunder-reconcile-test--member "7" "source"))
         (target-member (jaunder--make-inventory-member
                         :id "8" :slug "target"
                         :edit-uri "https://example.test/atompub/alice/posts/8"
                         :alternate-href "https://example.test/@alice/target"))
         row calls
         (jaunder--active-blog '(:base-url "https://example.test" :username "alice")))
    (unwind-protect
        (progn
          (with-temp-file source-path (insert source-bytes))
          (with-temp-file target-path
            (insert "#+PROPERTY: JAUNDER_ID 8\n#+PROPERTY: JAUNDER_SLUG target\n\nTarget"))
          (setq row (jaunder--make-reconcile-row
                     :state 'server-ahead :key "post:7" :local source-local
                     :member source-member
                     :local-sha256 (jaunder--reconcile-file-sha256 source-path)
                     :remote-etag "\"old\""))
          (let* ((inventory (jaunder--join-inventory
                             (list source-local target-local)
                             (list source-member target-member)))
                 (jaunder-reconcile-report
                  (jaunder--make-reconcile-report :root root :inventory inventory)))
            (cl-letf (((symbol-function 'jaunder--fetch-service-document)
                       #'jaunder-reconcile-test--legacy-service-document)
                      ((symbol-function 'jaunder--http-request)
                       (lambda (&rest _)
                         (push 'get calls)
                         (if (cdr calls)
                             '(:status 200 :headers (("etag" . "\"old\"")))
                           (list :status 200
                                 :headers '(("etag" . "\"old\"")
                                            ("x-jaunder-instance" .
                                             "12345678-1234-1234-1234-123456789abc"))
                                 :body (concat
                                        "<entry xmlns=\"http://www.w3.org/2005/Atom\""
                                        " xmlns:app=\"http://www.w3.org/2007/app\""
                                        " xmlns:j=\"https://jaunder.org/ns/atompub\">"
                                        "<title>Source</title>"
                                        "<link rel=\"edit\" href=\"https://example.test/atompub/alice/posts/7\"/>"
                                        "<j:slug>source</j:slug>"
                                        "<content type=\"text/org\">[[https://example.test/@alice/target][Target]]</content>"
                                        "<app:control><app:draft>yes</app:draft></app:control>"
                                        "</entry>")))))
                      ((symbol-function 'jaunder--reconcile-pull-unique-match)
                       (lambda (&rest _) '(:ok t))))
              (should (eq (plist-get (jaunder--reconcile-pull-server-ahead-row row)
                                     :outcome)
                          'success))))
          (should (= (length calls) 2))
          (should (string-match-p
                   (regexp-quote "[[./target.org][Target]]")
                   (with-temp-buffer
                     (insert-file-contents source-path)
                     (buffer-string)))))
      (delete-directory root t))))

(ert-deftest jaunder-reconcile-legacy-refresh-preserves-local-audience-lines ()
  (let ((path (make-temp-file "jaunder-legacy-audience-" nil ".org"))
        (staged (concat "#+TITLE: Remote\n#+PROPERTY: JAUNDER_STATUS published\n"
                        "#+PROPERTY: JAUNDER_ID 42\n\nRemote body.\n")))
    (unwind-protect
        (progn
          (with-temp-file path
            (insert (concat "#+TITLE: Local\n"
                            "#+PROPERTY: JAUNDER_AUDIENCE named:17\n"
                            "#+PROPERTY: JAUNDER_ID 42\n"
                            "#+PROPERTY: JAUNDER_AUDIENCE public\n\n"
                            "Local body.\n#+PROPERTY: JAUNDER_AUDIENCE body-text\n")))
          (should
           (equal (jaunder--reconcile-preserve-legacy-audience path staged)
                  (concat "#+TITLE: Remote\n#+PROPERTY: JAUNDER_STATUS published\n"
                          "#+PROPERTY: JAUNDER_AUDIENCE named:17\n"
                          "#+PROPERTY: JAUNDER_AUDIENCE public\n"
                          "#+PROPERTY: JAUNDER_ID 42\n\nRemote body.\n")))
          (should-error
           (jaunder--reconcile-preserve-legacy-audience
            path "#+TITLE: Invalid staged Post\n\nRemote body.\n")))
      (should (string-match-p
               "Local body"
               (with-temp-buffer (insert-file-contents path) (buffer-string))))
      (delete-file path))))

(ert-deftest jaunder-reconcile-legacy-staged-install-retains-audience ()
  (let* ((path (make-temp-file "jaunder-legacy-install-" nil ".org"))
         (staged (list :slug "remote" :id "42" :etag "\"remote\""
                       :audience-omitted t :synced-at "2026-08-25T00:00:00Z"
                       :bytes (concat "#+TITLE: Remote\n"
                                      "#+PROPERTY: JAUNDER_STATUS published\n"
                                      "#+PROPERTY: JAUNDER_ID 42\n\nRemote body.\n"))))
    (unwind-protect
        (progn
          (with-temp-file path
            (insert (concat "#+TITLE: Local\n"
                            "#+PROPERTY: JAUNDER_AUDIENCE subscribers\n"
                            "#+PROPERTY: JAUNDER_ID 42\n\nLocal body.\n")))
          (cl-letf (((symbol-function 'jaunder--reconcile-pull-preflight)
                     (lambda (&rest _) nil))
                    ((symbol-function 'jaunder--reconcile-pull-destination)
                     (lambda (&rest _) path)))
            (should (eq (plist-get
                         (jaunder--reconcile-pull-install-staged
                          nil staged '(:http-status 200) path)
                         :outcome) 'success)))
          (should (equal (with-temp-buffer (insert-file-contents path) (buffer-string))
                         (concat "#+TITLE: Remote\n"
                                 "#+PROPERTY: JAUNDER_STATUS published\n"
                                 "#+PROPERTY: JAUNDER_AUDIENCE subscribers\n"
                                 "#+PROPERTY: JAUNDER_ID 42\n\nRemote body.\n"))))
      (delete-file path))))

(ert-deftest jaunder-reconcile-pull-stale-etag-blocks-before-local-replacement ()
  "A changed staged ETag leaves the reviewed matched file untouched."
  (let* ((root (file-name-as-directory (make-temp-file "jaunder-pull-stale-" t)))
         (path (expand-file-name "old.org" root))
         (before (jaunder-reconcile-test--pulled-bytes "7" "old" "\"old\""))
         (row (progn (with-temp-file path (insert before))
                     (jaunder-reconcile-test--matched-pull-row path "old")))
         (jaunder-reconcile-report (jaunder--make-reconcile-report :root root)))
    (unwind-protect
        (cl-letf (((symbol-function 'jaunder--pull-stage-member)
                   (lambda (&rest _) (list :etag "\"new\"" :id "7" :slug "old"
                                           :bytes "replacement")))
                  ((symbol-function 'jaunder--reconcile-pull-unique-match)
                   (lambda (&rest _) '(:ok t)))
                  ((symbol-function 'jaunder--reconcile-pull-remote-revalidation)
                   (lambda (&rest _) '(:ok t :etag "\"old\"" :http-status 200)))
                  ((symbol-function 'jaunder--reconcile-replace-pulled-file)
                   (lambda (&rest _) (error "must not replace"))))
          (let ((result (jaunder--reconcile-pull-server-ahead-row row)))
            (should (eq (plist-get result :outcome) 'blocked))
            (should (eq (plist-get result :reason) 'etag-stale))
            (should (equal (with-temp-buffer (insert-file-contents-literally path) (buffer-string))
                           before))))
      (delete-directory root t))))

(ert-deftest jaunder-reconcile-pull-preflight-blocks-digest-id-and-modified-buffer-races ()
  "Changed local bytes, identity, or a modified visited buffer blocks matched pull."
  (let* ((root (file-name-as-directory (make-temp-file "jaunder-pull-race-" t)))
         (path (expand-file-name "old.org" root))
         (bytes (jaunder-reconcile-test--pulled-bytes "7" "old" "\"old\""))
         row buffer
         (jaunder-reconcile-report (jaunder--make-reconcile-report :root root)))
    (unwind-protect
        (progn
          (with-temp-file path (insert bytes))
          (setq row (jaunder-reconcile-test--matched-pull-row path "old"))
          (with-temp-file path (insert "changed"))
          (should (eq (jaunder--reconcile-pull-preflight row '(:slug "old"))
                      'local-bytes-changed))
          (with-temp-file path (insert bytes))
          (setq row (jaunder-reconcile-test--matched-pull-row path "old"))
          (setq buffer (find-file-noselect path))
          (with-current-buffer buffer (insert "edited"))
          (should (eq (jaunder--reconcile-pull-preflight row '(:slug "old"))
                      'local-buffer-modified))
          (with-current-buffer buffer (set-buffer-modified-p nil))
          (with-temp-file path (insert (jaunder-reconcile-test--pulled-bytes "8" "old" "\"old\"")))
          (setf (jaunder-reconcile-row-local-sha256 row)
                (jaunder--reconcile-file-sha256 path))
          (should (eq (jaunder--reconcile-pull-preflight row '(:slug "old"))
                      'matched-identity-changed)))
      (when (buffer-live-p buffer) (kill-buffer buffer))
      (delete-directory root t))))

(ert-deftest jaunder-reconcile-pull-clean-buffer-renames-and-follows-installed-file ()
  "A displayed clean buffer follows a full matched pull's canonical rename."
  (let* ((root (file-name-as-directory (make-temp-file "jaunder-pull-rename-" t)))
         (path (expand-file-name "old.org" root))
         (destination (expand-file-name "new.org" root))
         (old (jaunder-reconcile-test--pulled-bytes "7" "old" "\"old\""))
         (new (jaunder-reconcile-test--pulled-bytes "7" "new" "\"old\""))
         (jaunder-reconcile-report (jaunder--make-reconcile-report :root root))
         row buffer window point window-start)
    (unwind-protect
        (progn
          (with-temp-file path (insert old))
          (setq row (jaunder-reconcile-test--matched-pull-row path "new")
                buffer (find-file-noselect path))
          (save-window-excursion
            (setq window (display-buffer buffer))
            (with-selected-window window
              (goto-char (point-max))
              (forward-line -1)
              (setq point (point))
              (set-window-start window (line-beginning-position) t)
              (setq window-start (window-start window)))
            (cl-letf (((symbol-function 'jaunder--pull-stage-member)
                       (lambda (&rest _) (list :id "7" :slug "new" :etag "\"old\""
                                               :synced-at "2026-09-17T00:00:00Z" :bytes new)))
                      ((symbol-function 'jaunder--reconcile-pull-unique-match)
                       (lambda (&rest _) '(:ok t)))
                      ((symbol-function 'jaunder--reconcile-pull-remote-revalidation)
                       (lambda (&rest _) '(:ok t :etag "\"old\"" :http-status 200))))
              (should (eq (plist-get (jaunder--reconcile-pull-server-ahead-row row) :outcome)
                          'success)))
            (should-not (file-exists-p path))
            (should (equal (buffer-file-name buffer) destination))
            (with-current-buffer buffer
              (should (equal (buffer-string) new))
              (should-not (buffer-modified-p))
              (should (= (point) point)))
            (should (= (window-start window) window-start))))
      (when (buffer-live-p buffer) (kill-buffer buffer))
      (delete-directory root t))))

(ert-deftest jaunder-reconcile-pull-destination-collision-and-between-step-retry-are-safe ()
  "An occupied canonical path blocks; a failed rename leaves one retryable ID file."
  (let* ((root (file-name-as-directory (make-temp-file "jaunder-pull-retry-" t)))
         (path (expand-file-name "old.org" root))
         (destination (expand-file-name "new.org" root))
         (old (jaunder-reconcile-test--pulled-bytes "7" "old" "\"old\""))
         (new (jaunder-reconcile-test--pulled-bytes "7" "new" "\"old\""))
         row (jaunder-reconcile-report (jaunder--make-reconcile-report :root root)))
    (unwind-protect
        (progn
          (with-temp-file path (insert old))
          (setq row (jaunder-reconcile-test--matched-pull-row path "new"))
          (with-temp-file destination (insert "other"))
          (should (eq (jaunder--reconcile-pull-preflight row '(:slug "new"))
                      'pull-destination-occupied))
          (delete-file destination)
          (let ((real-rename (symbol-function 'rename-file)) (moves 0))
            (cl-letf (((symbol-function 'rename-file)
                       (lambda (from to &optional ok)
                         (setq moves (1+ moves))
                         (if (= moves 2) (error "injected rename failure")
                           (funcall real-rename from to ok))))
                      ((symbol-function 'jaunder--pull-stage-member)
                       (lambda (&rest _) (list :etag "\"old\"" :id "7" :slug "new"
                                               :synced-at "2026-09-17T00:00:00Z" :bytes new)))
                      ((symbol-function 'jaunder--reconcile-pull-remote-revalidation)
                       (lambda (&rest _) '(:ok t :etag "\"old\"" :http-status 200)))
                      ((symbol-function 'jaunder--reconcile-pull-unique-match)
                       (lambda (&rest _) '(:ok t))))
              (let ((result (jaunder--reconcile-pull-server-ahead-row row)))
                (should (eq (plist-get result :outcome) 'failed))
                (should (eq (plist-get result :reason) 'pull-rename-failed))
                (should (eq (plist-get result :local-effect) 'replaced-at-old-path))
                (should (equal (plist-get result :post-id) "7"))
                (should (equal (plist-get result :slug) "new"))
                (should (equal (plist-get result :etag) "\"old\""))
                (should (equal (plist-get result :synced-at) "2026-09-17T00:00:00Z"))
                (should (= (plist-get result :http-status) 200)))))
          (should (file-exists-p path))
          (should-not (file-exists-p destination))
          (should (equal (jaunder--reconcile-current-local-id row) "7"))
          (should (= (length (directory-files root t "\\.org\\'")) 1))
          (setf (jaunder-reconcile-row-local-sha256 row)
                (jaunder--reconcile-file-sha256 path))
          (cl-letf (((symbol-function 'jaunder--pull-stage-member)
                     (lambda (&rest _) (list :etag "\"old\"" :id "7" :slug "new"
                                             :synced-at "2026-09-17T00:00:00Z" :bytes new)))
                    ((symbol-function 'jaunder--reconcile-pull-remote-revalidation)
                     (lambda (&rest _) '(:ok t :etag "\"old\"" :http-status 200)))
                    ((symbol-function 'jaunder--reconcile-pull-unique-match)
                     (lambda (&rest _) '(:ok t))))
            (should (eq (plist-get (jaunder--reconcile-pull-server-ahead-row row) :outcome)
                        'success)))
          (should-not (file-exists-p path))
          (should (file-exists-p destination))
          (should (= (length (directory-files root t "\\.org\\'")) 1)))
      (delete-directory root t))))

(ert-deftest jaunder-reconcile-pull-remote-revalidation-precedes-one-final-local-preflight ()
  "Remote revalidation races on disk, buffer, and destination block before replacement."
  (dolist (race '(bytes buffer destination))
    (let* ((root (file-name-as-directory (make-temp-file "jaunder-pull-order-" t)))
           (path (expand-file-name "old.org" root))
           (bytes (jaunder-reconcile-test--pulled-bytes "7" "old" "\"old\""))
           (row nil) calls
           (jaunder-reconcile-report (jaunder--make-reconcile-report :root root)))
      (unwind-protect
          (progn
            (with-temp-file path (insert bytes))
            (setq row (jaunder-reconcile-test--matched-pull-row path "new"))
            (let ((real-preflight (symbol-function 'jaunder--reconcile-pull-preflight)))
              (cl-letf (((symbol-function 'jaunder--pull-stage-member)
                         (lambda (&rest _) (push 'stage calls)
                           (list :etag "\"old\"" :id "7" :slug "new"
                                 :synced-at "2026-09-17T00:00:00Z" :bytes bytes)))
                        ((symbol-function 'jaunder--reconcile-pull-remote-revalidation)
                         (lambda (&rest _)
                           (push 'remote-revalidation calls)
                           (pcase race
                             ('bytes (with-temp-file path (insert "raced")))
                             ('buffer (with-current-buffer (find-file-noselect path) (insert "raced")))
                             ('destination (with-temp-file (expand-file-name "new.org" root)
                                             (insert "raced"))))
                           '(:ok t :etag "\"old\"" :http-status 200)))
                        ((symbol-function 'jaunder--reconcile-pull-unique-match)
                         (lambda (&rest _) (push 'unique-inventory calls) '(:ok t)))
                        ((symbol-function 'jaunder--reconcile-pull-preflight)
                         (lambda (&rest arguments)
                           (push 'final-preflight calls)
                           (apply real-preflight arguments)))
                        ((symbol-function 'jaunder--reconcile-replace-pulled-file)
                         (lambda (&rest _) (error "must not replace"))))
                (let ((result (jaunder--reconcile-pull-server-ahead-row row)))
                  (should (eq (plist-get result :outcome) 'blocked))
                  (should (equal (nreverse calls)
                                 '(stage unique-inventory remote-revalidation final-preflight))))))
            (let ((buffer (get-file-buffer path)))
              (when (buffer-live-p buffer) (with-current-buffer buffer (set-buffer-modified-p nil))
                    (kill-buffer buffer)))
            (delete-directory root t))))))

(ert-deftest jaunder-reconcile-pull-remote-revalidation-reports-distinct-evidence ()
  "ETag mismatch, malformed ETag, HTTP, and transport failures stay actionable."
  (let ((row (jaunder--make-reconcile-row :member (jaunder-reconcile-test--member "7" "post"))))
    (dolist (fixture '(((:status 200 :headers (("etag" . "\"new\""))) etag-stale)
                       ((:status 200 :headers nil) pull-revalidation-etag-invalid)
                       ((:status 503 :headers (("etag" . "\"old\""))) pull-revalidation-http-error)))
      (cl-letf (((symbol-function 'jaunder--http-request) (lambda (&rest _) (car fixture))))
        (should (eq (plist-get (jaunder--reconcile-pull-remote-revalidation row "\"old\"") :reason)
                    (cadr fixture)))))
    (cl-letf (((symbol-function 'jaunder--http-request) (lambda (&rest _) (error "offline"))))
      (let ((result (jaunder--reconcile-pull-remote-revalidation row "\"old\"")))
        (should (eq (plist-get result :reason) 'pull-revalidation-transport-error))
        (should (string-match-p "offline" (plist-get result :detail)))))))

(ert-deftest jaunder-reconcile-pull-fresh-duplicate-identities-return-blocked-evidence ()
  "Fresh duplicate local and remote IDs are actionable blocked pull results."
  (let* ((root (file-name-as-directory (make-temp-file "jaunder-pull-duplicate-" t)))
         (path (expand-file-name "old.org" root))
         (bytes (jaunder-reconcile-test--pulled-bytes "7" "old" "\"old\""))
         (jaunder-reconcile-report (jaunder--make-reconcile-report :root root))
         row)
    (unwind-protect
        (progn
          (with-temp-file path (insert bytes))
          (setq row (jaunder-reconcile-test--matched-pull-row path "old"))
          (dolist (fixture
                   (list
                    (cons 'duplicate-local-id
                          (jaunder--join-inventory
                           (list (jaunder-reconcile-test--local path "7")
                                 (jaunder-reconcile-test--local (expand-file-name "copy.org" root) "7"))
                           (list (jaunder-reconcile-test--member "7" "old"))))
                    (cons 'duplicate-remote-id 'remote-error)))
            (cl-letf (((symbol-function 'jaunder--pull-stage-member)
                       (lambda (&rest _) (list :id "7" :slug "old" :etag "\"old\""
                                               :synced-at "2026-09-17T00:00:00Z" :bytes bytes)))
                      ((symbol-function 'jaunder--inventory-for-root)
                       (lambda (&rest _)
                         (if (eq (cdr fixture) 'remote-error)
                             (signal 'jaunder-inventory-duplicate-remote-id '("7"))
                           (cdr fixture)))))
              (let ((result (jaunder--reconcile-pull-server-ahead-row row)))
                (should (eq (plist-get result :outcome) 'blocked))
                (should (eq (plist-get result :reason) (car fixture)))
                (should (stringp (plist-get result :detail)))))))
      (delete-directory root t))))

(ert-deftest jaunder-reconcile-pull-staged-identity-drift-is-structured-blocked ()
  "A real D2 slug drift retains current Member evidence without local mutation."
  (let* ((root (file-name-as-directory (make-temp-file "jaunder-pull-drift-" t)))
         (path (expand-file-name "old.org" root))
         (before (jaunder-reconcile-test--pulled-bytes "7" "old" "\"old\""))
         (entry (concat "<entry xmlns=\"http://www.w3.org/2005/Atom\""
                        " xmlns:app=\"http://www.w3.org/2007/app\""
                        " xmlns:j=\"https://jaunder.org/ns/atompub\">"
                        "<title>Renamed</title>"
                        "<link rel=\"edit\" href=\"https://example.test/atompub/alice/posts/7\"/>"
                        "<j:slug>renamed</j:slug><content type=\"text/org\">Body</content>"
                        "<app:control><app:draft>yes</app:draft></app:control></entry>"))
         (jaunder--active-blog '(:base-url "https://example.test" :username "alice"))
         (jaunder-reconcile-report (jaunder--make-reconcile-report :root root))
         row)
    (unwind-protect
        (progn
          (with-temp-file path (insert before))
          (setq row (jaunder-reconcile-test--matched-pull-row path "old"))
          (cl-letf (((symbol-function 'jaunder--fetch-service-document)
                     #'jaunder-reconcile-test--legacy-service-document)
                    ((symbol-function 'jaunder--http-request)
                     (lambda (method url &rest _)
                       (should (equal method "GET"))
                       (should (equal url "https://example.test/atompub/alice/posts/7"))
                       (list :status 200
                             :headers '(("etag" . "\"current\"")
                                        ("x-jaunder-instance" .
                                         "12345678-1234-1234-1234-123456789abc"))
                             :body entry))))
            (let ((result (jaunder--reconcile-pull-server-ahead-row row)))
              (should (eq (plist-get result :outcome) 'blocked))
              (should (eq (plist-get result :reason) 'staged-identity-changed))
              (should (equal (plist-get result :post-id) "7"))
              (should (equal (plist-get result :slug) "renamed"))
              (should (equal (plist-get result :etag) "\"current\""))
              (should (= (plist-get result :http-status) 200))
              (should (equal (plist-get result :detail)
                             "Member response identity changed since inventory"))))
          (should (equal (with-temp-buffer (insert-file-contents-literally path) (buffer-string))
                         before))
          (should-not (file-exists-p (expand-file-name "renamed.org" root))))
      (delete-directory root t))))

(ert-deftest jaunder-reconcile-selection-and-toggle-reject-non-rows ()
  "Region selection ignores headings and toggling requires an actual report row."
  (let* ((row (jaunder--make-reconcile-row :state 'server-only :key "post:7"
                                           :member (jaunder-reconcile-test--member "7" "post")))
         (report (jaunder--make-reconcile-report :root "/tmp" :rows (list row)))
         (buffer (jaunder--render-reconcile-report report)))
    (unwind-protect
        (with-current-buffer buffer
          (goto-char (point-min))
          (should-error (jaunder-reconcile-toggle-mark) :type 'user-error)
          (search-forward "server-only (1)")
          (set-mark (line-beginning-position))
          (goto-char (line-end-position))
          (activate-mark)
          (should-not (jaunder-reconcile-selected-rows))
          (deactivate-mark))
      (kill-buffer buffer))))

(ert-deftest jaunder-reconcile-batch-honors-cancellation-before-an-operation ()
  "Cancellation records no operation and leaves a visible empty batch result."
  (let* ((row (jaunder--make-reconcile-row :state 'local-draft :key "local:draft"))
         (report (jaunder--make-reconcile-report :root "/tmp" :rows (list row)))
         (buffer (jaunder--render-reconcile-report report))
         called)
    (unwind-protect
        (cl-letf (((symbol-function 'jaunder--reconcile-refresh-buffer)
                   (lambda (&rest _) nil)))
          (jaunder--reconcile-execute-batch buffer (list row) 'push
                                            (lambda (_) (setq called t))
                                            (lambda () t))
          (should-not called))
      (kill-buffer buffer))))

(ert-deftest jaunder-reconcile-delete-review-and-local-preservation-branches ()
  "Invalid review responses and unsafe local removal return explicit evidence."
  (let ((row (jaunder--make-reconcile-row
              :state 'unchanged :key "post:7"
              :member (jaunder-reconcile-test--member "7" "post"))))
    (dolist (response (list '(:status 200 :headers nil) '(:status 503 :headers nil)))
      (cl-letf (((symbol-function 'jaunder--http-request) (lambda (&rest _) response)))
        (should (eq (plist-get (jaunder--reconcile-delete-etag row) :outcome) 'blocked))))
    (let* ((path (make-temp-file "jaunder-reconcile-delete-local-" nil ".org"))
           (buffer (find-file-noselect path))
           (row (jaunder--make-reconcile-row
		 :local (jaunder-reconcile-test--local path "7"))))
      (unwind-protect
          (progn
            (with-current-buffer buffer (insert "changed"))
            (should (eq (plist-get (jaunder--reconcile-delete-local-file row) :reason)
			'local-buffer-modified))
            (with-current-buffer buffer (set-buffer-modified-p nil))
            (cl-letf (((symbol-function 'jaunder--reconcile-current-local-id) (lambda (_) "8")))
              (should (eq (plist-get (jaunder--reconcile-delete-local-file row) :reason)
                          'matched-identity-changed))))
	(when (buffer-live-p buffer) (kill-buffer buffer))
	(delete-file path)))))

(ert-deftest jaunder-reconcile-pull-blocks-invalid-review-and-surfaces-stage-failures ()
  "A server-ahead pull preserves row identity for invalid reviews and exceptions."
  (let* ((row (jaunder--make-reconcile-row
	       :state 'server-ahead :key "post:7" :remote-etag "weak"
	       :local (jaunder-reconcile-test--local "/tmp/post.org" "7")
	       :member (jaunder-reconcile-test--member "7" "post")))
         (jaunder-reconcile-report (jaunder--make-reconcile-report :root "/tmp")))
    (should (eq (plist-get (jaunder--reconcile-pull-server-ahead-row row) :reason)
                'reviewed-etag-invalid))
    (setf (jaunder-reconcile-row-remote-etag row) "\"old\"")
    (cl-letf (((symbol-function 'jaunder--pull-stage-member) (lambda (&rest _) (error "offline"))))
      (should (eq (plist-get (jaunder--reconcile-pull-server-ahead-row row) :reason)
                  'pull-failed)))))

(ert-deftest jaunder-reconcile-selected-commands-reject-empty-selection ()
  "Every selected operation rejects an empty selection before transport or prompts."
  (let ((buffer (jaunder--render-reconcile-report
                 (jaunder--make-reconcile-report :root "/tmp" :rows nil))))
    (unwind-protect
        (with-current-buffer buffer
          (dolist (command '(jaunder-reconcile-push-selected
			     jaunder-reconcile-pull-selected
			     jaunder-reconcile-delete-selected))
	    (should-error (funcall command) :type 'user-error)))
      (kill-buffer buffer))))

(ert-deftest jaunder-reconcile-report-interactions-cover-row-region-removal-and-stale-point ()
  "Rows selected by region and marking retain only current report positions."
  (let* ((row (jaunder--make-reconcile-row :state 'server-only :key "post:7"
                                           :member (jaunder-reconcile-test--member "7" "post")))
         (report (jaunder--make-reconcile-report :root "/tmp" :rows (list row)))
         (buffer (generate-new-buffer " *jaunder-reconcile-interaction*")))
    (unwind-protect
        (with-current-buffer buffer
          (setq-local jaunder-reconcile-report report)
          (setq-local jaunder-reconcile-marks (make-hash-table :test #'equal))
          (insert "row")
          (add-text-properties (point-min) (point-max) `(jaunder-reconcile-row ,row))
          (set-mark (point-min))
          (goto-char (point-max))
          (activate-mark)
          (should (equal (jaunder-reconcile-selected-rows) (list row)))
          (deactivate-mark)
          (goto-char (point-min))
          (cl-letf (((symbol-function 'jaunder--render-reconcile-report)
                     (lambda (&rest _) nil)))
            (jaunder-reconcile-toggle-mark)
            (jaunder-reconcile-toggle-mark))
          (jaunder--render-reconcile-report
           (jaunder--make-reconcile-report :root "/tmp" :rows nil) buffer)
          (should (= (point) (point-min))))
      (kill-buffer buffer))))

(ert-deftest jaunder-reconcile-local-safety-and-push-missing-file-are-blocked ()
  "A late on-disk identity drift and missing push file produce named blocks."
  (let ((path (make-temp-file "jaunder-reconcile-safety-" nil ".org")))
    (unwind-protect
        (progn
          (with-temp-file path (insert "#+PROPERTY: JAUNDER_ID 7\n"))
          (let ((row (jaunder--make-reconcile-row
                      :state 'local-ahead :local (jaunder-reconcile-test--local path "7")
                      :member (jaunder-reconcile-test--member "7" "post"))))
            (cl-letf (((symbol-function 'jaunder--buffer-property) (lambda (_) "7"))
                      ((symbol-function 'jaunder--reconcile-current-local-id) (lambda (_) "8")))
              (should (eq (jaunder--reconcile-local-mutation-safety-reason row)
                          'matched-identity-changed))))
	  (delete-file path)))
    (let ((row (jaunder--make-reconcile-row
		:state 'local-draft
		:local (jaunder-reconcile-test--local "/definitely/missing.org" nil))))
      (cl-letf (((symbol-function 'jaunder--reconcile-local-mutation-safety-reason) (lambda (_) nil)))
	(should (eq (plist-get (jaunder--reconcile-push-row row) :reason)
                    'local-file-missing))))))

(ert-deftest jaunder-reconcile-delete-review-transport-and-buffer-retention-are-explicit ()
  "Review transport errors and failed buffer closure retain actionable outcomes."
  (let ((row (jaunder--make-reconcile-row
	      :member (jaunder-reconcile-test--member "7" "post"))))
    (cl-letf (((symbol-function 'jaunder--http-request) (lambda (&rest _) (error "offline"))))
      (should (eq (plist-get (jaunder--reconcile-delete-etag row) :reason)
                  'member-transport-error))))
  (let* ((path (make-temp-file "jaunder-reconcile-retain-" nil ".org"))
         (buffer (find-file-noselect path))
         (row (jaunder--make-reconcile-row :local (jaunder-reconcile-test--local path "7")
                                           :member (jaunder-reconcile-test--member "7" "post"))))
    (unwind-protect
        (progn
          (with-current-buffer buffer (set-buffer-modified-p nil))
          (cl-letf (((symbol-function 'jaunder--reconcile-current-local-id) (lambda (_) "7"))
                    ((symbol-function 'kill-buffer) (lambda (&rest _) nil)))
            (should (eq (plist-get (jaunder--reconcile-delete-local-file row) :local-effect)
			'removed-buffer-retained))))
      (when (buffer-live-p buffer) (kill-buffer buffer))
      (when (file-exists-p path) (delete-file path)))))

(ert-deftest jaunder-reconcile-fresh-inventory-and-staging-fallbacks-preserve-row-identity ()
  "Changed inventory, inventory errors, and sparse staged drift remain structured."
  (let* ((path (make-temp-file "jaunder-reconcile-unique-" nil ".org"))
         (row (jaunder--make-reconcile-row
	       :local (jaunder-reconcile-test--local path "7")
	       :member (jaunder-reconcile-test--member "7" "post")
	       :key "post:7" :state 'server-ahead :remote-etag "\"old\""))
         (jaunder-reconcile-report (jaunder--make-reconcile-report :root "/tmp")))
    (unwind-protect
        (progn
          (cl-letf (((symbol-function 'jaunder--inventory-for-root)
                     (lambda (&rest _) (jaunder--join-inventory nil nil))))
            (should (eq (plist-get (jaunder--reconcile-pull-unique-match row) :reason)
                        'matched-identity-changed)))
          (cl-letf (((symbol-function 'jaunder--inventory-for-root) (lambda (&rest _) (error "offline"))))
            (should (eq (plist-get (jaunder--reconcile-pull-unique-match row) :reason)
                        'fresh-inventory-failed)))
          (cl-letf (((symbol-function 'jaunder--pull-stage-member)
                     (lambda (&rest _)
		       (signal 'jaunder-pull-stage-identity-changed (list nil)))))
            (let ((result (jaunder--reconcile-pull-server-ahead-row row)))
	      (should (equal (plist-get result :post-id) "7"))
	      (should (equal (plist-get result :slug) "post")))))
      (when (file-exists-p path) (delete-file path)))))

(ert-deftest jaunder-reconcile-replacement-cleans-a-temporary-after-write-failure ()
  "A failure after temporary allocation removes the uninstalled pull bytes."
  (let* ((root (file-name-as-directory (make-temp-file "jaunder-reconcile-cleanup-" t)))
         (path (expand-file-name "post.org" root)))
    (unwind-protect
        (cl-letf (((symbol-function 'rename-file) (lambda (&rest _) (error "rename failed"))))
          (should-error (jaunder--reconcile-replace-pulled-file path path "replacement"))
          (should-not (directory-files root nil "\\`\\.jaunder-pull-")))
      (delete-directory root t))))

(ert-deftest jaunder-reconcile-delete-selected-records-preflight-and-ineligible-reviews ()
  "Review blocks become ordered terminal results without any remote mutation."
  (let* ((safe (jaunder--make-reconcile-row :state 'server-only :key "post:7"
                                            :member (jaunder-reconcile-test--member "7" "post")))
         (unsafe (jaunder--make-reconcile-row :state 'conflict :key "conflict:7"))
         (buffer (jaunder--render-reconcile-report
                  (jaunder--make-reconcile-report :root "/tmp" :rows (list safe unsafe))))
         methods)
    (unwind-protect
        (with-current-buffer buffer
          (puthash "post:7" t jaunder-reconcile-marks)
          (puthash "conflict:7" t jaunder-reconcile-marks)
          (cl-letf (((symbol-function 'jaunder--call-with-blog) (lambda (_ thunk) (funcall thunk)))
                    ((symbol-function 'jaunder--reconcile-delete-preflight)
                     (lambda (_) 'local-buffer-modified))
                    ((symbol-function 'jaunder--reconcile-refresh-buffer)
                     (lambda (&rest _) nil))
                    ((symbol-function 'jaunder--http-request)
                     (lambda (method &rest _)
                       (push method methods)
                       (error "unexpected remote mutation")))
                    ((symbol-function 'y-or-n-p) (lambda (_) t)))
            (jaunder-reconcile-delete-selected)
            (should-not methods)
            (should (= (length jaunder-reconcile-last-batch-results) 2))
            (should (equal (mapcar #'jaunder-reconcile-result-row-key
                                   jaunder-reconcile-last-batch-results)
                           '("conflict:7" "post:7")))
            (should (equal (mapcar #'jaunder-reconcile-result-outcome
                                   jaunder-reconcile-last-batch-results)
                           '(blocked blocked)))
            (should (equal (mapcar #'jaunder-reconcile-result-reason
                                   jaunder-reconcile-last-batch-results)
                           '(delete-ineligible local-buffer-modified)))
            (let ((ineligible (car jaunder-reconcile-last-batch-results))
                  (preflight (cadr jaunder-reconcile-last-batch-results)))
              (should (eq (jaunder-reconcile-result-detail ineligible) 'conflict))
              (should (equal (jaunder-reconcile-result-post-id preflight) "7"))
              (should (equal (jaunder-reconcile-result-slug preflight) "post"))
              (should (eq (jaunder-reconcile-result-local-effect preflight) 'unchanged)))))
      (kill-buffer buffer))))

(provide 'jaunder-reconcile-test)
;;; jaunder-reconcile-test.el ends here
