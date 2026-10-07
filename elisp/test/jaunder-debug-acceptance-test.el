;;; jaunder-debug-acceptance-test.el --- Diagnostic contract proofs -*- lexical-binding: t; -*-

;; Copyright (C) 2026 Jaunder contributors

;;; Commentary:
;; Independent contract fixtures exercise the writer and macro callers.  Each
;; proof owns its buffer and dynamically binds all diagnostic session state.
;; Actual feature-boundary evidence belongs to the instrumentation tests.

;;; Code:

(require 'ert)
(require 'cl-lib)
(require 'jaunder-debug)

(defconst jaunder-debug-acceptance--labels
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

(defconst jaunder-debug-acceptance--enums
  '((method "GET" "HEAD" "POST" "PUT" "DELETE")
    (format "org" "markdown" "html" "atom")
    (action "new" "complete" "cancel" "discard" "publish" "draft" "create"
            "recover" "update" "push" "pull" "keep-local" "keep-remote"
            "merge" "delete" "refresh")
    (eligibility "eligible" "ineligible")
    (decision "proceed" "blocked" "no-op" "retry" "recovered" "partial"
              "remote-unknown")
    (reason "invalid" "stale" "ambiguous" "ineligible" "missing" "unsafe-path"
            "modified" "conflict" "transport" "decode" "io" "partial"
            "remote-unknown" "unexpected")))

(defvar jaunder-debug-acceptance--warnings nil)

(defmacro jaunder-debug-acceptance--with-state (&rest body)
  "Run BODY with isolated enabled diagnostics and a fixed warning recorder."
  (declare (indent 0) (debug body))
  `(let ((jaunder-debug t)
         (jaunder--debug-buffer-name " *Jaunder diagnostic acceptance*")
         (jaunder--debug-id-counter 0)
         (jaunder--debug-event-count 0)
         (jaunder--debug-discarded 0)
         (jaunder--debug-operation-stack nil)
         (jaunder-debug-acceptance--warnings nil))
     (unwind-protect
         (cl-letf (((symbol-function 'display-warning)
                    (lambda (_type text &rest _)
                      (push text jaunder-debug-acceptance--warnings))))
           ,@body)
       (when-let* ((buffer (get-buffer jaunder--debug-buffer-name)))
         (kill-buffer buffer)))))

(defun jaunder-debug-acceptance--text ()
  "Return this proof's retained diagnostic text, or an empty string."
  (if-let* ((buffer (get-buffer jaunder--debug-buffer-name)))
      (with-current-buffer buffer (buffer-string))
    ""))

(defun jaunder-debug-acceptance--emit-field (key value)
  "Exercise literal KEY and quoted VALUE through the real operation macro."
  (eval (list 'jaunder--with-debug-operation "config.resolve"
              (list key (list 'quote value)) nil) t))

(ert-deftest jaunder-debug-acceptance-expanded-label-registry ()
  "The complete spec label registry is accepted, with no extra labels."
  (jaunder-debug-acceptance--with-state
   (should (equal (sort (copy-sequence jaunder-debug-acceptance--labels) #'string<)
                  (sort (copy-sequence jaunder--debug-labels) #'string<)))
   (dolist (label jaunder-debug-acceptance--labels)
     (jaunder-debug-clear)
     (eval (list 'jaunder--with-debug-operation label nil nil) t)
     (let ((lines (split-string (jaunder-debug-acceptance--text) "\n" t)))
       (should (= 2 (length lines)))
       (should (string-match-p (regexp-quote (concat "label=" label " phase=start"))
                               (car lines)))
       (should (string-match-p (regexp-quote (concat "label=" label " phase=end"))
                               (cadr lines)))))
   (should-not jaunder-debug-acceptance--warnings)))

(ert-deftest jaunder-debug-acceptance-labels-are-static-literals ()
  "Dynamic label forms are rejected without evaluating payload or fields."
  (dolist (label '(label (concat "config." "resolve") (error "PRIVATE-LABEL")))
    (should-error
     (macroexpand-1 (list 'jaunder--with-debug-operation label
                          '(count (error "PRIVATE-FIELD")) 'body))
     :type 'error))
  (should (macroexpand-1 '(jaunder--with-debug-operation "config.resolve" () nil))))

(ert-deftest jaunder-debug-acceptance-serialized-optional-vocabulary ()
  "Every enum, numeric and boolean boundary reaches only safe serialized text."
  (jaunder-debug-acceptance--with-state
   (should (equal jaunder-debug-acceptance--enums jaunder--debug-enums))
   (dolist (entry jaunder-debug-acceptance--enums)
     (dolist (value (append (cdr entry) '("unknown" "PRIVATE-CREDENTIAL\ncontent")))
       (jaunder-debug-clear)
       (jaunder-debug-acceptance--emit-field (car entry) value)
       (let ((expected (if (member value (cdr entry)) value "unknown")))
         (should (string-match-p
                  (regexp-quote (format "%s=%s" (car entry) expected))
                  (jaunder-debug-acceptance--text)))
         (should-not (string-match-p "PRIVATE-CREDENTIAL" (jaunder-debug-acceptance--text))))))
   (dolist (key '(count bytes page members))
     (dolist (value '(0 9007199254740991 9007199254740992))
       (jaunder-debug-clear)
       (jaunder-debug-acceptance--emit-field key value)
       (should (string-match-p
                (regexp-quote (format "%s=%d" key (min value 9007199254740991)))
                (jaunder-debug-acceptance--text)))))
   (dolist (value '(100 599))
     (jaunder-debug-clear)
     (jaunder-debug-acceptance--emit-field 'http-status value)
     (should (string-match-p (format "http-status=%d" value)
                             (jaunder-debug-acceptance--text))))
   (dolist (value '(nil t))
     (jaunder-debug-clear)
     (jaunder-debug-acceptance--emit-field 'reused value)
     (should (string-match-p (format "reused=%s" value)
                             (jaunder-debug-acceptance--text))))
   (should-not jaunder-debug-acceptance--warnings)))

(ert-deftest jaunder-debug-acceptance-rejects-optional-values-before-encoding ()
  "Unknown keys, reserved keys, malformed fields and wrong types never encode."
  (jaunder-debug-acceptance--with-state
   (let ((fields '((path "PRIVATE-PATH") (count) (count . "PRIVATE-PAYLOAD"))))
     (dolist (key '(count bytes page members))
       (dolist (value '(-1 1.5 "PRIVATE-NUMBER"))
         (push (list key value) fields)))
     (dolist (key '(method format action eligibility decision reason))
       (push (list key '(PRIVATE-ENUM)) fields))
     (dolist (key '(at correlation span parent label phase elapsed-ms outcome))
       (push (list key "PRIVATE-COMMON") fields))
     (dolist (value '(99 600 "PRIVATE-STATUS" nil))
       (push (list 'http-status value) fields))
     (push '(reused PRIVATE-BOOLEAN) fields)
     (dolist (field fields)
       (let ((formatted nil)
             (warnings-before (length jaunder-debug-acceptance--warnings)))
         (cl-letf (((symbol-function 'jaunder--debug-format-event)
                    (lambda (_) (setq formatted t) "PRIVATE-ENCODED")))
           (should-not (jaunder--debug-write "config.resolve" "start"
                                             "debug-1" "debug-2" nil field)))
         (should-not formatted)
         (should (= (1+ warnings-before) (length jaunder-debug-acceptance--warnings))))))
   (should-not (get-buffer jaunder--debug-buffer-name))
   (should (cl-every (lambda (text) (equal text "jaunder: diagnostic output unavailable"))
                     jaunder-debug-acceptance--warnings))))

(ert-deftest jaunder-debug-acceptance-common-values-and-timestamps ()
  "All ID positions and timestamp components are validated before encoding."
  (jaunder-debug-acceptance--with-state
   (dolist (position '(2 3 4))
     (dolist (value (list "" (make-string 33 ?a) "UPPER" "PRIVATE/URL" "é" 42))
       (let ((args (list "config.resolve" "start" "debug-1" "debug-2" nil nil))
             (formatted nil))
         (setf (nth position args) value)
         (cl-letf (((symbol-function 'jaunder--debug-format-event)
                    (lambda (_) (setq formatted t) "PRIVATE")))
           (should-not (apply #'jaunder--debug-write args)))
         (should-not formatted))))
   (dolist (timestamp '("PRIVATE-TIME" "2026-10-07T::::::::.789Z"
                        "2026-99-99T99:99:99.000Z" "2026-10-07T12:00:00.12Z"))
     (let ((formatted nil))
       (cl-letf (((symbol-function 'jaunder--debug-timestamp) (lambda () timestamp))
                 ((symbol-function 'jaunder--debug-format-event)
                  (lambda (_) (setq formatted t) "PRIVATE")))
         (should-not (jaunder--debug-write "config.resolve" "start" "debug-1" "debug-2" nil nil)))
       (should-not formatted)))
   (dolist (args '(("PRIVATE-LABEL" "start" "debug-1" "debug-2" nil nil)
                   ("config.resolve" "PRIVATE-PHASE" "debug-1" "debug-2" nil nil)
                   ("config.resolve" "end" "debug-1" "debug-2" nil nil "PRIVATE-OUTCOME" 0)
                   ("config.resolve" "end" "debug-1" "debug-2" nil nil "success" -1)
                   ("config.resolve" "end" "debug-1" "debug-2" nil nil "success" 1.5)
                   ("config.resolve" "start" "debug-1" "debug-2" nil nil "success" 0)))
     (should-not (apply #'jaunder--debug-write args)))
   (should-not (get-buffer jaunder--debug-buffer-name))
   (should (jaunder--debug-write "config.resolve" "start" (make-string 32 ?a)
                                 (make-string 32 ?b) (make-string 32 ?c) nil))))

(ert-deftest jaunder-debug-acceptance-byte-limits-and-eviction-saturation ()
  "Event and marker limits exclude newline; eviction counts saturate exactly."
  (jaunder-debug-acceptance--with-state
   (should (= 1024 (string-bytes (jaunder--debug-format-event
                                  (list (cons 'x (make-string 1022 ?a)))))))
   (should-not (jaunder--debug-format-event (list (cons 'x (make-string 1023 ?a)))))
   (should-not (jaunder--debug-format-event '((x . "é"))))
   (let ((jaunder--debug-maximum-events 2))
     (dotimes (number 4)
       (jaunder--debug-write "config.resolve" "start" "debug-1"
                             (format "debug-%d" (+ number 2)) nil nil))
     (let ((lines (split-string (jaunder-debug-acceptance--text) "\n" t)))
       (should (equal "evicted=2" (car lines)))
       (should (= 3 (length lines)))
       (should (string-match-p "span=debug-4 " (nth 1 lines)))
       (should (string-match-p "span=debug-5 " (nth 2 lines))))
     (setq jaunder--debug-discarded 9007199254740991)
     (jaunder--debug-write "config.resolve" "start" "debug-1" "debug-6" nil nil)
     (let ((marker (car (split-string (jaunder-debug-acceptance--text) "\n" t))))
       (should (equal marker "evicted=9007199254740991"))
       (should (<= (string-bytes marker) 128))
       (should (string-match-p "\\`[[:ascii:]]*\\'" marker))))))

(define-error 'jaunder-debug-acceptance-error "Controlled business failure")

(ert-deftest jaunder-debug-acceptance-failure-matrix-preserves-primary-outcomes ()
  "Diagnostic errors and quits at every stage preserve all primary outcomes."
  (jaunder-debug-acceptance--with-state
   (dolist (stage '(initial update begin-clock end-clock begin-append end-append encoder))
     (dolist (diagnostic-condition '(error quit))
       (dolist (warning-mode '(working warning-fails both-fail))
         (dolist (primary '(return error quit))
           (jaunder-debug-clear)
           (let* ((token (list 'unique-object))
                  (data (list "PRIVATE-BUSINESS" token))
                  (runs 0) (clocks 0) (appends 0) warning-texts fallback-texts
                  (append-line (symbol-function 'jaunder--debug-append-line))
                  (actual nil)
                  (fault (lambda () (signal diagnostic-condition '("PRIVATE-DIAGNOSTIC")))))
             (cl-letf (((symbol-function 'jaunder--debug-timestamp)
                        (lambda () "2026-10-07T12:00:00.123Z"))
                       ((symbol-function 'jaunder--debug-now)
                        (lambda ()
                          (setq clocks (1+ clocks))
                          (when (or (and (eq stage 'begin-clock) (= clocks 1))
                                    (and (eq stage 'end-clock) (= clocks 2)))
                            (funcall fault))
                          10.0))
                       ((symbol-function 'jaunder--debug-append-line)
                        (lambda (line)
                          (setq appends (1+ appends))
                          (when (or (and (eq stage 'begin-append) (= appends 1))
                                    (and (eq stage 'end-append) (= appends 2)))
                            (funcall fault))
                          (funcall append-line line)))
                       ((symbol-function 'display-warning)
                        (lambda (_type text &rest _)
                          (push text warning-texts)
                          (unless (eq warning-mode 'working) (funcall fault))))
                       ((symbol-function 'message)
                        (lambda (text &rest _)
                          (push text fallback-texts)
                          (when (eq warning-mode 'both-fail) (funcall fault)))))
               (let ((formatter (symbol-function 'jaunder--debug-format-event)))
                 (cl-letf (((symbol-function 'jaunder--debug-format-event)
                            (lambda (event)
                              (when (eq stage 'encoder) (funcall fault))
                              (funcall formatter event))))
                   (setq actual
                         (condition-case condition
                             (jaunder--with-debug-operation "config.resolve"
                                                            (count (if (eq stage 'initial) (funcall fault) 1))
                                                            (setq runs (1+ runs))
                                                            (jaunder--debug-fields count (if (eq stage 'update) (funcall fault) 2))
                                                            (pcase primary
                                                              ('return token)
                                                              ('error (signal 'jaunder-debug-acceptance-error data))
                                                              ('quit (signal 'quit data))))
                           (jaunder-debug-acceptance-error condition)
                           (quit condition))))))
             (should (= runs 1))
             (if (eq primary 'return)
                 (should (eq actual token))
               (should (equal actual (cons (if (eq primary 'quit) 'quit
                                             'jaunder-debug-acceptance-error) data)))
               (should (eq (nth 2 actual) token)))
             (should-not jaunder--debug-operation-stack)
             (should warning-texts)
             (should (cl-every (lambda (text) (equal text "jaunder: diagnostic output unavailable"))
                               (append warning-texts fallback-texts)))
             (unless (eq warning-mode 'working) (should fallback-texts))
             (should-not (string-match-p "PRIVATE" (jaunder-debug-acceptance--text)))
             (jaunder-debug-clear)
             (jaunder--with-debug-operation "config.resolve" () nil)
             (let ((text (jaunder-debug-acceptance--text)))
               (should-not (string-match-p "parent=" text))
               (should (= 2 (length (split-string text "\n" t))))))))))))

(ert-deftest jaunder-debug-acceptance-failed-child-cannot-update-parent-fields ()
  "An unlogged child cannot attach its terminal fields to an ancestor span."
  (jaunder-debug-acceptance--with-state
   (jaunder--with-debug-operation "config.resolve" (count 1)
                                  (jaunder--with-debug-operation "auth.lookup" (count (error "PRIVATE-FIELD"))
                                                                 (jaunder--debug-fields count 99 members 7)))
   (let ((text (jaunder-debug-acceptance--text)))
     (should (= 2 (length (split-string text "\n" t))))
     (should-not (string-match-p "count=99\\|members=7" text))
     (should (string-match-p "outcome=success count=1" text)))
   (should-not jaunder--debug-operation-stack)))

(ert-deftest jaunder-debug-acceptance-eviction-preserves-caller-match-data ()
  "Marker updates cannot disturb regular-expression evidence used by BODY."
  (jaunder-debug-acceptance--with-state
   (let ((jaunder--debug-maximum-events 1)
         (source "prefix123"))
     (dotimes (number 2)
       (jaunder--debug-write "config.resolve" "start" "debug-1"
                             (format "seed-%d" number) nil nil))
     (string-match "\\([0-9]+\\)" source)
     (let ((before (match-data t)))
       (should (equal "123"
                      (jaunder--with-debug-operation "config.resolve" ()
                                                     (jaunder--debug-fields count 1)
                                                     (match-string 1 source))))
       (should (equal before (match-data t)))))))

(ert-deftest jaunder-debug-acceptance-pending-user-quit-stops-the-operation ()
  "Pending keyboard cancellation is deferred through logging, never consumed."
  (dolist (stage '(clock initial update))
    (jaunder-debug-acceptance--with-state
     (let ((quit-flag nil) (runs 0) (samples 0))
       (cl-letf (((symbol-function 'jaunder--debug-now)
                  (lambda ()
                    (setq samples (1+ samples))
                    (when (and (eq stage 'clock) (= samples 1)) (setq quit-flag t))
                    10.0)))
         (let ((condition
                (condition-case condition
                    (jaunder--with-debug-operation "config.resolve"
                                                   (count (progn (when (eq stage 'initial) (setq quit-flag t)) 1))
                                                   (setq runs (1+ runs))
                                                   (jaunder--debug-fields count (progn (when (eq stage 'update) (setq quit-flag t)) 2))
                                                   (setq runs (1+ runs)))
                  (quit condition))))
           (should (eq (car-safe condition) 'quit))))
       (should (= runs (if (eq stage 'update) 1 0)))
       (should-not jaunder--debug-operation-stack)
       (should-not jaunder-debug-acceptance--warnings)
       (let ((text (jaunder-debug-acceptance--text)))
         (should (= 2 (length (split-string text "\n" t))))
         (should (string-match-p "outcome=cancelled" text)))))))

(ert-deftest jaunder-debug-acceptance-view-and-buffer-lifetime ()
  "Emission never displays; q buries evidence; clear and recreation preserve IDs."
  (jaunder-debug-acceptance--with-state
   (cl-letf (((symbol-function 'pop-to-buffer) (lambda (&rest _) (ert-fail "automatic pop")))
             ((symbol-function 'display-buffer) (lambda (&rest _) (ert-fail "automatic display")))
             ((symbol-function 'switch-to-buffer) (lambda (&rest _) (ert-fail "automatic switch"))))
     (jaunder--with-debug-operation "config.resolve" () nil))
   (let ((text (jaunder-debug-acceptance--text)))
     (save-window-excursion
       (jaunder-debug-show)
       (should (eq (current-buffer) (get-buffer jaunder--debug-buffer-name)))
       (should buffer-read-only)
       (should (derived-mode-p 'special-mode))
       (call-interactively (key-binding (kbd "q")))
       (should (get-buffer jaunder--debug-buffer-name))
       (should-not (get-buffer-window jaunder--debug-buffer-name))
       (should (equal text (jaunder-debug-acceptance--text)))))
   (let ((last-id jaunder--debug-id-counter))
     (jaunder-debug-clear)
     (should jaunder-debug)
     (should (equal "" (jaunder-debug-acceptance--text)))
     (jaunder--with-debug-operation "config.resolve" () nil)
     (should (> jaunder--debug-id-counter last-id)))
   (setq jaunder--debug-discarded 77)
   (let ((last-id jaunder--debug-id-counter))
     (kill-buffer jaunder--debug-buffer-name)
     (jaunder--with-debug-operation "config.resolve" () nil)
     (should (> jaunder--debug-id-counter last-id))
     (should (= 0 jaunder--debug-discarded))
     (should (= 2 jaunder--debug-event-count)))
   (jaunder-debug-disable)
   (let ((text (jaunder-debug-acceptance--text)))
     (jaunder--with-debug-operation "config.resolve" () nil)
     (should (equal text (jaunder-debug-acceptance--text))))
   (jaunder-debug-clear)
   (should-not jaunder-debug)
   (should (= 0 jaunder--debug-event-count))
   (should (= 0 jaunder--debug-discarded))
   (kill-buffer jaunder--debug-buffer-name)
   (jaunder-debug-clear)
   (should-not (get-buffer jaunder--debug-buffer-name))))

(provide 'jaunder-debug-acceptance-test)
;;; jaunder-debug-acceptance-test.el ends here
