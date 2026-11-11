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

;;; Local publication preparation

(ert-deftest jaunder-date-prepare-preserves-exact-instant-across-zone-change ()
  "An unchanged minute DATE carries the original sub-minute instant."
  (should
   (equal (jaunder--prepare-local-date
           "[2026-07-01 Wed 09:00]" "America/New_York"
           "2026-07-01T13:00:42.123456789Z" "Europe/London" t)
          '(:utc "2026-07-01T13:00:42.123456789Z"
                 :date "[2026-07-01 Wed 14:00]" :zone "Europe/London"))))

(ert-deftest jaunder-date-prepare-preserves-later-fold-for-scheduled-post ()
  "A scheduled Post's unchanged DATE must not select the earlier fold."
  (should
   (equal (jaunder--prepare-local-date
           "[2030-11-03 Sun 01:30]" "America/New_York"
           "2030-11-03T06:30:17.987654321Z" "Europe/London" t)
          '(:utc "2030-11-03T06:30:17.987654321Z"
                 :date "[2030-11-03 Sun 06:30]" :zone "Europe/London"))))

(ert-deftest jaunder-date-prepare-changed-date-uses-latest-zone ()
  (should
   (equal (jaunder--prepare-local-date
           "[2026-07-01 Wed 10:15]" "America/New_York"
           "2026-07-01T13:00:42.123456789Z" "Europe/London" t)
          '(:utc "2026-07-01T09:15:00Z"
                 :date "[2026-07-01 Wed 10:15]" :zone "Europe/London"))))

