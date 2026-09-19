;;; jaunder-post-link.el --- Local Post Link publish preflight -*- lexical-binding: t; -*-

;; Copyright (C) 2026 Jaunder contributors

;;; Commentary:
;; Local Post Links are relative body-level Org file links between Posts in one
;; configured Jaunder root.  This module validates their exact local evidence
;; against the read-only Collection inventory and substitutes only the
;; server-advertised alternate href in the body sent for publication.  It owns
;; neither media upload nor publish write-back.

;;; Code:

(require 'seq)
(require 'jaunder-config)
(require 'jaunder-org)
(require 'jaunder-inventory)

(defun jaunder--local-post-link-root ()
  "Return the configured Jaunder root containing the current Post file."
  (let ((entry (and buffer-file-name (jaunder--blog-entry-for buffer-file-name))))
    (or (and entry (file-name-as-directory (expand-file-name (car entry))))
        (error "jaunder: Local Post Link has no configured root"))))

(defun jaunder--local-post-link-target (record root)
  "Return RECORD's target local evidence, or signal its visible diagnostic.
ROOT is the configured Jaunder root.  No filename search is performed."
  (when (or (plist-get record :search-option)
            (string-match-p "[?#]" (plist-get record :path)))
    (error "jaunder: Local Post Link cannot carry a search target, query, or fragment"))
  (let ((path (plist-get record :file)))
    (unless (and (file-exists-p path)
                 (file-in-directory-p (file-truename path) (file-truename root))
                 (file-regular-p path))
      (error "jaunder: Local Post Link target must be a regular file in the configured root"))
    (pcase-let ((`(,id ,slug) (jaunder--read-local-properties path)))
      (let ((canonical-id (jaunder--canonical-post-id id)))
        (unless canonical-id
          (error "jaunder: Local Post Link target has missing or invalid Post ID"))
        (jaunder--make-inventory-local :path path :id canonical-id :slug slug)))))

(defun jaunder--local-post-link-member (local members)
  "Return LOCAL's uniquely identified Member in MEMBERS, or signal a diagnostic."
  (let ((member (seq-find (lambda (candidate)
                            (equal (jaunder-inventory-member-id candidate)
                                   (jaunder-inventory-local-id local)))
                          members)))
    (unless member
      (error "jaunder: Local Post Link target is not a Collection Member"))
    (let ((evidence (jaunder--inventory-local-member-evidence-reason local member)))
      (when evidence
        (error "jaunder: Local Post Link target evidence failed: %s" evidence)))
    (when (jaunder-inventory-member-alternate-invalid-reason member)
      (error "jaunder: Local Post Link Member alternate is invalid: %s"
             (jaunder-inventory-member-alternate-invalid-reason member)))
    member))

(defun jaunder--localize-post-links (body)
  "Return BODY with proven Local Post Links replaced by authoritative hrefs.
The current buffer remains authored source.  Candidates are validated before
Collection fetch and media upload; only their exact target files join the remote
Member inventory."
  (let ((records (seq-filter #'jaunder--local-post-link-candidate-p
                             (jaunder--org-body-links))))
    (if (null records)
        body
      (let* ((root (jaunder--local-post-link-root))
             (locals (mapcar (lambda (record)
                               (jaunder--local-post-link-target record root))
                             records))
             (members (jaunder--fetch-collection-members))
             (urls (mapcar (lambda (local)
                             (jaunder-inventory-member-alternate-href
                              (jaunder--local-post-link-member local members)))
                           locals)))
        (jaunder--org-substitute-links body #'jaunder--local-post-link-candidate-p urls)))))

(provide 'jaunder-post-link)
;;; jaunder-post-link.el ends here
