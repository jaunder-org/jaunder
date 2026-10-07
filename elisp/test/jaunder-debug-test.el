;;; jaunder-debug-test.el --- Tests for bounded diagnostics -*- lexical-binding: t; -*-

;; Copyright (C) 2026 Jaunder contributors

;;; Commentary:
;; Isolated core contract proofs for lazy disabled diagnostics, safe field/event
;; validation, bounded retention, operation correlation, and failure isolation.

;;; Code:

(require 'ert)
(require 'cl-lib)
(require 'jaunder)

(defmacro jaunder-debug-test--deftest (name args doc &rest body)
  "Register NAME with ARGS, DOC and BODY isolated from the client session."
  (declare (indent defun) (debug defun))
  `(ert-deftest ,name ,args ,doc
                (let ((jaunder-debug nil)
                      (jaunder--debug-buffer-name " *Jaunder diagnostic core tests*")
                      (jaunder--debug-id-counter 0)
                      (jaunder--debug-event-count 0)
                      (jaunder--debug-discarded 0)
                      (jaunder--debug-operation-stack nil))
                  (unwind-protect (progn ,@body)
                    (when-let* ((buffer (get-buffer jaunder--debug-buffer-name)))
                      (kill-buffer buffer))))))

(define-error 'jaunder-debug-test-business "Diagnostic test business condition")

(defconst jaunder-debug-test--enums
  '((method "GET" "HEAD" "POST" "PUT" "DELETE")
    (format "org" "markdown" "html" "atom")
    (action "new" "complete" "cancel" "discard" "publish" "draft" "create"
            "recover" "update" "push" "pull" "keep-local" "keep-remote" "merge"
            "delete" "refresh")
    (eligibility "eligible" "ineligible")
    (decision "proceed" "blocked" "no-op" "retry" "recovered" "partial"
              "remote-unknown")
    (reason "invalid" "stale" "ambiguous" "ineligible" "missing" "unsafe-path"
            "modified" "conflict" "transport" "decode" "io" "partial"
            "remote-unknown" "unexpected")))

(defun jaunder-debug-test--reset ()
  "Reset diagnostic state and remove the diagnostic buffer."
  (setq jaunder-debug nil
        jaunder--debug-id-counter 0
        jaunder--debug-event-count 0
        jaunder--debug-discarded 0
        jaunder--debug-operation-stack nil)
  (when-let* ((buffer (get-buffer jaunder--debug-buffer-name))) (kill-buffer buffer)))

(defun jaunder-debug-test--text ()
  "Return retained diagnostic text."
  (with-current-buffer jaunder--debug-buffer-name (buffer-string)))

(jaunder-debug-test--deftest jaunder-debug-disabled-bypasses-every-diagnostic-route ()
                             "Disabled forms do not evaluate fields or reach diagnostic state or output."
                             (jaunder-debug-test--reset)
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
                             (jaunder-debug-test--reset)
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
                             (jaunder-debug-test--reset)
                             (let ((jaunder-debug t) (times '(10 9)))
                               (cl-letf (((symbol-function 'jaunder--debug-now) (lambda () (pop times)))
                                         ((symbol-function 'jaunder--debug-timestamp)
                                          (lambda () "2026-10-07T12:00:00.123Z")))
                                 (jaunder--with-debug-operation "config.resolve" () nil))
                               (should (string-match-p "elapsed-ms=0" (jaunder-debug-test--text))))
                             (jaunder-debug-test--reset)
                             (let ((jaunder-debug t)
                                   (times (list 0 (/ (+ jaunder--debug-maximum-number 1000000) 1000.0))))
                               (cl-letf (((symbol-function 'jaunder--debug-now) (lambda () (pop times)))
                                         ((symbol-function 'jaunder--debug-timestamp)
                                          (lambda () "2026-10-07T12:00:00.123Z")))
                                 (jaunder--with-debug-operation "config.resolve" () nil))
                               (should (string-match-p (format "elapsed-ms=%d" jaunder--debug-maximum-number)
                                                       (jaunder-debug-test--text)))))

(jaunder-debug-test--deftest jaunder-debug-enums-and-boundaries-use-an-independent-registry ()
                             "Every literal spec enum and numeric boundary is accepted or rejected safely."
                             (jaunder-debug-test--reset)
                             (dolist (entry jaunder-debug-test--enums)
                               (dolist (value (cdr entry))
                                 (should (equal value (plist-get (jaunder--debug-normalize-fields
                                                                  (list (car entry) value))
                                                                 (car entry))))))
                             (dolist (key '(method format action eligibility decision reason))
                               (should (equal "unknown"
                                              (plist-get (jaunder--debug-normalize-fields
                                                          (list key "private-value")) key))))
                             (dolist (key '(count bytes page members))
                               (should (= jaunder--debug-maximum-number
                                          (plist-get (jaunder--debug-normalize-fields
                                                      (list key (1+ jaunder--debug-maximum-number))) key)))
                               (should-not (jaunder--debug-normalize-fields (list key -1))))
                             (should (jaunder--debug-normalize-fields '(http-status 100)))
                             (should (jaunder--debug-normalize-fields '(http-status 599)))
                             (should-not (jaunder--debug-normalize-fields '(http-status 99)))
                             (should-not (jaunder--debug-normalize-fields '(http-status "200")))
                             (should-not (jaunder--debug-normalize-fields '(reused yes)))
                             (should-not (jaunder--debug-normalize-fields '(elapsed-ms 1)))
                             (should-not (jaunder--debug-normalize-fields '(path "/private"))) )

(jaunder-debug-test--deftest jaunder-debug-common-fields-are-validated-before-formatting ()
                             "Invalid common values cannot reach formatter or buffer output."
                             (jaunder-debug-test--reset)
                             (let ((formatted nil))
                               (cl-letf (((symbol-function 'jaunder--debug-format-event)
                                          (lambda (_) (setq formatted t) "unsafe")))
                                 (dolist (args '(("unknown" "start" "debug-1" "debug-2" nil nil)
                                                 ("config.resolve" "bad" "debug-1" "debug-2" nil nil)
                                                 ("config.resolve" "start" "UPPER" "debug-2" nil nil)
                                                 ("config.resolve" "end" "debug-1" "debug-2" nil nil "bad" 0)
                                                 ("config.resolve" "end" "debug-1" "debug-2" nil nil "success" -1)))
                                   (should-not (apply #'jaunder--debug-write args))))
                               (should-not formatted)
                               (should-not (get-buffer jaunder--debug-buffer-name)))
                             (should (jaunder--debug-id-p (make-string 32 ?a)))
                             (should-not (jaunder--debug-id-p (make-string 33 ?a))))

(jaunder-debug-test--deftest jaunder-debug-timestamp-has-a-real-nonzero-fraction ()
                             "Timestamp milliseconds derive from the diagnostic clock rather than a constant."
                             (jaunder-debug-test--reset)
                             (cl-letf (((symbol-function 'jaunder--debug-now) (lambda () 1700000000.789)))
                               (should (string-match-p "\\.789Z\\'" (jaunder--debug-timestamp)))))

(jaunder-debug-test--deftest jaunder-debug-preserves-body-semantics-when-diagnostics-fail ()
                             "Metadata, sink, clock, warning, and fallback failures cannot alter BODY."
                             (jaunder-debug-test--reset)
                             (setq jaunder-debug t)
                             (dolist (failure '(jaunder--debug-next-id jaunder--debug-now jaunder--debug-format-event))
                               (let ((runs 0))
                                 (cl-letf (((symbol-function failure) (lambda (&rest _) (error "private diagnostic failure"))))
                                   (should (eq 'value (jaunder--with-debug-operation "config.resolve" ()
                                                                                     (setq runs (1+ runs)) 'value)))
                                   (should (= runs 1))))
                               (cl-letf (((symbol-function 'display-warning) (lambda (&rest _) (error "warning")))
                                         ((symbol-function 'jaunder--debug-fallback-warning) (lambda () (error "fallback")))
                                         ((symbol-function 'jaunder--debug-format-event) (lambda (_) (error "sink"))))
                                 (should-error (jaunder--with-debug-operation "config.resolve" ()
                                                                              (error "business-error")))
                                 (should (eq 'quit (condition-case err
                                                       (jaunder--with-debug-operation "config.resolve" () (signal 'quit '(x)))
                                                     (quit (car err))))))
                               (should-not jaunder--debug-operation-stack)
                               (jaunder--with-debug-operation "config.resolve" () nil)
                               (should (string-match-p "correlation=debug-" (jaunder-debug-test--text)))))

(jaunder-debug-test--deftest jaunder-debug-failures-preserve-exact-business-conditions ()
                             "Initial, update, and terminal failures preserve return, error, and quit data."
                             (jaunder-debug-test--reset)
                             (let ((jaunder-debug t) (value (list 'same)) (runs 0))
                               (should (eq value
                                           (jaunder--with-debug-operation "config.resolve"
                                                                          (count (error "private initial metadata"))
                                                                          (jaunder--debug-fields count (signal 'quit '(private-update)))
                                                                          (setq runs (1+ runs))
                                                                          value)))
                               (should (= runs 1))
                               (should-not jaunder--debug-operation-stack))
                             (jaunder-debug-test--reset)
                             (let ((jaunder-debug t) (fail-terminal nil) (runs 0)
                                   (original (symbol-function 'jaunder--debug-append-line)))
                               (cl-letf (((symbol-function 'jaunder--debug-append-line)
                                          (lambda (line)
                                            (if fail-terminal
                                                (error "private terminal sink")
                                              (funcall original line)))))
                                 (condition-case err
                                     (jaunder--with-debug-operation "config.resolve" ()
                                                                    (setq runs (1+ runs) fail-terminal t)
                                                                    (signal 'jaunder-debug-test-business '(alpha 42)))
                                   (jaunder-debug-test-business
                                    (should (equal '(alpha 42) (cdr err)))))
                                 (should (eq 'quit
                                             (condition-case err
                                                 (jaunder--with-debug-operation "config.resolve" ()
                                                                                (signal 'quit '(alpha 42)))
                                               (quit (should (equal '(alpha 42) (cdr err))) (car err)))))
                                 (setq fail-terminal nil)
                                 (jaunder--with-debug-operation "config.resolve" () nil)
                                 (should-not (string-match-p "parent=" (jaunder-debug-test--text)))
                                 (should (= runs 1))
                                 (should-not jaunder--debug-operation-stack))))

(jaunder-debug-test--deftest jaunder-debug-fields-are-lazy-merged-and-noninterfering ()
                             "Terminal fields retain initial fields, merge updates, and swallow failures."
                             (jaunder-debug-test--reset)
                             (setq jaunder-debug t)
                             (jaunder--with-debug-operation "config.resolve" (method "GET" count 1)
                                                            (jaunder--debug-fields count 2 decision "blocked")
                                                            (jaunder--debug-fields members 3)
                                                            (jaunder--debug-fields count (error "private terminal field"))
                                                            'value)
                             (let ((text (jaunder-debug-test--text)))
                               (should (string-match-p "method=GET" text))
                               (should (string-match-p "count=2" text))
                               (should (string-match-p "decision=blocked" text))
                               (should (string-match-p "members=3" text))
                               (should-not (string-match-p "private terminal field" text))))

(jaunder-debug-test--deftest jaunder-debug-pairs-exact-root-child-and-standalone-ids ()
                             "Nested spans inherit correlation while a later span receives a new root ID."
                             (jaunder-debug-test--reset)
                             (setq jaunder-debug t)
                             (cl-letf (((symbol-function 'jaunder--debug-timestamp) (lambda () "2026-10-07T12:00:00.123Z")))
                               (jaunder--with-debug-operation "config.resolve" ()
                                                              (jaunder--with-debug-operation "auth.lookup" () nil))
                               (jaunder--with-debug-operation "config.resolve" () nil))
                             (let ((text (jaunder-debug-test--text)))
                               (should (string-match-p "correlation=debug-1 span=debug-2 label=config.resolve phase=start" text))
                               (should (string-match-p "correlation=debug-1 span=debug-3 label=auth.lookup phase=start parent=debug-2" text))
                               (should (string-match-p "span=debug-3 label=auth.lookup phase=end parent=debug-2" text))
                               (should (string-match-p "correlation=debug-4 span=debug-5 label=config.resolve phase=start" text))))

(jaunder-debug-test--deftest jaunder-debug-retention-and-lifecycle-are-exact ()
                             "Retention evicts oldest events and lifecycle controls preserve stated evidence."
                             (jaunder-debug-test--reset)
                             (setq jaunder-debug t)
                             (let ((jaunder--debug-maximum-events 3))
                               (dotimes (number 5)
                                 (jaunder--debug-write "config.resolve" "start" "debug-1"
                                                       (format "debug-%d" (+ number 2)) nil nil))
                               (let ((text (jaunder-debug-test--text)))
                                 (should (string-prefix-p "evicted=2\n" text))
                                 (with-current-buffer jaunder--debug-buffer-name
                                   (should (= 4 (count-lines (point-min) (point-max)))))
                                 (should-not (string-match-p "span=debug-2 " text))
                                 (should (string-match-p "span=debug-6 " text))))
                             (let ((retained (jaunder-debug-test--text)))
                               (jaunder-debug-disable)
                               (jaunder--with-debug-operation "config.resolve" () nil)
                               (should (equal retained (jaunder-debug-test--text))))
                             (jaunder-debug-clear)
                             (should (equal "" (jaunder-debug-test--text)))
                             (let ((before jaunder--debug-id-counter))
                               (kill-buffer jaunder--debug-buffer-name)
                               (setq jaunder-debug t)
                               (jaunder--with-debug-operation "config.resolve" () nil)
                               (should (> jaunder--debug-id-counter before))
                               (should (= 2 jaunder--debug-event-count))
                               (should (= 0 jaunder--debug-discarded))))

(jaunder-debug-test--deftest jaunder-debug-real-maximum-retention-bound ()
                             "The production 10,000-event bound retains one marker plus newest events."
                             (jaunder-debug-test--reset)
                             (dotimes (number (1+ jaunder--debug-maximum-events))
                               (jaunder--debug-write "config.resolve" "start" "debug-1"
                                                     (format "debug-%d" (+ number 2)) nil nil))
                             (with-current-buffer jaunder--debug-buffer-name
                               (should (= jaunder--debug-maximum-events jaunder--debug-event-count))
                               (should (= 1 jaunder--debug-discarded))
                               (should (string-prefix-p "evicted=1\n" (buffer-string)))
                               (should (= (1+ jaunder--debug-maximum-events)
                                          (count-lines (point-min) (point-max))))))

(jaunder-debug-test--deftest jaunder-debug-show-and-clear-are-noncreating-or-read-only ()
                             "Show creates a read-only special buffer and clear alone does not create one."
                             (jaunder-debug-test--reset)
                             (jaunder-debug-clear)
                             (should-not (get-buffer jaunder--debug-buffer-name))
                             (let ((displayed nil))
                               (cl-letf (((symbol-function 'pop-to-buffer) (lambda (buffer &rest _) (setq displayed buffer))))
                                 (jaunder-debug-show))
                               (should displayed)
                               (with-current-buffer displayed
                                 (should (derived-mode-p 'jaunder-debug-mode))
                                 (should buffer-read-only)
                                 (should (eq (lookup-key jaunder-debug-mode-map "q") 'quit-window)))))

(provide 'jaunder-debug-test)
;;; jaunder-debug-test.el ends here
