;;; jaunder-reconcile-performance-fixture.el --- Shared paginated fixture -*- lexical-binding: t; -*-

;; Copyright (C) 2026 Jaunder contributors

;;; Commentary:
;; The inventory-only and complete selected-pull measurements share the same
;; 100-Member, four-page Collection topology.  Only the full pull needs ETags
;; in the Collection; the inventory-only test also covers their absence.
;; Public alternate hrefs exercise identity-aware Local Post Link reversal.

;;; Code:

(defun jaunder-test--collection-page (url &optional etag)
  "Return one 25-Member Collection page for URL, optionally with ETAG."
  (let* ((page (if (string-match "/page-\\([2-4]\\)\\'" url)
                   (string-to-number (match-string 1 url)) 1))
         (start (1+ (* 25 (1- page))))
         (next (when (< page 4)
                 (format "https://example.test/page-%d" (1+ page)))))
    (concat
     "<feed xmlns=\"http://www.w3.org/2005/Atom\" xmlns:j=\"https://jaunder.org/ns/atompub\">"
     (when next (format "<link rel=\"next\" href=\"%s\"/>" next))
     (mapconcat
      (lambda (id)
        (format
         "<entry><link rel=\"edit\" href=\"https://example.test/atompub/alice/posts/%d\"/><link rel=\"alternate\" href=\"https://example.test/~alice/post-%03d\"/><j:slug>post-%03d</j:slug>%s</entry>"
         id id id (if etag (format "<j:etag>%s</j:etag>" etag) "")))
      (number-sequence start (+ start 24)) "")
     "</feed>")))

(provide 'jaunder-reconcile-performance-fixture)
;;; jaunder-reconcile-performance-fixture.el ends here
