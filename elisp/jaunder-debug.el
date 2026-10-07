;;; jaunder-debug.el --- Opt-in bounded client diagnostics -*- lexical-binding: t; -*-

;; Copyright (C) 2026 Jaunder contributors

;;; Commentary:
;; Private, bounded operation timing for the Protocol Client.  This module
;; accepts only the finite diagnostic vocabulary so callers cannot accidentally
;; serialize request or authored data.

;;; Code:

(require 'cl-lib)

(defgroup jaunder-debug nil
  "Opt-in Jaunder client diagnostics."
  :group 'applications)

(defcustom jaunder-debug nil
  "When non-nil, record bounded operation diagnostics in `*Jaunder Debug*'."
  :type 'boolean
  :group 'jaunder-debug)

(defconst jaunder--debug-maximum-number 9007199254740991)
(defconst jaunder--debug-maximum-events 10000)
(defconst jaunder--debug-buffer-name "*Jaunder Debug*")
(defconst jaunder--debug-labels
  '("config.resolve" "auth.lookup" "author.new" "author.complete" "author.cancel"
    "publish.post" "publish.draft" "publish.validate" "publish.create"
    "publish.recover" "publish.update" "publish.checkpoint" "delete.post"
    "report.open" "report.refresh" "inventory.build" "inventory.local"
    "inventory.collection" "inventory.page" "reconcile.batch" "reconcile.row"
    "conflict.local" "conflict.remote" "conflict.merge" "merge.stage"
    "merge.finish" "merge.cancel" "merge.discard" "pull.stage"
    "pull.revalidate" "pull.preflight" "pull.install" "transport.request"
    "service.read" "service.parse" "atom.parse" "atom.serialize" "org.parse"
    "org.serialize" "member.identity" "member.parse" "post-link.publish"
    "post-link.pull" "post-link.evidence" "media.plan" "media.materialize"
    "media.apply" "media.upload" "media.download" "media.path" "media.hash"
    "media.verify"))
(defconst jaunder--debug-enums
  '((method . ("GET" "HEAD" "POST" "PUT" "DELETE"))
    (format . ("org" "markdown" "html" "atom"))
    (action . ("new" "complete" "cancel" "discard" "publish" "draft" "create"
               "recover" "update" "push" "pull" "keep-local" "keep-remote"
               "merge" "delete" "refresh"))
    (eligibility . ("eligible" "ineligible"))
    (decision . ("proceed" "blocked" "no-op" "retry" "recovered" "partial"
                 "remote-unknown"))
    (reason . ("invalid" "stale" "ambiguous" "ineligible" "missing" "unsafe-path"
               "modified" "conflict" "transport" "decode" "io" "partial"
               "remote-unknown" "unexpected"))))
(defconst jaunder--debug-reserved-fields
  '(at correlation span parent label phase elapsed-ms outcome))

(cl-defstruct (jaunder--debug-span (:constructor jaunder--make-debug-span))
  "One logged operation's identity, clock, terminal fields and outcome."
  label correlation id parent start fields (outcome "success"))

(defvar jaunder--debug-id-counter 0)
(defvar jaunder--debug-event-count 0)
(defvar jaunder--debug-discarded 0)
(defvar jaunder--debug-operation-stack nil)

(define-derived-mode jaunder-debug-mode special-mode "Jaunder-Debug"
  "Mode for retained Jaunder diagnostic events."
  (setq-local buffer-read-only t))

(defun jaunder--debug-now ()
  "Return the current diagnostic clock value."
  (float-time))

(defun jaunder--debug-timestamp ()
  "Return the current UTC timestamp with actual millisecond precision."
  (let* ((now (jaunder--debug-now))
         (seconds (floor now))
         (milliseconds (floor (* 1000 (- now seconds)))))
    (format "%s.%03dZ" (format-time-string "%Y-%m-%dT%H:%M:%S" seconds t)
            milliseconds)))

(defun jaunder--debug-id-p (value)
  "Return non-nil when VALUE is an internally safe diagnostic ID."
  (and (stringp value)
       (let ((case-fold-search nil))
         (string-match-p "\\`[a-z0-9-]\\{1,32\\}\\'" value))))

(defun jaunder--debug-next-id ()
  "Allocate an ID that remains unique for this Emacs session."
  (setq jaunder--debug-id-counter (1+ jaunder--debug-id-counter))
  (format "debug-%d" jaunder--debug-id-counter))

(defun jaunder--debug-saturate (value)
  "Limit VALUE to the permitted nonnegative diagnostic numeric range."
  (min jaunder--debug-maximum-number (max 0 value)))

(defun jaunder--debug-normalize-fields (fields)
  "Return validated optional FIELDS, mapping unknown enum values to `unknown'."
  (when (and (listp fields) (cl-evenp (length fields)))
    (let ((valid t) result)
      (while fields
        (let* ((key (pop fields))
               (value (pop fields))
               (enum (assq key jaunder--debug-enums)))
          (cond
           ((memq key jaunder--debug-reserved-fields) (setq valid nil))
           (enum
            (unless (stringp value) (setq valid nil))
            (when (stringp value)
              (setq result (plist-put result key
                                      (if (member value (cdr enum)) value "unknown")))))
           ((memq key '(count bytes page members))
            (if (and (integerp value) (>= value 0))
                (setq result (plist-put result key (jaunder--debug-saturate value)))
              (setq valid nil)))
           ((eq key 'http-status)
            (if (and (integerp value) (<= 100 value 599))
                (setq result (plist-put result key value))
              (setq valid nil)))
           ((eq key 'reused)
            (if (booleanp value)
                (setq result (plist-put result key value))
              (setq valid nil)))
           (t (setq valid nil)))))
      (and valid result))))

(defun jaunder--debug-fallback-warning ()
  "Use an independent fixed fallback when the warning channel itself fails."
  ;; `message' keeps a broken warning channel from becoming a second diagnostic sink.
  (condition-case nil
      (message "jaunder: diagnostic output unavailable")
    (error nil)
    (quit nil)))

(defun jaunder--debug-warning ()
  "Report a fixed non-sensitive diagnostic failure without recursive logging."
  (condition-case nil
      (display-warning 'jaunder "jaunder: diagnostic output unavailable" :warning)
    (error (condition-case nil
               (jaunder--debug-fallback-warning)
             (error nil) (quit nil)))
    (quit (condition-case nil
              (jaunder--debug-fallback-warning)
            (error nil) (quit nil)))))

(defun jaunder--debug-safe (thunk)
  "Run ancillary diagnostic THUNK without changing a primary operation outcome."
  ;; Defer pending keyboard quit until the operation's handler can observe it.
  ;; An explicit diagnostic quit remains an ancillary failure like an error.
  (let ((inhibit-quit t))
    (save-match-data
      (condition-case nil
          (funcall thunk)
        (error (jaunder--debug-warning) nil)
        (quit (jaunder--debug-warning) nil)))))

(defun jaunder--debug-format-event (event)
  "Encode already validated EVENT as an ASCII line no longer than 1,024 bytes."
  (let ((line (mapconcat (lambda (pair) (format "%s=%s" (car pair) (cdr pair))) event " ")))
    (and (string-match-p "\\`[[:ascii:]]*\\'" line)
         (<= (string-bytes line) 1024)
         line)))

(defun jaunder--debug-timestamp-p (value)
  "Return non-nil for a valid UTC millisecond timestamp VALUE."
  (and (stringp value)
       (string-match-p
        "\\`[0-9]\\{4\\}-[0-9]\\{2\\}-[0-9]\\{2\\}T[0-9]\\{2\\}:[0-9]\\{2\\}:[0-9]\\{2\\}\\.[0-9]\\{3\\}Z\\'"
        value)
       ;; Encoding normalizes impossible dates; round-tripping rejects them.
       (let ((time (encode-time
                    (string-to-number (substring value 17 19))
                    (string-to-number (substring value 14 16))
                    (string-to-number (substring value 11 13))
                    (string-to-number (substring value 8 10))
                    (string-to-number (substring value 5 7))
                    (string-to-number (substring value 0 4)) t)))
         (equal (substring value 0 19)
                (format-time-string "%Y-%m-%dT%H:%M:%S" time t)))))

(defun jaunder--debug-common-event (label phase correlation span parent fields outcome elapsed)
  "Return a fully validated event alist, or nil before any formatting occurs."
  (when (and (member label jaunder--debug-labels)
             (member phase '("start" "end"))
             (jaunder--debug-id-p correlation)
             (jaunder--debug-id-p span)
             (or (null parent) (jaunder--debug-id-p parent))
             (or (null fields) (jaunder--debug-normalize-fields fields))
             (if (equal phase "end")
                 (and (member outcome '("success" "error" "cancelled"))
                      (integerp elapsed) (>= elapsed 0))
               (and (null outcome) (null elapsed))))
    (let ((timestamp (jaunder--debug-timestamp))
          (safe-fields (jaunder--debug-normalize-fields fields)))
      (when (jaunder--debug-timestamp-p timestamp)
        (append `((at . ,timestamp) (correlation . ,correlation) (span . ,span)
                  (label . ,label) (phase . ,phase))
                (when parent `((parent . ,parent)))
                (when (equal phase "end")
                  `((elapsed-ms . ,(jaunder--debug-saturate elapsed))
                    (outcome . ,outcome)))
                (mapcar (lambda (key) (cons (symbol-name key) (plist-get safe-fields key)))
                        (cl-loop for key on safe-fields by #'cddr collect (car key))))))))

(defun jaunder--debug-buffer ()
  "Return the diagnostic buffer, resetting retained accounting after a kill."
  (unless (get-buffer jaunder--debug-buffer-name)
    (setq jaunder--debug-event-count 0 jaunder--debug-discarded 0))
  (let ((buffer (get-buffer-create jaunder--debug-buffer-name)))
    (with-current-buffer buffer
      (unless (derived-mode-p 'jaunder-debug-mode) (jaunder-debug-mode)))
    buffer))

(defun jaunder--debug-update-marker ()
  "Maintain one bounded first-line eviction marker in the current buffer."
  (when (> jaunder--debug-discarded 0)
    (goto-char (point-min))
    (if (looking-at "evicted=[0-9]+$")
        (replace-match (format "evicted=%d" jaunder--debug-discarded) t t)
      (insert (format "evicted=%d\n" jaunder--debug-discarded)))))

(defun jaunder--debug-append-line (line)
  "Append LINE incrementally while retaining only the newest event lines."
  (with-current-buffer (jaunder--debug-buffer)
    (let ((inhibit-read-only t))
      (goto-char (point-max))
      (insert line "\n")
      (setq jaunder--debug-event-count (1+ jaunder--debug-event-count))
      (when (> jaunder--debug-event-count jaunder--debug-maximum-events)
        (goto-char (point-min))
        (when (> jaunder--debug-discarded 0) (forward-line 1))
        (delete-region (point) (progn (forward-line 1) (point)))
        (setq jaunder--debug-event-count (1- jaunder--debug-event-count)
              jaunder--debug-discarded
              (jaunder--debug-saturate (1+ jaunder--debug-discarded)))
        (goto-char (point-min))
        (jaunder--debug-update-marker)))))

(defun jaunder--debug-write (label phase correlation span parent fields &optional outcome elapsed)
  "Write one validated event and report any rejected or failed diagnostic write."
  (condition-case nil
      (let ((event (jaunder--debug-common-event label phase correlation span parent fields outcome elapsed)))
        (if-let* ((line (and event (jaunder--debug-format-event event))))
            (progn (jaunder--debug-append-line line) t)
          (jaunder--debug-warning)
          nil))
    (error (jaunder--debug-warning) nil)
    (quit (jaunder--debug-warning) nil)))

(defun jaunder--debug-merge-fields (old new)
  "Merge NEW validated optional fields into OLD without losing earlier fields."
  (while new
    (setq old (plist-put old (pop new) (pop new))))
  old)

(defun jaunder--debug-fields-internal (fields)
  "Add validated terminal FIELDS to the current operation span."
  (when jaunder--debug-operation-stack
    (let ((safe (jaunder--debug-normalize-fields fields)))
      (unless (or safe (null fields)) (error "invalid diagnostic fields"))
      (let ((span (car jaunder--debug-operation-stack)))
        (setf (jaunder--debug-span-fields span)
              (jaunder--debug-merge-fields (jaunder--debug-span-fields span) safe))))))

(defun jaunder--debug-begin (label field-thunk)
  "Start LABEL using FIELD-THUNK, returning state only after a logged start."
  (let* ((fields (funcall field-thunk))
         (parent-state (car jaunder--debug-operation-stack))
         (correlation (if parent-state (jaunder--debug-span-correlation parent-state)
                        (jaunder--debug-next-id)))
         (span (jaunder--debug-next-id))
         (start (jaunder--debug-now))
         (state (jaunder--make-debug-span
                 :label label :correlation correlation :id span
                 :parent (and parent-state (jaunder--debug-span-id parent-state))
                 :start start :fields fields)))
    (unless (or (null fields) (jaunder--debug-normalize-fields fields))
      (error "invalid diagnostic fields"))
    (when (jaunder--debug-write label "start" correlation span
                                (jaunder--debug-span-parent state) fields)
      (push state jaunder--debug-operation-stack)
      state)))

(defun jaunder--debug-finish (state)
  "Finish STATE without allowing terminal diagnostics to escape."
  (let ((elapsed (max 0 (floor (* 1000 (- (jaunder--debug-now)
                                          (jaunder--debug-span-start state)))))))
    (jaunder--debug-write (jaunder--debug-span-label state) "end"
                          (jaunder--debug-span-correlation state)
                          (jaunder--debug-span-id state)
                          (jaunder--debug-span-parent state)
                          (jaunder--debug-span-fields state)
                          (jaunder--debug-span-outcome state) elapsed)))

(defun jaunder--debug-field-form (fields)
  "Construct a lazy plist form from alternating literal keys and value forms."
  (let (result)
    (while fields
      (let ((key (pop fields)) (value (pop fields)))
        (setq result (append result (list `(quote ,key) value)))))
    `(list ,@result)))

(defmacro jaunder--debug-fields (&rest fields)
  "Lazily add terminal field forms to the current diagnostic operation."
  `(when jaunder-debug
     (jaunder--debug-safe
      (lambda () (jaunder--debug-fields-internal ,(jaunder--debug-field-form fields))))))

(defmacro jaunder--with-debug-operation (label fields &rest body)
  "Run BODY once with an optional diagnostic span for literal LABEL and FIELDS."
  (declare (indent 2) (debug (form form body)))
  (unless (stringp label) (error "Diagnostic labels must be literal strings"))
  (let ((state (make-symbol "state")))
    `(if (not jaunder-debug)
         (progn ,@body)
       (let ((jaunder--debug-operation-stack jaunder--debug-operation-stack)
             ,state)
         (unwind-protect
             (condition-case err
                 (progn
                   ;; Install state and cleanup before releasing a pending user quit.
                   (let ((inhibit-quit t))
                     (setq ,state
                           (jaunder--debug-safe
                            (lambda ()
                              (jaunder--debug-begin
                               ,label (lambda () ,(jaunder--debug-field-form fields))))))
                     ;; Failed setup cannot lend an ancestor's field-update target.
                     (unless ,state (setq jaunder--debug-operation-stack nil)))
                   ,@body)
               (quit (when ,state (setf (jaunder--debug-span-outcome ,state) "cancelled"))
                     (signal (car err) (cdr err)))
               (error (when ,state (setf (jaunder--debug-span-outcome ,state) "error"))
                      (signal (car err) (cdr err))))
           (when ,state
             (unwind-protect
                 (when jaunder-debug
                   (jaunder--debug-safe (lambda () (jaunder--debug-finish ,state))))
               (setq jaunder--debug-operation-stack
                     (delq ,state jaunder--debug-operation-stack)))))))))

(defun jaunder-debug-show ()
  "Show the retained diagnostic buffer without changing its contents."
  (interactive)
  (pop-to-buffer (jaunder--debug-buffer)))

(defun jaunder-debug-clear ()
  "Clear an existing diagnostic buffer without creating one."
  (interactive)
  (setq jaunder--debug-event-count 0 jaunder--debug-discarded 0)
  (when-let* ((buffer (get-buffer jaunder--debug-buffer-name)))
    (with-current-buffer buffer
      (let ((inhibit-read-only t)) (erase-buffer)))))

(defun jaunder-debug-disable ()
  "Disable future diagnostics while retaining existing evidence."
  (interactive)
  (setq jaunder-debug nil))

(provide 'jaunder-debug)
;;; jaunder-debug.el ends here
