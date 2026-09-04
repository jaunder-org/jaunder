;;; jaunder-service-test.el --- ERT suite for jaunder-service -*- lexical-binding: t; -*-

;;; Commentary:
;; Unit tests for the corresponding jaunder package module.

;;; Code:

(require 'ert)
(require 'jaunder)

(defun jaunder-test--response (status headers body)
  "Build a `jaunder--http-request'-shaped plist for tests."
  (list :status status
        :headers (mapcar (lambda (h) (cons (downcase (car h)) (cdr h))) headers)
        :body body))

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

;;; #216 — missing-format-media-type warning

(defconst jaunder-test--service-doc-with-feature
  (concat "<?xml version=\"1.0\"?>"
          "<service xmlns=\"http://www.w3.org/2007/app\""
          " xmlns:j=\"https://jaunder.org/ns/atompub\">"
          "<workspace><j:extension version=\"1\""
          " features=\"format-media-type slug\"/></workspace></service>")
  "A service document advertising the format-media-type feature.")

(ert-deftest jaunder-parse-service-features-reads-features-attr ()
  (should (equal (jaunder--parse-service-features
                  jaunder-test--service-doc-with-feature)
                 '("format-media-type" "slug"))))

(ert-deftest jaunder-parse-service-features-absent-is-empty ()
  ;; Parses fine but advertises nothing → empty list, not `unknown'.
  (should (equal (jaunder--parse-service-features
                  (concat "<service xmlns=\"http://www.w3.org/2007/app\">"
                          "<workspace/></service>"))
                 '())))

(ert-deftest jaunder-parse-service-features-ignores-incidental-text ()
  ;; AC-216e: the token in a title text node is not the `features' attribute.
  (should-not
   (member "format-media-type"
           (jaunder--parse-service-features
            (concat "<service xmlns=\"http://www.w3.org/2007/app\">"
                    "<workspace><atom:title"
                    " xmlns:atom=\"http://www.w3.org/2005/Atom\">"
                    "format-media-type</atom:title></workspace></service>")))))

(ert-deftest jaunder-parse-service-features-unparseable-is-unknown ()
  ;; AC-216d: a 2xx body that is not parseable XML → unknown, not "absent".
  (should (eq (jaunder--parse-service-features "garbage, not xml") 'unknown)))

(ert-deftest jaunder-warn-missing-fmt-fires-when-absent ()
  ;; AC-216a
  (let ((jaunder--service-doc-cache nil))
    (cl-letf (((symbol-function 'jaunder--fetch-service-features)
               (lambda (_base) '("slug"))))
             (let ((warnings (jaunder-test--capturing-warnings
                              (jaunder--warn-missing-format-media-type "https://blog"))))
               (should (= (length warnings) 1))
               (should (eq (nth 0 (car warnings)) 'jaunder))
               (should (string-prefix-p "jaunder: " (nth 1 (car warnings))))
               (should (string-match-p "format-media-type" (nth 1 (car warnings))))
               (should (string-match-p "https://blog" (nth 1 (car warnings))))))))

(ert-deftest jaunder-warn-missing-fmt-caches-once-per-blog ()
  ;; AC-216b: a second publish neither re-warns nor re-fetches.
  (let ((jaunder--service-doc-cache nil) (fetches 0))
    (cl-letf (((symbol-function 'jaunder--fetch-service-features)
               (lambda (_base) (cl-incf fetches) '("slug"))))
             (let ((first (jaunder-test--capturing-warnings
                           (jaunder--warn-missing-format-media-type "https://blog")))
                   (second (jaunder-test--capturing-warnings
                            (jaunder--warn-missing-format-media-type "https://blog"))))
               (should (= (length first) 1))
               (should (null second))
               (should (= fetches 1))))))

(ert-deftest jaunder-warn-missing-fmt-silent-when-present ()
  ;; AC-216c
  (let ((jaunder--service-doc-cache nil))
    (cl-letf (((symbol-function 'jaunder--fetch-service-features)
               (lambda (_base) '("format-media-type" "slug"))))
             (should-not (jaunder-test--capturing-warnings
                          (jaunder--warn-missing-format-media-type "https://blog"))))))

(ert-deftest jaunder-warn-missing-fmt-unknown-not-cached ()
  ;; AC-216d: unknown → no warning and no cache entry (a later publish retries).
  (let ((jaunder--service-doc-cache nil))
    (cl-letf (((symbol-function 'jaunder--fetch-service-features)
               (lambda (_base) 'unknown)))
             (should-not (jaunder-test--capturing-warnings
                          (jaunder--warn-missing-format-media-type "https://blog")))
             (should-not (assoc "https://blog" jaunder--service-doc-cache)))))

(ert-deftest jaunder-fetch-service-features-catches-signal ()
  ;; AC-216d seam: a transport signal becomes `unknown', never propagates.
  (cl-letf (((symbol-function 'jaunder--http-request)
             (lambda (&rest _) (error "boom"))))
           (should (eq (jaunder--fetch-service-features "https://blog") 'unknown))))

(ert-deftest jaunder-warn-missing-fmt-suppressed ()
  ;; AC-216f: disabled → no fetch, no warning.
  (let ((jaunder--service-doc-cache nil) (fetches 0))
    (cl-letf (((symbol-function 'jaunder--fetch-service-features)
               (lambda (_base) (cl-incf fetches) '("slug"))))
             (let ((jaunder-warn-missing-format-media-type nil))
               (should-not (jaunder-test--capturing-warnings
                            (jaunder--warn-missing-format-media-type "https://blog")))
               (should (= fetches 0))))))

;;; Review follow-ups

(ert-deftest jaunder-parse-service-features-non-service-root-is-unknown ()
  ;; A 2xx HTML/error page from a proxy parses to a non-service root → unknown
  ;; (skip + no cache), not a false "feature absent".
  (should (eq (jaunder--parse-service-features
               "<html><body>Error 500</body></html>")
              'unknown)))

(ert-deftest jaunder-fetch-service-features-non-2xx-is-unknown ()
  ;; AC-216d: a 4xx/5xx status → unknown (never a "feature absent").
  (cl-letf (((symbol-function 'jaunder--http-request)
             (lambda (&rest _) (jaunder-test--response 404 nil "nope"))))
           (should (eq (jaunder--fetch-service-features "https://blog") 'unknown))))

;;; jaunder-service-test.el ends here
