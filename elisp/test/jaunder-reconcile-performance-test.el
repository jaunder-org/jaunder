;;; jaunder-reconcile-performance-test.el --- Selected batch stage diagnostics -*- lexical-binding: t; -*-

;; Copyright (C) 2026 Jaunder contributors

;;; Commentary:
;; Public batch stage diagnostics retain the original timeout context.

;;; Code:

(require 'ert)
(require 'cl-lib)
(require 'jaunder)

(ert-deftest jaunder-reconcile-pull-batch-shows-stages-and-retains-timeout-context ()
  "A stalled first Post reports its stage; the next eligible Post still runs."
  (let* ((jaunder-blogs '(("/tmp/" :base-url "https://example.test" :username "alice")))
         (rows (mapcar (lambda (id)
                         (jaunder--make-reconcile-row
                          :key (format "post:%s" id) :state 'server-ahead
                          :remote-etag "\"same\""
                          :local (jaunder--make-inventory-local
                                  :path (format "/tmp/%s.org" id) :id id)
                          :member (jaunder--make-inventory-member :id id :slug id)))
                       '("7" "8")))
         (buffer (jaunder--render-reconcile-report
                  (jaunder--make-reconcile-report :root "/tmp" :rows rows)))
         progress)
    (unwind-protect
        (with-current-buffer buffer
          (cl-letf (((symbol-function 'jaunder--pull-stage-member)
                     (lambda (_root member)
                       (if (equal (jaunder-inventory-member-id member) "7")
                           (error "read timed out")
                         '(:id "8" :slug "8" :etag "\"same\"" :bytes "post"))))
                    ((symbol-function 'jaunder--reconcile-pull-unique-match)
                     (lambda (_) '(:ok t)))
                    ((symbol-function 'jaunder--reconcile-pull-remote-revalidation)
                     (lambda (&rest _) '(:ok t)))
                    ((symbol-function 'jaunder--reconcile-pull-install-staged)
                     (lambda (&rest _) '(:outcome success :local-effect replaced)))
                    ((symbol-function 'jaunder--reconcile-refresh-buffer)
                     (lambda (&rest _) nil))
                    ((symbol-function 'message)
                     (lambda (format-string &rest args)
                       (push (apply #'format format-string args) progress))))
            (should (eq (jaunder--call-with-blog
                         "/tmp" (lambda ()
                                  (jaunder--reconcile-execute-batch
                                   buffer rows 'pull #'jaunder--reconcile-pull-row)))
                        'completed))
            (let ((results jaunder-reconcile-last-batch-results)
                  (events (nreverse progress)))
              (should (equal (mapcar #'jaunder-reconcile-result-outcome results)
                             '(failed success)))
              (should (string-match-p "staging Member: .*read timed out"
                                      (jaunder-reconcile-result-detail (car results))))
              (dolist (stage '("1/2 Post 7 — staging Member"
                               "2/2 Post 8 — staging Member"
                               "2/2 Post 8 — verifying fresh Collection"
                               "2/2 Post 8 — revalidating Member"
                               "2/2 Post 8 — installing local Post"
                               "refreshing report after 2/2"
                               "batch complete after 2/2"))
                (should (cl-some (lambda (event) (string-match-p (regexp-quote stage) event))
                                 events))))))
      (kill-buffer buffer))))

(provide 'jaunder-reconcile-performance-test)
;;; jaunder-reconcile-performance-test.el ends here
