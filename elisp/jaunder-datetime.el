;;; jaunder-datetime.el --- Jaunder date/timezone handling -*- lexical-binding: t; -*-

;; Copyright (C) 2026 Jaunder contributors

;; This program is free software: you can redistribute it and/or modify
;; it under the terms of the GNU General Public License as published by
;; the Free Software Foundation, either version 3 of the License, or
;; (at your option) any later version.
;;
;; This program is distributed in the hope that it will be useful,
;; but WITHOUT ANY WARRANTY; without even the implied warranty of
;; MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
;; GNU General Public License for more details.
;;
;; You should have received a copy of the GNU General Public License
;; along with this program.  If not, see <https://www.gnu.org/licenses/>.

;;; Commentary:
;; Timezone-aware conversion between org #+DATE: timestamps and RFC-3339 UTC,
;; plus capturing the machine zone into JAUNDER_DATE_TZ so a post's publish time
;; is interpreted in a recorded zone rather than one re-inferred on another host.

;;; Code:

(require 'org)
(require 'seq)
(require 'jaunder-config)
(require 'jaunder-warn)

(defun jaunder--offset->seconds (offset)
  "Parse a numeric UTC OFFSET string (\"±HHMM\" or \"±HH:MM\") to integer seconds.
Returns nil when OFFSET is not a numeric offset.  Used only for the
JAUNDER_DATE_TZ fallback: `encode-time' silently misreads an offset *string*
as UTC, so a numeric offset must be handed to it as integer seconds."
  (when (and offset
             (string-match
              "\\`\\([+-]\\)\\([0-9]\\{2\\}\\):?\\([0-9]\\{2\\}\\)\\'" offset))
    (let ((sign (if (string= (match-string 1 offset) "-") -1 1))
          (hours (string-to-number (match-string 2 offset)))
          (mins (string-to-number (match-string 3 offset))))
      (* sign (+ (* hours 3600) (* mins 60))))))

(defun jaunder--resolve-zone (tz)
  "Resolve a JAUNDER_DATE_TZ string TZ to an `encode-time' ZONE value.
An IANA name is preferred and returned as-is (DST-correct); a numeric offset
is parsed to integer seconds (the fallback — see `jaunder--offset->seconds').
nil or empty TZ falls back to the local zone.  A typo'd IANA name is silently
treated as UTC by `encode-time'; time zones are hard and we do our best."
  (cond
   ((or (null tz) (string= (string-trim tz) "")) nil)
   ((jaunder--offset->seconds tz))
   (t tz)))

(defun jaunder--org-date->utc (date-raw tz)
  "Interpret org timestamp DATE-RAW in zone TZ; return RFC-3339 UTC, or nil.
DATE-RAW is a raw #+DATE value (e.g. \"[2026-07-01 Wed 09:00]\"); TZ is a
JAUNDER_DATE_TZ string (IANA name preferred, numeric offset as fallback).
Returns nil when DATE-RAW does not parse to a time."
  (let ((decoded (ignore-errors (org-parse-time-string date-raw))))
    (when decoded
      (setf (nth 8 decoded) (jaunder--resolve-zone tz))
      (let ((utc (format-time-string "%Y-%m-%dT%H:%M:%SZ" (encode-time decoded) t)))
        (jaunder--publication-utc utc)
        utc))))

(defun jaunder--utc->org-date (utc tz)
  "Render an org inactive timestamp for UTC interpreted in zone TZ.
UTC is an RFC-3339 UTC string (e.g. \"2026-07-01T13:00:00Z\"); TZ a
JAUNDER_DATE_TZ string.  Inverse of `jaunder--org-date->utc' at org's minute
resolution: a server UTC carrying non-zero seconds is truncated to the minute
\(org timestamps have no seconds field)."
  (format-time-string "[%Y-%m-%d %a %H:%M]"
                      (date-to-time utc)
                      (jaunder--resolve-zone tz)))

(defun jaunder--strict-local-date->utc (date zone)
  "Interpret local Org DATE in validated ZONE, rejecting gaps.
Choose the earlier fold instant, as at the strict Org seam in ADR-0182.
Unlike `jaunder--org-date->utc', this is local preparation only."
  (let* ((decoded (ignore-errors (org-parse-time-string date)))
         (civil (and decoded (seq-subseq decoded 1 6)))
         (resolved (jaunder--resolve-zone zone))
         candidates)
    (unless (and civil (seq-every-p #'integerp civil))
      (user-error "jaunder: invalid DATE; supply an Org date and time"))
    ;; encode-time can normalize gaps and pick either fold.  Only candidates
    ;; that render back to the authored civil fields are eligible.
    (dolist (dst '(nil t))
      (let ((parts (copy-sequence decoded)))
        (setf (nth 0 parts) 0 (nth 7 parts) dst (nth 8 parts) resolved)
        (let ((instant (encode-time parts)))
          (when (equal civil (seq-subseq (decode-time instant resolved) 1 6))
            (push instant candidates)))))
    (unless candidates
      (user-error "jaunder: DATE is nonexistent or invalid in %s; choose another local time" zone))
    (let ((utc (format-time-string "%Y-%m-%dT%H:%M:%SZ"
                                   (car (sort candidates #'time-less-p)) t)))
      (jaunder--publication-utc utc)
      utc)))

(defun jaunder--local-baseline-zone-p (zone)
  "Return non-nil for a recognized named ZONE or valid legacy numeric offset.
Use the same TZDIR zoneinfo database as Emacs conversion (the standard
system directory when unset), not a second list of timezone names."
  (and (stringp zone)
       (or (string-match-p "\\`[+-]\\(?:[01][0-9]\\|2[0-3]\\):?[0-5][0-9]\\'" zone)
           (and (not (equal zone "Etc/Unknown"))
                (string-match-p
                 "\\`[A-Za-z0-9_+-]+\\(?:/[A-Za-z0-9_+-]+\\)*\\'" zone)
                (let ((file (expand-file-name
                             zone (or (getenv "TZDIR") "/usr/share/zoneinfo"))))
                  ;; A TZif file, not a typo that libc silently treats as UTC.
                  (and (file-regular-p file)
                       (ignore-errors
                         (with-temp-buffer
                           (insert-file-contents-literally file nil 0 4)
                           (equal (buffer-string) "TZif")))))))))

(defun jaunder--prepare-local-date (date old-zone saved-utc latest-zone existing)
  "Prepare local DATE for LATEST-ZONE without changing any buffer or file.
OLD-ZONE and SAVED-UTC are the paired local baseline.  EXISTING is non-nil
for an existing published or scheduled Post, never for a new Post or draft.
Return a plist with :utc, :date and :zone for eventual confirmed write-back.
The caller supplies the validated effective LATEST-ZONE; capture and network
write-back belong to the publish workflow, not this pure preparation step."
  ;; A missing baseline must never turn a routine edit into a new instant.
  (when existing
    (unless (and (stringp saved-utc)
                 (ignore-errors (jaunder--publication-utc saved-utc)))
      (user-error "jaunder: missing/invalid JAUNDER_DATE_UTC; pull/reconcile before publishing")))
  (when (and existing (not (jaunder--local-baseline-zone-p old-zone)))
    (user-error "jaunder: missing/invalid JAUNDER_DATE_TZ baseline; pull/reconcile before publishing"))
  (let* ((unchanged
          (and existing saved-utc
               (equal (seq-subseq (org-parse-time-string date) 1 6)
                      (seq-subseq
                       (org-parse-time-string
                        (jaunder--utc->org-date saved-utc old-zone)) 1 6))))
         (utc (if unchanged saved-utc
                (jaunder--strict-local-date->utc date latest-zone))))
    (list :utc utc :date (jaunder--utc->org-date utc latest-zone)
          :zone latest-zone)))

(defun jaunder--local-iana-zone-p (zone)
  "Return non-nil for ZONE in local tzdata's named IANA Zone/Link inventory.
A TZif file alone may be a posix/right or administrator-local name.  Use the
machine-readable tzdata.zi inventory, including aliases, without normalizing
spelling.  A missing inventory makes capture unavailable.  Its tzdata version
may differ from the server's bundled Jiff database; server validation remains
authoritative."
  (and (jaunder--local-baseline-zone-p zone)
       (not (jaunder--zone-offset-p zone))
       (let ((inventory (expand-file-name "tzdata.zi" (or (getenv "TZDIR") "/usr/share/zoneinfo"))))
         (when (file-readable-p inventory)
           (with-temp-buffer
             (insert-file-contents inventory)
             (let ((case-fold-search t)
                   (name (regexp-quote zone)))
               (or (re-search-forward (concat "^\\(?:Z\\|Zone\\)[ \t]+" name "\\(?:[ \t]\\|$\\)") nil t)
                   (progn
                     (goto-char (point-min))
                     (re-search-forward
                      (concat "^\\(?:L\\|Link\\)[ \t]+[^ \t\n]+[ \t]+" name "\\(?:[ \t]\\|$\\)") nil t)))))))))

(defun jaunder--capture-editing-zone ()
  "Capture the current named device zone, else an exact bounded fixed offset.
Warn for every inferred fixed offset or explicit UTC fallback.  Never infer
regional identity or DST history from an offset, or reuse the old Post zone."
  (let* ((zone (condition-case nil (jaunder--current-zone-name) (error nil)))
         (named (condition-case nil (jaunder--local-iana-zone-p zone) (error nil))))
    (if named
        zone
      ;; Emacs reports current device offset in seconds EAST of UTC; Etc/GMT
      ;; names reverse that sign.  Require integer whole hours, never round.
      (let* ((east (condition-case nil (car (current-time-zone)) (error nil)))
             (fixed (when (and (integerp east) (<= -43200 east 43200)
                               (zerop (% east 3600)))
                      (if (zerop east) "Etc/GMT"
                        (format "Etc/GMT%+d" (- (/ east 3600)))))))
        ;; Conversion must work locally even when the name inventory is absent.
        (if (and fixed (jaunder--local-baseline-zone-p fixed))
            (progn
              (jaunder--warn "device timezone name unavailable or unsupported; inferred fixed offset %s; regional timezone and DST history are unknown" fixed)
              fixed)
          (jaunder--warn "cannot capture a supported named device timezone or exact supported whole-hour offset; using UTC")
          "UTC")))))

(defconst jaunder--datetime-rfc-3339-regexp
  "\\`\\([0-9]\\{4\\}\\)-\\([0-9]\\{2\\}\\)-\\([0-9]\\{2\\}\\)T\\([0-9]\\{2\\}\\):\\([0-9]\\{2\\}\\):\\([0-9]\\{2\\}\\)\\(?:\\.[0-9]\\{1,9\\}\\)?\\(?:Z\\|[+-]\\([0-9]\\{2\\}\\):\\([0-9]\\{2\\}\\)\\)\\'"
  "RFC-3339 timestamp shape with captured calendar and numeric-offset fields.")

(defun jaunder--datetime-components-valid-p (published)
  "Return non-nil when matched RFC-3339 PUBLISHED fields are semantically valid.
This rejects values `date-to-time' would normalize, including impossible
Gregorian dates and out-of-range numeric offsets."
  (let* ((year (string-to-number (match-string 1 published)))
         (month (string-to-number (match-string 2 published)))
         (day (string-to-number (match-string 3 published)))
         (hour (string-to-number (match-string 4 published)))
         (minute (string-to-number (match-string 5 published)))
         (second (string-to-number (match-string 6 published)))
         (offset-hour (match-string 7 published))
         (offset-minute (match-string 8 published))
         (days-in-month
          (and (<= 1 month 12)
               (aref [0 31 28 31 30 31 30 31 31 30 31 30 31] month))))
    (when (and (= month 2)
               (or (= 0 (% year 400))
                   (and (= 0 (% year 4))
                        (/= 0 (% year 100)))))
      (setq days-in-month 29))
    (and days-in-month
         (<= 1 day days-in-month)
         (<= 0 hour 23)
         (<= 0 minute 59)
         (<= 0 second 59)
         (or (null offset-hour)
             (and (<= (string-to-number offset-hour) 23)
                  (<= (string-to-number offset-minute) 59))))))

(defun jaunder--publication-utc (published)
  "Canonicalize PUBLISHED to fixed-nine UTC within the application range.
Retain source nanoseconds as text; only whole seconds pass through Emacs."
  (unless (and (stringp published)
               (string-match jaunder--datetime-rfc-3339-regexp published)
               (jaunder--datetime-components-valid-p published))
    (error "jaunder: invalid offset-qualified RFC-3339 instant"))
  (let* ((fraction (if (string-match "\\.\\([0-9]+\\)" published)
                       (match-string 1 published) ""))
         (utc (concat
               (format-time-string
                "%Y-%m-%dT%H:%M:%S"
                (date-to-time
                 (replace-regexp-in-string "\\.[0-9]+" "" published)) t)
               "." fraction (make-string (- 9 (length fraction)) ?0) "Z")))
    (unless (and (= (length utc) 30)
                 (not (string< utc "0001-01-01T00:00:00.000000000Z"))
                 (not (string< "9999-12-30T22:00:00.999999999Z" utc)))
      (error "jaunder: UTC instant outside application range"))
    utc))

(defun jaunder--utc-time (value)
  "Parse valid RFC-3339 VALUE into an exact Emacs time, preserving nanoseconds."
  (let ((utc (jaunder--publication-utc value)))
    ;; `date-to-time' ignores fractions on supported Emacs versions.  Add
    ;; integer picoseconds to its whole-second result, never a floating clock.
    (time-add (date-to-time (concat (substring utc 0 19) "Z"))
              (list 0 0 0 (* 1000 (string-to-number (substring utc 20 29)))))))

(defun jaunder--canonical-utc (&optional time)
  "Emit TIME (or the current clock) as application-range fixed-nine UTC.
String inputs retain their source precision through the publication seam."
  (jaunder--publication-utc
   (if (stringp time) time
     (format-time-string "%Y-%m-%dT%H:%M:%S.%9NZ" (or time (current-time)) t))))

(defun jaunder--current-zone-name ()
  "Return the machine's current IANA zone name, else a numeric offset string.
Prefers a `TZ' IANA name, then the /etc/localtime symlink target; falls back to
the current numeric UTC offset (IANA preferred, offset caveat).  The TZ branch
trusts a non-empty, non-`:'-prefixed value as an IANA name; a POSIX-style TZ
\(e.g. \"EST5EDT\") is passed through as-is."
  (or (let ((tz (getenv "TZ")))
        (and tz (not (string= tz "")) (not (string-prefix-p ":" tz)) tz))
      (let ((link (ignore-errors (file-symlink-p "/etc/localtime"))))
        (and link (string-match "zoneinfo/\\(.+\\)\\'" link)
             (match-string 1 link)))
      (format-time-string "%z")))

(defun jaunder--zone-offset-p (zone)
  "Return non-nil when ZONE is a numeric offset like \"-0400\", not an IANA name."
  (and (stringp zone) (string-match-p "\\`[+-][0-9]" zone)))

(defun jaunder--warn-zone-mismatch (recorded)
  "Warn when RECORDED zone differs from the machine's current zone.
RECORDED is the pre-existing JAUNDER_DATE_TZ (nil when unset).  No warning when
RECORDED is nil, equals the current zone, or when both are numeric offsets — a
same-machine DST offset flip (e.g. -0500 -> -0400) is not a real zone change, so
comparing offsets would only false-positive.  Gated by
`jaunder-warn-zone-mismatch'."
  (when (and jaunder-warn-zone-mismatch recorded)
    (let ((current (jaunder--current-zone-name)))
      (unless (or (string= recorded current)
                  (and (jaunder--zone-offset-p recorded)
                       (jaunder--zone-offset-p current)))
        (jaunder--warn
         "recorded timezone %s differs from this machine's zone %s; #+DATE: will be interpreted in the recorded zone %s"
         recorded current recorded)))))

(provide 'jaunder-datetime)
;;; jaunder-datetime.el ends here
