;;; jaunder-post-link.el --- Local Post Link mapping -*- lexical-binding: t; -*-

;; Copyright (C) 2026 Jaunder contributors

;;; Commentary:
;; Local Post Links are relative body-level Org file links between Posts in one
;; configured Jaunder root.  This module validates exact local and Collection
;; evidence to map authored relative links to server-advertised alternate hrefs
;; for publication, and to restore relative destinations during pull only when
;; the same proof remains current.  It owns neither media transfer nor Post
;; installation/write-back.

;;; Code:

(require 'seq)
(require 'jaunder-config)
(require 'jaunder-org)
(require 'jaunder-inventory)
(require 'jaunder-debug)

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
  (jaunder--with-debug-operation "post-link.publish" ()
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
                                       (jaunder--org-substitute-links body #'jaunder--local-post-link-candidate-p urls))))))

(defun jaunder--pulled-post-link-target-p (local member root)
  "Return non-nil when LOCAL still proves MEMBER's same-root Org target."
  (let ((path (jaunder-inventory-local-path local)))
    (and (not (jaunder--inventory-local-member-evidence-reason local member))
         (file-regular-p path)
         (file-in-directory-p (file-truename path) (file-truename root))
         (pcase-let ((`(,id ,slug) (jaunder--read-local-properties path)))
           (let ((current (jaunder--make-inventory-local
                           :path path :id (jaunder--canonical-post-id id) :slug slug)))
             (not (jaunder--inventory-local-member-evidence-reason current member)))))))

(defun jaunder--pulled-post-link-replacements (root members locals)
  "Return exact canonical-href to local-link replacements proven by inventories.
Post IDs and canonical hrefs require singleton inventory evidence before current
file proof.  Invalid alternate outcomes, duplicate evidence (even stale), and
incomplete local evidence deliberately produce no replacement."
  (jaunder--with-debug-operation
   "post-link.evidence" (members (length members) count (length locals))
   (let ((by-href (make-hash-table :test #'equal))
         (local-by-id (jaunder--index-by locals #'jaunder-inventory-local-id))
         (member-by-id (jaunder--index-by members #'jaunder-inventory-member-id))
         (member-by-href (jaunder--index-by members #'jaunder-inventory-member-alternate-href)))
     (maphash
      (lambda (id member-bucket)
        (let ((local-bucket (gethash id local-by-id)))
          (when (and (= (length member-bucket) 1) (= (length local-bucket) 1))
            (let* ((member (car member-bucket))
                   (href (jaunder-inventory-member-alternate-href member)))
              (when (and href
                         (= (length (gethash href member-by-href)) 1)
                         (not (jaunder-inventory-member-alternate-invalid-reason member))
                         (jaunder--pulled-post-link-target-p (car local-bucket) member root))
                (puthash href (list (format "./%s.org" (jaunder-inventory-member-slug member)))
                         by-href))))))
      member-by-id)
     by-href)))

(defun jaunder--reverse-pulled-post-links (body root members locals)
  "Restore proven Local Post Links in Org BODY without changing other source.
MEMBERS and LOCALS are inventory evidence.  The Org parser authorizes only real
body links; replacements edit just their destination spans, right-to-left.
Canonical hrefs compare as unnormalized, byte-for-byte source strings."
  (jaunder--with-debug-operation "post-link.pull" ()
                                 (let ((replacements (jaunder--pulled-post-link-replacements root members locals)))
                                   (with-temp-buffer
                                     (insert body)
                                     (org-mode)
                                     (let (edits)
                                       (org-element-map
                                        (org-element-parse-buffer) 'link
                                        (lambda (link)
                                          (let* ((raw (org-element-property :raw-link link))
                                                 (values (and raw (gethash raw replacements))))
                                            (when (= (length values) 1)
                                              (let ((begin (org-element-property :begin link)))
                                                (when (and (stringp raw)
                                                           (string-prefix-p (concat "[[" raw)
                                                                            (buffer-substring-no-properties begin (point-max))))
                                                  (push (list (+ begin 2) (+ begin 2 (length raw)) (car values)) edits)))))))
                                       (dolist (edit edits)
                                         (delete-region (nth 0 edit) (nth 1 edit))
                                         (goto-char (nth 0 edit))
                                         (insert (nth 2 edit)))
                                       (buffer-substring-no-properties (point-min) (point-max)))))))

(provide 'jaunder-post-link)
;;; jaunder-post-link.el ends here