(ert-deftest jaunder-date-prepare-requires-existing-publication-baseline ()
  "Missing or invalid markers stop even when the authored DATE changed."
  (dolist (marker '(nil "" "not-a-time" "2026-02-30T13:00:00Z"
                        "2026-07-01T13:00:00"))
    (let ((err (should-error
                (jaunder--prepare-local-date
                 "[2026-07-01 Wed 10:15]" "America/New_York"
                 marker "Europe/London" t)
                :type 'user-error)))
      (should (string-match-p "pull/reconcile" (error-message-string err)))
      (should (string-match-p "JAUNDER_DATE_UTC" (error-message-string err)))))
  (let ((err (should-error
              (jaunder--prepare-local-date
               "[2026-07-01 Wed 09:00]" nil "2026-07-01T13:00:00Z"
               "Europe/London" t)
              :type 'user-error)))
    (should (string-match-p "pull/reconcile" (error-message-string err)))))

(ert-deftest jaunder-date-prepare-new-post-needs-no-marker ()
  (should
   (equal (jaunder--prepare-local-date
           "[2030-07-01 Mon 09:00]" nil nil "America/New_York" nil)
          '(:utc "2030-07-01T13:00:00Z"
                 :date "[2030-07-01 Mon 09:00]" :zone "America/New_York"))))

(ert-deftest jaunder-date-prepare-authored-fold-chooses-earlier-instant ()
  "Both new and changed DATE use the strict Org fold policy."
  (dolist (existing '(nil t))
    (should
     (equal (jaunder--prepare-local-date
             "[2030-11-03 Sun 01:30]" "Europe/London"
             (and existing "2030-11-02T12:00:00Z")
             "America/New_York" existing)
            '(:utc "2030-11-03T05:30:00Z"
                   :date "[2030-11-03 Sun 01:30]" :zone "America/New_York")))))

(ert-deftest jaunder-date-prepare-authored-gap-rejects-without-local-mutation ()
  "Preparing a new or changed DATE never normalizes a gap or writes headers."
  (dolist (existing '(nil t))
    (with-temp-buffer
      (insert "#+DATE: [2030-03-10 Sun 02:30]\n"
              "#+PROPERTY: JAUNDER_DATE_TZ Europe/London\n"
              "#+PROPERTY: JAUNDER_DATE_UTC 2030-03-09T12:00:00Z\n")
      (let ((before (buffer-string)))
        (should-error
         (jaunder--prepare-local-date
          "[2030-03-10 Sun 02:30]" "Europe/London"
          (and existing "2030-03-09T12:00:00Z") "America/New_York" existing)
         :type 'user-error)
        (should (equal before (buffer-string)))))))

(ert-deftest jaunder-date-prepare-legacy-baseline-is-read-only ()
  "Legacy numeric zones can describe the old DATE, never the new saved zone."
  (with-temp-buffer
    (insert "#+DATE: [2026-07-01 Wed 09:00]\n"
            "#+PROPERTY: JAUNDER_DATE_TZ -0400\n"
            "#+PROPERTY: JAUNDER_DATE_UTC 2026-07-01T13:00:42.123Z\n")
    (let ((before (buffer-string)))
      (should
       (equal (jaunder--prepare-local-date
               "[2026-07-01 Wed 09:00]" "-0400"
               "2026-07-01T13:00:42.123Z" "Europe/London" t)
              '(:utc "2026-07-01T13:00:42.123Z"
                     :date "[2026-07-01 Wed 14:00]" :zone "Europe/London")))
      (should (equal before (buffer-string))))))

(ert-deftest jaunder-date-prepare-rejects-invalid-recorded-zone ()
  "Neither an unchanged-looking nor edited DATE can repair an invalid baseline."
  (dolist (zone '("America/New_Yrok" "Etc/Unknown" "+2460" "+05:99" "" "  "))
    (dolist (date '("[2026-07-01 Wed 09:00]" "[2026-07-01 Wed 10:15]"))
      (let ((err (should-error
                  (jaunder--prepare-local-date
                   date zone "2026-07-01T13:00:42.123Z" "Europe/London" t)
                  :type 'user-error)))
        (should (string-match-p "JAUNDER_DATE_TZ" (error-message-string err)))
        (should (string-match-p "pull/reconcile" (error-message-string err)))))))

(ert-deftest jaunder-date-prepare-preserves-positive-legacy-offset-baseline ()
  (dolist (zone '("+0530" "+05:30"))
    (should
     (equal (jaunder--prepare-local-date
             "[2026-07-01 Wed 18:30]" zone "2026-07-01T13:00:42.123Z"
             "Europe/London" t)
            '(:utc "2026-07-01T13:00:42.123Z"
                   :date "[2026-07-01 Wed 14:00]" :zone "Europe/London")))))

(ert-deftest jaunder-publication-utc-canonical-nanoseconds ()
  "Owned UTC output pads precision without passing fractions through a clock."
  (dolist (pair '(("2026-07-01T13:00:00Z" . "2026-07-01T13:00:00.000000000Z")
                  ("1969-12-31T18:59:59.000000001-05:00" . "1969-12-31T23:59:59.000000001Z")
                  ("2030-11-03T01:30:17.987654321-05:00" . "2030-11-03T06:30:17.987654321Z")
                  ("2026-07-01T18:30:42.123+05:30" . "2026-07-01T13:00:42.123000000Z")))
    (should (equal (jaunder--publication-utc (car pair)) (cdr pair)))))

(ert-deftest jaunder-sync-marker-historical-and-canonical ()
  "Reconciliation reads historical markers and exact new nanoseconds alike."
  (dolist (value '("1969-12-31T23:59:59Z"
                   "1969-12-31T18:59:59.123456789-05:00"
                   "1969-12-31T23:59:59.123456789Z"
                   "1969-12-31T23:59:59.1+00:00"))
    (let ((time (jaunder--reconcile-synced-time value)))
      (should time)
      (should (equal (format-time-string "%Y-%m-%dT%H:%M:%S.%9NZ" time t)
                     (cond ((string-match-p "123456789" value)
                            "1969-12-31T23:59:59.123456789Z")
                           ((string-suffix-p ".1+00:00" value)
                            "1969-12-31T23:59:59.100000000Z")
                           (t "1969-12-31T23:59:59.000000000Z")))))))

(ert-deftest jaunder-owned-utc-normalized-application-bounds ()
  "Bounds are UTC instants, not local-year or post-1970 restrictions."
  (dolist (pair '(("0001-01-01T00:00:00Z" . "0001-01-01T00:00:00.000000000Z")
                  ("0000-12-31T23:00:00-01:00" . "0001-01-01T00:00:00.000000000Z")
                  ("9999-12-30T23:00:00.999999999+01:00" . "9999-12-30T22:00:00.999999999Z")))
    (should (equal (jaunder--publication-utc (car pair)) (cdr pair)))
    (should (jaunder--reconcile-synced-time (car pair))))
  (dolist (value '("0000-12-31T23:59:59.999999999Z"
                   "0001-01-01T00:00:00+00:01"
                   "9999-12-30T22:00:01Z"
                   "9999-12-30T22:00:00.999999999-00:01"
                   "2026-02-30T12:00:00Z"
                   "2026-07-01T12:00:00+24:00"
                   "2026-07-01T12:00:00.1234567890Z"))
    (should-error (jaunder--publication-utc value))
    (should-not (jaunder--reconcile-synced-time value))))

(ert-deftest jaunder-date-prepare-rejects-normalized-out-of-range-local-date ()
  "Local preparation must reject UTC crossings before any publication request."
  (dolist (case '(("[0001-01-01 Mon 00:00]" "Etc/GMT-1")
                  ("[9999-12-31 Fri 00:00]" "UTC")))
    (should-error (jaunder--prepare-local-date
                   (car case) nil nil (cadr case) nil))))

;;; jaunder-datetime-test.el ends here
