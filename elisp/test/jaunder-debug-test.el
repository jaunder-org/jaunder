;;; jaunder-debug-test.el --- Tests for bounded diagnostics -*- lexical-binding: t; -*-

;; Copyright (C) 2026 Jaunder contributors

;;; Commentary:
;; Isolated core contract proofs for lazy disabled diagnostics, safe field/event
;; validation, bounded retention, operation correlation, and failure isolation.

;;; Code:

(require 'ert)
(require 'cl-lib)
(require 'jaunder)

(load (expand-file-name "jaunder-debug-boundary-fixture.el"
                        (file-name-directory (or load-file-name buffer-file-name))) nil t)

(defmacro jaunder-debug-test--deftest (name args doc &rest body)
  "Register NAME with ARGS, DOC and BODY isolated from the client session."
  (declare (indent defun) (debug defun))
  `(ert-deftest ,name ,args ,doc
                (jaunder-debug-boundary--with-session ,@body)))

(jaunder-debug-test--deftest jaunder-debug-disabled-bypasses-every-diagnostic-route ()
                             "Disabled forms do not evaluate fields or reach diagnostic state or output."
                             (let ((body 0))
                               (cl-letf (((symbol-function 'jaunder--debug-now) (lambda () (error "clock")))
                                         ((symbol-function 'jaunder--debug-format-event) (lambda (_) (error "format")))
                                         ((symbol-function 'jaunder--debug-buffer) (lambda () (error "buffer")))
                                         ((symbol-function 'jaunder--debug-next-id) (lambda () (error "id"))))
                                 (should (equal 'result
                                                (jaunder--with-debug-operation "config.resolve"
                                                                               (count (error "field"))
                                                                               (jaunder--debug-fields count (error "terminal field"))
                                                                               (setq body (1+ body))
                                                                               'result))))
                               (should (= body 1))
                               (should (= jaunder--debug-id-counter 0))
                               (should-not jaunder--debug-operation-stack)
                               (should-not (get-buffer jaunder--debug-buffer-name))))

(jaunder-debug-test--deftest jaunder-debug-disable-mid-span-stops-terminal-diagnostics ()
                             "Disabling in BODY retains the start but performs no terminal diagnostic work."
                             (let ((jaunder-debug t) (formats 0))
                               (let ((original (symbol-function 'jaunder--debug-format-event)))
                                 (cl-letf (((symbol-function 'jaunder--debug-format-event)
                                            (lambda (event) (setq formats (1+ formats)) (funcall original event))))
                                   (jaunder--with-debug-operation "config.resolve" ()
                                                                  (jaunder-debug-disable)
                                                                  'result)))
                               (should (= formats 1))
                               (should-not jaunder-debug)
                               (should-not jaunder--debug-operation-stack)
                               (should (= 1 jaunder--debug-event-count))))

(jaunder-debug-test--deftest jaunder-debug-elapsed-clock-is-nonnegative-and-saturates ()
                             "Backward and enormous terminal clocks produce bounded elapsed milliseconds."
                             (let ((jaunder-debug t) (times '(10 9)))
                               (cl-letf (((symbol-function 'jaunder--debug-now) (lambda () (pop times)))
                                         ((symbol-function 'jaunder--debug-timestamp)
                                          (lambda () "2026-10-07T12:00:00.123Z")))
                                 (jaunder--with-debug-operation "config.resolve" () nil))
                               (should (string-match-p "elapsed-ms=0" (jaunder-debug-boundary--text))))
                             (jaunder-debug-clear)
                             (let ((jaunder-debug t)
                                   (times (list 0 (/ (+ jaunder--debug-maximum-number 1000000) 1000.0))))
                               (cl-letf (((symbol-function 'jaunder--debug-now) (lambda () (pop times)))
                                         ((symbol-function 'jaunder--debug-timestamp)
                                          (lambda () "2026-10-07T12:00:00.123Z")))
                                 (jaunder--with-debug-operation "config.resolve" () nil))
                               (should (string-match-p (format "elapsed-ms=%d" jaunder--debug-maximum-number)
                                                       (jaunder-debug-boundary--text)))))

(jaunder-debug-test--deftest jaunder-debug-timestamp-has-a-real-nonzero-fraction ()
                             "Timestamp milliseconds derive from the diagnostic clock rather than a constant."
                             (cl-letf (((symbol-function 'jaunder--debug-now) (lambda () 1700000000.789)))
                               (should (string-match-p "\\.789Z\\'" (jaunder--debug-timestamp)))))

(jaunder-debug-test--deftest jaunder-debug-fields-are-lazy-merged-and-noninterfering ()
                             "Terminal fields retain initial fields, merge updates, and swallow failures."
                             (setq jaunder-debug t)
                             (jaunder--with-debug-operation "config.resolve" (method "GET" count 1)
                                                            (jaunder--debug-fields count 2 decision "blocked")
                                                            (jaunder--debug-fields members 3)
                                                            (jaunder--debug-fields count (error "private terminal field"))
                                                            'value)
                             (let ((text (jaunder-debug-boundary--text)))
                               (should (string-match-p "method=GET" text))
                               (should (string-match-p "count=2" text))
                               (should (string-match-p "decision=blocked" text))
                               (should (string-match-p "members=3" text))
                               (should-not (string-match-p "private terminal field" text))))

(jaunder-debug-test--deftest jaunder-debug-pairs-exact-root-child-and-standalone-ids ()
                             "Nested spans inherit correlation while a later span receives a new root ID."
                             (setq jaunder-debug t)
                             (cl-letf (((symbol-function 'jaunder--debug-timestamp) (lambda () "2026-10-07T12:00:00.123Z")))
                               (jaunder--with-debug-operation "config.resolve" ()
                                                              (jaunder--with-debug-operation "auth.lookup" () nil))
                               (jaunder--with-debug-operation "config.resolve" () nil))
                             (let ((text (jaunder-debug-boundary--text)))
                               (should (string-match-p "correlation=debug-1 span=debug-2 label=config.resolve phase=start" text))
                               (should (string-match-p "correlation=debug-1 span=debug-3 label=auth.lookup phase=start parent=debug-2" text))
                               (should (string-match-p "span=debug-3 label=auth.lookup phase=end parent=debug-2" text))
                               (should (string-match-p "correlation=debug-4 span=debug-5 label=config.resolve phase=start" text))))

(jaunder-debug-test--deftest jaunder-debug-real-maximum-retention-bound ()
                             "The production 10,000-event bound retains one marker plus newest events."
                             (dotimes (number (1+ jaunder--debug-maximum-events))
                               (jaunder--debug-write "config.resolve" "start" "debug-1"
                                                     (format "debug-%d" (+ number 2)) nil nil))
                             (with-current-buffer jaunder--debug-buffer-name
                               (should (= jaunder--debug-maximum-events jaunder--debug-event-count))
                               (should (= 1 jaunder--debug-discarded))
                               (should (string-prefix-p "evicted=1\n" (buffer-string)))
                               (should (= (1+ jaunder--debug-maximum-events)
                                          (count-lines (point-min) (point-max))))))

(defconst jaunder-debug-test--labels
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

(defconst jaunder-debug-test--enums
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

(defvar jaunder-debug-test--warnings nil)

(defmacro jaunder-debug-test--with-enabled-session (&rest body)
  "Run BODY with enabled diagnostics and an isolated warning recorder."
  (declare (indent 0) (debug body))
  `(jaunder-debug-boundary--with-session
    (let ((jaunder-debug t)
          (jaunder-debug-test--warnings nil))
      (cl-letf (((symbol-function 'display-warning)
                 (lambda (_type text &rest _)
                   (push text jaunder-debug-test--warnings))))
        ,@body))))

(defun jaunder-debug-test--emit-field (key value)
  "Exercise literal KEY and quoted VALUE through the real operation macro."
  (eval (list 'jaunder--with-debug-operation "config.resolve"
              (list key (list 'quote value)) nil) t))

(ert-deftest jaunder-debug-expanded-label-registry ()
  "The complete spec label registry is accepted, with no extra labels."
  (jaunder-debug-test--with-enabled-session
   (should (equal (sort (copy-sequence jaunder-debug-test--labels) #'string<)
                  (sort (copy-sequence jaunder--debug-labels) #'string<)))
   (dolist (label jaunder-debug-test--labels)
     (jaunder-debug-clear)
     (eval (list 'jaunder--with-debug-operation label nil nil) t)
     (let ((lines (split-string (jaunder-debug-boundary--text) "\n" t)))
       (should (= 2 (length lines)))
       (should (string-match-p (regexp-quote (concat "label=" label " phase=start"))
                               (car lines)))
       (should (string-match-p (regexp-quote (concat "label=" label " phase=end"))
                               (cadr lines)))))
   (should-not jaunder-debug-test--warnings)))

(ert-deftest jaunder-debug-labels-are-static-literals ()
  "Dynamic label forms are rejected without evaluating payload or fields."
  (dolist (label '(label (concat "config." "resolve") (error "PRIVATE-LABEL")))
    (should-error
     (macroexpand-1 (list 'jaunder--with-debug-operation label
                          '(count (error "PRIVATE-FIELD")) 'body))
     :type 'error))
  (should (macroexpand-1 '(jaunder--with-debug-operation "config.resolve" () nil))))

(ert-deftest jaunder-debug-serialized-optional-vocabulary ()
  "Every enum, numeric and boolean boundary reaches only safe serialized text."
  (jaunder-debug-test--with-enabled-session
   (should (equal jaunder-debug-test--enums jaunder--debug-enums))
   (dolist (entry jaunder-debug-test--enums)
     (dolist (value (append (cdr entry) '("unknown" "PRIVATE-CREDENTIAL\ncontent")))
       (jaunder-debug-clear)
       (jaunder-debug-test--emit-field (car entry) value)
       (let ((expected (if (member value (cdr entry)) value "unknown")))
         (should (string-match-p
                  (regexp-quote (format "%s=%s" (car entry) expected))
                  (jaunder-debug-boundary--text)))
         (should-not (string-match-p "PRIVATE-CREDENTIAL" (jaunder-debug-boundary--text))))))
   (dolist (key '(count bytes page members))
     (dolist (value '(0 9007199254740991 9007199254740992))
       (jaunder-debug-clear)
       (jaunder-debug-test--emit-field key value)
       (should (string-match-p
                (regexp-quote (format "%s=%d" key (min value 9007199254740991)))
                (jaunder-debug-boundary--text)))))
   (dolist (value '(100 599))
     (jaunder-debug-clear)
     (jaunder-debug-test--emit-field 'http-status value)
     (should (string-match-p (format "http-status=%d" value)
                             (jaunder-debug-boundary--text))))
   (dolist (value '(nil t))
     (jaunder-debug-clear)
     (jaunder-debug-test--emit-field 'reused value)
     (should (string-match-p (format "reused=%s" value)
                             (jaunder-debug-boundary--text))))
   (should-not jaunder-debug-test--warnings)))

(ert-deftest jaunder-debug-rejects-optional-values-before-encoding ()
  "Unknown keys, reserved keys, malformed fields and wrong types never encode."
  (jaunder-debug-test--with-enabled-session
   (let ((fields '((path "PRIVATE-PATH") (count) (count . "PRIVATE-PAYLOAD"))))
     (dolist (key '(count bytes page members))
       (dolist (value '(-1 1.5 "PRIVATE-NUMBER"))
         (push (list key value) fields)))
     (dolist (key '(method format action eligibility decision reason))
       (push (list key '(PRIVATE-ENUM)) fields))
     (dolist (key '(at correlation span parent label phase elapsed-ms outcome))
       (push (list key "PRIVATE-COMMON") fields))
     (dolist (field '((at "2026-10-07T12:00:00.123Z")
                      (correlation "debug-1") (span "debug-2") (parent "debug-3")
                      (label "config.resolve") (phase "start")
                      (elapsed-ms 1) (outcome "success")))
       (push field fields))
     (dolist (value '(99 600 "PRIVATE-STATUS" nil))
       (push (list 'http-status value) fields))
     (push '(reused PRIVATE-BOOLEAN) fields)
     (dolist (field fields)
       (let ((formatted nil)
             (warnings-before (length jaunder-debug-test--warnings)))
         (cl-letf (((symbol-function 'jaunder--debug-format-event)
                    (lambda (_) (setq formatted t) "PRIVATE-ENCODED")))
           (should-not (jaunder--debug-write "config.resolve" "start"
                                             "debug-1" "debug-2" nil field)))
         (should-not formatted)
         (should (= (1+ warnings-before) (length jaunder-debug-test--warnings))))))
   (should-not (get-buffer jaunder--debug-buffer-name))
   (should (cl-every (lambda (text) (equal text "jaunder: diagnostic output unavailable"))
                     jaunder-debug-test--warnings))))

(ert-deftest jaunder-debug-common-values-and-timestamps ()
  "All ID positions and timestamp components are validated before encoding."
  (jaunder-debug-test--with-enabled-session
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
     (let ((formatted nil))
       (cl-letf (((symbol-function 'jaunder--debug-format-event)
                  (lambda (_) (setq formatted t) "PRIVATE")))
         (should-not (apply #'jaunder--debug-write args)))
       (should-not formatted)))
   (should-not (get-buffer jaunder--debug-buffer-name))
   (should (jaunder--debug-write "config.resolve" "start" (make-string 32 ?a)
                                 (make-string 32 ?b) (make-string 32 ?c) nil))))

(ert-deftest jaunder-debug-byte-limits-and-eviction-saturation ()
  "Event and marker limits exclude newline; eviction counts saturate exactly."
  (jaunder-debug-test--with-enabled-session
   (should (= 1024 (string-bytes (jaunder--debug-format-event
                                  (list (cons 'x (make-string 1022 ?a)))))))
   (should-not (jaunder--debug-format-event (list (cons 'x (make-string 1023 ?a)))))
   (should-not (jaunder--debug-format-event '((x . "é"))))
   (let ((jaunder--debug-maximum-events 2))
     (dotimes (number 4)
       (jaunder--debug-write "config.resolve" "start" "debug-1"
                             (format "debug-%d" (+ number 2)) nil nil))
     (let ((lines (split-string (jaunder-debug-boundary--text) "\n" t)))
       (should (equal "evicted=2" (car lines)))
       (should (= 3 (length lines)))
       (should (string-match-p "span=debug-4 " (nth 1 lines)))
       (should (string-match-p "span=debug-5 " (nth 2 lines))))
     (setq jaunder--debug-discarded 9007199254740991)
     (jaunder--debug-write "config.resolve" "start" "debug-1" "debug-6" nil nil)
     (let ((marker (car (split-string (jaunder-debug-boundary--text) "\n" t))))
       (should (equal marker "evicted=9007199254740991"))
       (should (<= (string-bytes marker) 128))
       (should (string-match-p "\\`[[:ascii:]]*\\'" marker))))))

(define-error 'jaunder-debug-error "Controlled business failure")

(ert-deftest jaunder-debug-failure-matrix-preserves-primary-outcomes ()
  "Diagnostic errors and quits at every stage preserve all primary outcomes."
  (jaunder-debug-test--with-enabled-session
   (dolist (stage '(id initial update begin-clock end-clock begin-append end-append encoder))
     (dolist (diagnostic-condition '(error quit))
       (dolist (warning-mode '(working warning-fails both-fail))
         (dolist (primary '(return error quit))
           (jaunder-debug-clear)
           (let* ((token (list 'unique-object))
                  (data (list "PRIVATE-BUSINESS" token))
                  (runs 0) (clocks 0) (appends 0) warning-texts fallback-texts
                  (append-line (symbol-function 'jaunder--debug-append-line))
                  (next-id (symbol-function 'jaunder--debug-next-id))
                  (actual nil)
                  (fault (lambda () (signal diagnostic-condition '("PRIVATE-DIAGNOSTIC")))))
             (cl-letf (((symbol-function 'jaunder--debug-next-id)
                        (lambda ()
                          (when (eq stage 'id) (funcall fault))
                          (funcall next-id)))
                       ((symbol-function 'jaunder--debug-timestamp)
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
                                                              ('error (signal 'jaunder-debug-error data))
                                                              ('quit (signal 'quit data))))
                           (jaunder-debug-error condition)
                           (quit condition))))))
             (should (= runs 1))
             (if (eq primary 'return)
                 (should (eq actual token))
               (should (equal actual (cons (if (eq primary 'quit) 'quit
                                             'jaunder-debug-error) data)))
               (should (eq (cdr actual) data))
               (should (eq (nth 2 actual) token)))
             (should-not jaunder--debug-operation-stack)
             (should warning-texts)
             (should (cl-every (lambda (text) (equal text "jaunder: diagnostic output unavailable"))
                               (append warning-texts fallback-texts)))
             (unless (eq warning-mode 'working) (should fallback-texts))
             (should-not (string-match-p "PRIVATE" (jaunder-debug-boundary--text)))
             (jaunder-debug-clear)
             (jaunder--with-debug-operation "config.resolve" () nil)
             (let ((text (jaunder-debug-boundary--text)))
               (should-not (string-match-p "parent=" text))
               (should (= 2 (length (split-string text "\n" t))))))))))))

(ert-deftest jaunder-debug-failed-child-cannot-update-parent-fields ()
  "An unlogged child cannot attach its terminal fields to an ancestor span."
  (jaunder-debug-test--with-enabled-session
   (jaunder--with-debug-operation "config.resolve" (count 1)
                                  (jaunder--with-debug-operation "auth.lookup" (count (error "PRIVATE-FIELD"))
                                                                 (jaunder--debug-fields count 99 members 7)))
   (let ((text (jaunder-debug-boundary--text)))
     (should (= 2 (length (split-string text "\n" t))))
     (should-not (string-match-p "count=99\\|members=7" text))
     (should (string-match-p "outcome=success count=1" text)))
   (should-not jaunder--debug-operation-stack)))

(ert-deftest jaunder-debug-eviction-preserves-caller-match-data ()
  "Marker updates cannot disturb regular-expression evidence used by BODY."
  (jaunder-debug-test--with-enabled-session
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

(ert-deftest jaunder-debug-pending-user-quit-stops-the-operation ()
  "Pending keyboard cancellation is deferred through logging, never consumed."
  (dolist (stage '(clock initial update terminal-clock terminal-write))
    (jaunder-debug-test--with-enabled-session
     (let ((quit-flag nil) (debug-on-quit nil) (runs 0) (samples 0) (writes 0)
           (append-line (symbol-function 'jaunder--debug-append-line)))
       (cl-letf (((symbol-function 'jaunder--debug-timestamp)
                  (lambda () "2026-10-07T12:00:00.123Z"))
                 ((symbol-function 'jaunder--debug-now)
                  (lambda ()
                    (setq samples (1+ samples))
                    (when (or (and (eq stage 'clock) (= samples 1))
                              (and (eq stage 'terminal-clock) (= samples 2)))
                      (setq quit-flag t))
                    10.0))
                 ((symbol-function 'jaunder--debug-append-line)
                  (lambda (line)
                    (funcall append-line line)
                    (setq writes (1+ writes))
                    (when (and (eq stage 'terminal-write) (= writes 2))
                      (setq quit-flag t)))))
         (let ((condition
                (condition-case condition
                    (jaunder--with-debug-operation "config.resolve"
                                                   (count (progn (when (eq stage 'initial) (setq quit-flag t)) 1))
                                                   (setq runs (1+ runs))
                                                   (jaunder--debug-fields count (progn (when (eq stage 'update) (setq quit-flag t)) 2))
                                                   (setq runs (1+ runs)))
                  (quit condition))))
           (should (eq (car-safe condition) 'quit))))
       (should (= runs (pcase stage ('update 1) ((or 'terminal-clock 'terminal-write) 2) (_ 0))))
       (should-not jaunder--debug-operation-stack)
       (should-not jaunder-debug-test--warnings)
       (let ((text (jaunder-debug-boundary--text)))
         (should (= 2 (length (split-string text "\n" t))))
         (should (string-match-p "outcome=cancelled" text)))))))

(ert-deftest jaunder-debug-terminal-cancellation-unwinds-nested-spans ()
  "A child terminal quit cancels both spans without updating parent fields."
  (jaunder-debug-test--with-enabled-session
   (let ((append-line (symbol-function 'jaunder--debug-append-line))
         (writes 0) (quit-flag nil))
     (cl-letf (((symbol-function 'jaunder--debug-append-line)
                (lambda (line)
                  (funcall append-line line)
                  (setq writes (1+ writes))
                  (when (= writes 3) (setq quit-flag t)))))
       (should (equal '(quit)
                      (condition-case condition
                          (jaunder--with-debug-operation "config.resolve" (count 7)
                                                         (jaunder--with-debug-operation "atom.serialize" () 'result)
                                                         (jaunder--debug-fields count 99))
                        (quit condition)))))
     (should-not jaunder--debug-operation-stack)
     (should-not jaunder-debug-test--warnings)
     (let ((text (jaunder-debug-boundary--text)))
       (should (= 4 (length (split-string text "\n" t))))
       (should (= 2 (cl-count-if (lambda (line) (string-match-p "outcome=cancelled" line))
                                 (split-string text "\n" t))))
       (should-not (string-match-p "count=99" text))))))

(ert-deftest jaunder-debug-view-and-buffer-lifetime ()
  "Emission never displays; q buries evidence; clear and recreation preserve IDs."
  (jaunder-debug-test--with-enabled-session
   (cl-letf (((symbol-function 'pop-to-buffer) (lambda (&rest _) (ert-fail "automatic pop")))
             ((symbol-function 'display-buffer) (lambda (&rest _) (ert-fail "automatic display")))
             ((symbol-function 'switch-to-buffer) (lambda (&rest _) (ert-fail "automatic switch"))))
     (jaunder--with-debug-operation "config.resolve" () nil))
   (let ((text (jaunder-debug-boundary--text)))
     (save-window-excursion
       (jaunder-debug-show)
       (should (eq (current-buffer) (get-buffer jaunder--debug-buffer-name)))
       (should buffer-read-only)
       (should (derived-mode-p 'jaunder-debug-mode))
       (call-interactively (key-binding (kbd "q")))
       (should (get-buffer jaunder--debug-buffer-name))
       (should-not (get-buffer-window jaunder--debug-buffer-name))
       (should (equal text (jaunder-debug-boundary--text)))))
   (let ((last-id jaunder--debug-id-counter))
     (jaunder-debug-clear)
     (should jaunder-debug)
     (should (equal "" (jaunder-debug-boundary--text)))
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
   (let ((text (jaunder-debug-boundary--text)))
     (jaunder--with-debug-operation "config.resolve" () nil)
     (should (equal text (jaunder-debug-boundary--text))))
   (jaunder-debug-clear)
   (should-not jaunder-debug)
   (should (= 0 jaunder--debug-event-count))
   (should (= 0 jaunder--debug-discarded))
   (kill-buffer jaunder--debug-buffer-name)
   (jaunder-debug-clear)
   (should-not (get-buffer jaunder--debug-buffer-name))
   (save-window-excursion
     (jaunder-debug-show)
     (should (eq (current-buffer) (get-buffer jaunder--debug-buffer-name)))
     (should buffer-read-only)
     (should (derived-mode-p 'jaunder-debug-mode))
     (should (equal "" (jaunder-debug-boundary--text)))
     (should (eq (key-binding (kbd "q")) 'quit-window))
     (call-interactively (key-binding (kbd "q"))))))

(ert-deftest jaunder-debug-invalid-initial-fields-preserve-body-and-ancestor ()
  "Rejected initial fields do not skip native work or target an ancestor."
  (jaunder-debug-boundary--with-session
   (let ((jaunder-debug t) (native (list "private native value")) (warnings 0))
     (cl-letf (((symbol-function 'display-warning)
                (lambda (type message &rest _)
                  (should (eq type 'jaunder))
                  (should (equal message "jaunder: diagnostic output unavailable"))
                  (cl-incf warnings))))
       (jaunder--with-debug-operation "report.open" (count 1)
                                      (should (eq native
                                                  (jaunder--with-debug-operation "publish.validate"
                                                                                 (count "private invalid count")
                                                                                 (jaunder--debug-fields count 9)
                                                                                 native)))))
     (should (= warnings 1))
     (let ((text (jaunder-debug-boundary--text)))
       (jaunder-debug-boundary--assert-tree text '("report.open"))
       (should (string-match-p "count=1" text))
       (should-not (string-match-p "count=9\\|private\\|publish.validate" text))))))

(ert-deftest jaunder-debug-oversize-terminal-rewrite-is-contained ()
  "A damaged retained line cannot grow past the event bound during cancellation."
  (jaunder-debug-boundary--with-session
   (let* ((jaunder-debug t) (warnings 0)
          (state (jaunder--make-debug-span :label "report.open" :id "debug-2"))
          (line "at=2026-10-07T12:00:00.000Z correlation=debug-1 span=debug-2 label=report.open phase=end elapsed-ms=0 outcome=success"))
     ;; Model a formatter/sink fault at the supported maximum. Changing success
     ;; to cancelled would add two bytes; the retained evidence must stay put.
     (setq line (concat line (make-string (- 1024 (string-bytes line)) ?\s)))
     (with-current-buffer (jaunder--debug-buffer)
       (let ((inhibit-read-only t)) (insert line "\n")))
     (cl-letf (((symbol-function 'display-warning)
                (lambda (type message &rest _)
                  (should (eq type 'jaunder))
                  (should (equal message "jaunder: diagnostic output unavailable"))
                  (cl-incf warnings))))
       (should-not (jaunder--debug-safe
                    (lambda () (jaunder--debug-cancel-terminal state)))))
     (should (= warnings 1))
     (should (equal (concat line "\n") (jaunder-debug-boundary--text))))))

(provide 'jaunder-debug-test)
;;; jaunder-debug-test.el ends here
