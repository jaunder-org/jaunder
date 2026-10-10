;;; jaunder-reconcile-operation.el --- Operation-owned read evidence -*- lexical-binding: t; -*-

;; Copyright (C) 2026 Jaunder contributors

;;; Commentary:
;; One short-lived owner for complete remote discovery and fresh local proof.
;; Discovery does not replace reviewed ETags or authorize mutations.  The private
;; dynamic state stays inside this module; consumers request inventory or proof
;; with scalar root/identity/path arguments.  Report refresh runs outside scope.

;;; Code:

(require 'cl-lib)
(require 'jaunder-config)
(require 'jaunder-inventory)

(defvar jaunder--operation-evidence nil
  "Private evidence owned by the current read operation.")

(defun jaunder--call-with-reconcile-operation (root origin username work)
  "Run WORK in independent evidence scope for ROOT, ORIGIN and USERNAME."
  (let ((jaunder--operation-evidence
         (list :root root :origin origin :username username :state 'unacquired)))
    (funcall work)))

(defun jaunder--call-without-reconcile-operation (work)
  "Run WORK without inheriting an enclosing operation's discovery."
  (let ((jaunder--operation-evidence nil))
    (funcall work)))

(defun jaunder--operation-active-p ()
  "Return non-nil inside an operation evidence scope."
  (not (null jaunder--operation-evidence)))

(defun jaunder--operation-check-scope (root)
  "Reject evidence use outside its captured ROOT, origin or User."
  (unless (and jaunder--operation-evidence
               (equal root (plist-get jaunder--operation-evidence :root))
               (equal (jaunder--active-base-url)
                      (plist-get jaunder--operation-evidence :origin))
               (equal (jaunder--active-username)
                      (plist-get jaunder--operation-evidence :username)))
    (error "jaunder reconcile: operation Collection evidence scope mismatch")))

(defun jaunder--operation-remote-members (root)
  "Return ROOT's complete discovery, or signal the retained acquisition failure."
  (jaunder--operation-check-scope root)
  (pcase (plist-get jaunder--operation-evidence :state)
    ((or 'complete 'complete-empty) (plist-get jaunder--operation-evidence :members))
    ('failed (let ((condition (plist-get jaunder--operation-evidence :condition)))
               (signal (car condition) (cdr condition))))
    ('unacquired
     (condition-case err
         (let ((members (jaunder--fetch-collection-members)))
           (setq jaunder--operation-evidence
                 (plist-put jaunder--operation-evidence :members members))
           (setq jaunder--operation-evidence
                 (plist-put jaunder--operation-evidence :state
                            (if members 'complete 'complete-empty)))
           members)
       (error
        (setq jaunder--operation-evidence
              (plist-put jaunder--operation-evidence :condition err))
        (setq jaunder--operation-evidence
              (plist-put jaunder--operation-evidence :state 'failed))
        (signal (car err) (cdr err)))))))

(defun jaunder--operation-current-inventory (root)
  "Join current local files in ROOT to operation discovery when scoped.
Unscoped consumers retain standalone complete acquisition semantics."
  (if (jaunder--operation-active-p)
      (progn
        (jaunder--operation-check-scope root)
        (jaunder--join-inventory (jaunder--scan-root-locals root)
                                 (jaunder--operation-remote-members root)))
    (jaunder--inventory-for-root root)))

(defun jaunder--operation-post-link-evidence (root)
  "Return remote and current local link proof for ROOT without mutation authority."
  (jaunder--inventory-post-link-evidence
   (jaunder--operation-current-inventory root)))

(defun jaunder--operation-unique-match (root id path)
  "Return current unique-match proof for ID at PATH in ROOT.
A duplicate local or remote Post ID is an actionable blocked result, rather
than an exception flattened into a generic pull failure."
  (condition-case err
      (let* ((inventory (jaunder--operation-current-inventory root))
             (duplicate-local
              (cl-find-if
               (lambda (conflict)
                 (and (memq 'duplicate-local-id
                            (jaunder-inventory-conflict-kinds conflict))
                      (cl-some (lambda (local)
                                 (and (equal (jaunder-inventory-local-id local) id)
                                      (equal (jaunder-inventory-local-path local) path)))
                               (jaunder-inventory-conflict-locals conflict))))
               (jaunder-inventory-conflicts inventory)))
             (matches (cl-remove-if-not
                       (lambda (match)
                         (and (equal (jaunder-inventory-local-path
                                      (jaunder-inventory-match-local match)) path)
                              (equal (jaunder-inventory-local-id
                                      (jaunder-inventory-match-local match)) id)
                              (equal (jaunder-inventory-member-id
                                      (jaunder-inventory-match-member match)) id)))
                       (jaunder-inventory-matched inventory)))
             (members (cl-remove-if-not
                       (lambda (member)
                         (equal (jaunder-inventory-member-id member) id))
                       (append (jaunder-inventory-server-only inventory)
                               (mapcar #'jaunder-inventory-match-member
                                       (jaunder-inventory-matched inventory))))))
        (cond (duplicate-local
               (list :reason 'duplicate-local-id
                     :detail (format "fresh inventory has duplicate local Post ID %s" id)))
              ((and (= (length matches) 1) (= (length members) 1))
               (list :ok t))
              (t (list :reason 'matched-identity-changed
                       :detail "fresh inventory no longer has the reviewed unique match"))))
    (jaunder-inventory-duplicate-remote-id
     (list :reason 'duplicate-remote-id
           :detail (format "fresh inventory has duplicate remote Post ID %s" id)))
    (error (list :reason 'fresh-inventory-failed
                 :detail (error-message-string err)))))

(defun jaunder--operation-local-unique-match (root id path)
  "Return final local uniqueness proof for ID at PATH in ROOT."
  (when (jaunder--operation-active-p)
    (jaunder--operation-check-scope root))
  (let* ((locals (jaunder--scan-root-locals root))
         (same-id (cl-remove-if-not
                   (lambda (local) (equal (jaunder-inventory-local-id local) id)) locals)))
    (cond ((> (length same-id) 1)
           (list :reason 'duplicate-local-id
                 :detail (format "final local scan has duplicate local Post ID %s" id)))
          ((/= (length same-id) 1)
           (list :reason 'matched-identity-changed
                 :detail "final local scan no longer has the reviewed local Post"))
          ((not (equal (jaunder-inventory-local-path (car same-id)) path))
           (list :reason 'matched-identity-changed
                 :detail "final local scan no longer has the reviewed local Post"))
          (t (list :ok t)))))

(provide 'jaunder-reconcile-operation)
;;; jaunder-reconcile-operation.el ends here
