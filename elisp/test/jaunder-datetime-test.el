;;; jaunder-datetime-test.el --- ERT suite for jaunder-datetime -*- lexical-binding: t; -*-

;;; Commentary:
;; Unit tests for the corresponding jaunder package module.

;;; Code:

(require 'ert)
(require 'jaunder)

;;; Publish-time warnings (shared idiom) ----------------------------------

(defmacro jaunder-test--capturing-warnings (&rest body)
  "Run BODY with `display-warning' captured; return the list of (TYPE MSG LEVEL).
Lets the warning tests assert on emitted warnings without touching the real
`*Warnings*' buffer."
  (declare (indent 0))
  `(let (jaunder-test--warnings)
     (cl-letf (((symbol-function 'display-warning)
                (lambda (type message &optional level &rest _)
                  (push (list type message level) jaunder-test--warnings))))
              ,@body)
     (nreverse jaunder-test--warnings)))

;;; offset parsing / zone resolution

(ert-deftest jaunder-offset->seconds-negative ()
  (should (= (jaunder--offset->seconds "-0500") (* -5 3600))))

(ert-deftest jaunder-offset->seconds-positive-with-minutes ()
  (should (= (jaunder--offset->seconds "+0530") (+ (* 5 3600) (* 30 60)))))

(ert-deftest jaunder-offset->seconds-colon-form ()
  (should (= (jaunder--offset->seconds "-05:00") (* -5 3600))))

(ert-deftest jaunder-offset->seconds-zero ()
  (should (= (jaunder--offset->seconds "+0000") 0)))

(ert-deftest jaunder-offset->seconds-iana-name-is-nil ()
  (should (null (jaunder--offset->seconds "America/New_York"))))

(ert-deftest jaunder-offset->seconds-garbage-is-nil ()
  (should (null (jaunder--offset->seconds "not-an-offset")))
  (should (null (jaunder--offset->seconds nil))))

(ert-deftest jaunder-resolve-zone-iana-passthrough ()
  (should (equal (jaunder--resolve-zone "America/New_York") "America/New_York")))

(ert-deftest jaunder-resolve-zone-numeric-to-seconds ()
  (should (= (jaunder--resolve-zone "-0500") (* -5 3600))))

(ert-deftest jaunder-resolve-zone-empty-is-local-nil ()
  (should (null (jaunder--resolve-zone nil)))
  (should (null (jaunder--resolve-zone "   "))))

;;; utc->org-date + machine-zone capture

(ert-deftest jaunder-utc->org-date-renders-in-zone ()
  ;; 13:00Z in America/New_York (EDT, -04:00) is 09:00 local.
  (should (equal (jaunder--utc->org-date "2026-07-01T13:00:00Z" "America/New_York")
                 "[2026-07-01 Wed 09:00]"))
  ;; Round-trips through the existing forward mapping.
  (should (equal (jaunder--org-date->utc
                  (jaunder--utc->org-date "2026-07-01T13:00:00Z" "America/New_York")
                  "America/New_York")
                 "2026-07-01T13:00:00Z")))

(ert-deftest jaunder-current-zone-name-is-nonempty ()
  (let ((z (jaunder--current-zone-name)))
    (should (stringp z))
    (should (> (length z) 0))))

(ert-deftest jaunder-current-zone-name-prefers-explicit-tz ()
  "A configured IANA zone outranks the host localtime link."
  (cl-letf (((symbol-function 'getenv)
             (lambda (name) (and (equal name "TZ") "America/Chicago"))))
           (should (equal (jaunder--current-zone-name) "America/Chicago"))))

(ert-deftest jaunder-current-zone-name-reads-localtime-zoneinfo-link ()
  "Without TZ, retain the named zone exposed by /etc/localtime."
  (cl-letf (((symbol-function 'getenv) (lambda (_name) nil))
            ((symbol-function 'file-symlink-p)
             (lambda (_path) "/usr/share/zoneinfo/Europe/Paris")))
           (should (equal (jaunder--current-zone-name) "Europe/Paris"))))

;;; #217 — zone-mismatch warning

(ert-deftest jaunder-zone-offset-p-recognizes-offsets ()
  (should (jaunder--zone-offset-p "-0400"))
  (should (jaunder--zone-offset-p "+0000"))
  (should-not (jaunder--zone-offset-p "America/New_York"))
  (should-not (jaunder--zone-offset-p nil)))

(ert-deftest jaunder-warn-zone-mismatch-fires-on-difference ()
  ;; AC-217a: recorded IANA zone differs from the machine's current zone.
  (cl-letf (((symbol-function 'jaunder--current-zone-name)
             (lambda () "Europe/London")))
           (let ((warnings (jaunder-test--capturing-warnings
                            (jaunder--warn-zone-mismatch "America/New_York"))))
             (should (= (length warnings) 1))
             (pcase-let ((`(,type ,message ,level) (car warnings)))
               (should (eq type 'jaunder))
               (should (eq level :warning))
               (should (string-prefix-p "jaunder: " message))
               (should (string-match-p "America/New_York" message))
               (should (string-match-p "Europe/London" message))))))

(ert-deftest jaunder-warn-zone-mismatch-silent-when-unset ()
  ;; AC-217b: no recorded zone yet (captured this publish) → nothing to warn about.
  (cl-letf (((symbol-function 'jaunder--current-zone-name)
             (lambda () "Europe/London")))
           (should-not (jaunder-test--capturing-warnings
                        (jaunder--warn-zone-mismatch nil)))))

(ert-deftest jaunder-warn-zone-mismatch-silent-when-equal ()
  ;; AC-217c (IANA): recorded == current.
  (cl-letf (((symbol-function 'jaunder--current-zone-name)
             (lambda () "America/New_York")))
           (should-not (jaunder-test--capturing-warnings
                        (jaunder--warn-zone-mismatch "America/New_York")))))

(ert-deftest jaunder-warn-zone-mismatch-silent-both-offsets ()
  ;; AC-217c (offset): two numeric offsets differ only across DST on one machine.
  (cl-letf (((symbol-function 'jaunder--current-zone-name)
             (lambda () "-0400")))
           (should-not (jaunder-test--capturing-warnings
                        (jaunder--warn-zone-mismatch "-0500")))))

(ert-deftest jaunder-warn-zone-mismatch-suppressed ()
  ;; AC-217d: the defcustom silences it even on a real difference.
  (cl-letf (((symbol-function 'jaunder--current-zone-name)
             (lambda () "Europe/London")))
           (let ((jaunder-warn-zone-mismatch nil))
             (should-not (jaunder-test--capturing-warnings
                          (jaunder--warn-zone-mismatch "America/New_York"))))))

;;; jaunder-datetime-test.el ends here
