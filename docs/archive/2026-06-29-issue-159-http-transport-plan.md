# C1 — AtomPub HTTP Transport (`jaunder--http-request`) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fill the `jaunder--http-request` seam in `elisp/jaunder.el` — an authenticated `url.el` transport returning a parsed `(:status :headers :body)` plist — and land the shared Unit C spec with this first sub-issue PR.

> **Pivot note (2026-06-29):** This plan was executed on `url.el` first and passed every
> local gate, but the CI `e2e-elisp-integration` VM hit a rare, CI-VM-only 401 traced to
> `url.el` request-header handling. C1 was re-implemented on **`plz` (curl)** — see
> **ADR-0038** for the full root-cause and decision. The `url.el` request code and the
> raw-buffer `jaunder--parse-http-response` shown in Tasks 2–3 below reflect the original
> approach; the **shipped** transport uses `plz` with a `jaunder--plz-response->plist`
> converter (same `(:status :headers :body)` contract). The TDD shape and the spec landing
> (Task 1) are unchanged.

**Architecture:** A pure response parser (`jaunder--parse-http-response`, `jaunder--response-header`) tested serverless with canned response strings, plus `jaunder--http-request` which drives `url.el` (Basic auth via the existing `jaunder--auth-secret`/`jaunder--basic-auth-header`) and feeds the raw response buffer through the pure parser. A live ERT test exercises the whole path against a real server via the #137 harness.

**Tech Stack:** Emacs Lisp (`url.el`, `auth-source`, ERT), the `jaunder-test--with-live-server` harness (#137), the `e2e-elisp-integration` nixosTest (ADR-0035).

## Global Constraints

- **Sub-issue of #74**, Unit C. Design fixed by the approved spec `docs/superpowers/specs/2026-06-29-issue-74-emacs-authoring-publish.md` (C1 row) + the epic spec's "HTTP"/"Publish flow" sections. This PR closes **#159** and lands that shared spec.
- **Elisp**: Emacs 27.1+, `lexical-binding: t`, built-in libraries only (`url`, `auth-source`); formatting enforced by `jaunder-fmt-check` (run `-f jaunder-fmt-fix` before committing). One ERT test per pure function (ADR-0031).
- **No `Co-Authored-By` trailers.** Per-task gate: full `cargo xtask check` before each commit (catches elisp-fmt/ert/coverage; warms the cache). Final gate: `cargo xtask validate` (the live test runs under the `e2e-elisp-integration` nixosTest).
- **Test file conventions**: pure tests in `elisp/test/*-test.el` (run by `run-tests.el`, globs `-test.el`); live tests in `elisp/test/*-integration.el` (run by `run-integration-tests.el`, globs `-integration.el`). No new gate wiring — the live test rides the existing `e2e-elisp-integration` check.
- All paths relative to the worktree root `/home/mdorman/src/jaunder/.claude/worktrees/issue-159-emacs-http-transport`.

---

### Task 1: Land the shared Unit C spec

The spec is already restored in the worktree at
`docs/superpowers/specs/2026-06-29-issue-74-emacs-authoring-publish.md`. Commit it
first so C2–C4 can reference it on `main`.

- [x] **Step 1: Verify the spec file is present**

Run: `ls docs/superpowers/specs/2026-06-29-issue-74-emacs-authoring-publish.md`
Expected: the path prints (no error).

- [x] **Step 2: Commit the spec**

```bash
git add docs/superpowers/specs/2026-06-29-issue-74-emacs-authoring-publish.md
git commit -m "docs: Unit C (emacs authoring/publish) spec (#74)"
```

---

### Task 2: Pure HTTP response parser

**Files:**
- Modify: `elisp/jaunder.el` (replace the `jaunder--http-request` stub region with the parser + header getter; the request fn comes in Task 3)
- Test: `elisp/test/jaunder-test.el`

**Interfaces:**
- Produces: `jaunder--parse-http-response (raw)` → plist `(:status INT :headers ALIST :body STRING)`, where `headers` keys are **lower-cased** header names mapped to their (string) values. `jaunder--response-header (response name)` → the value for `name` (case-insensitive) or nil.

- [x] **Step 1: Write the failing pure tests** — append to `elisp/test/jaunder-test.el`, before `;;; jaunder-test.el ends here`:

```elisp
(ert-deftest jaunder-parse-http-200-headers-and-body ()
  (let ((r (jaunder--parse-http-response
            (concat "HTTP/1.1 200 OK\r\n"
                    "ETag: \"v1\"\r\n"
                    "Content-Type: application/atom+xml\r\n"
                    "\r\n"
                    "<feed/>"))))
    (should (eq (plist-get r :status) 200))
    (should (equal (jaunder--response-header r "ETag") "\"v1\""))
    (should (equal (jaunder--response-header r "content-type") "application/atom+xml"))
    (should (equal (plist-get r :body) "<feed/>"))))

(ert-deftest jaunder-parse-http-location-and-status ()
  (let ((r (jaunder--parse-http-response
            (concat "HTTP/1.1 201 Created\r\n"
                    "Location: /atompub/alice/posts/42\r\n\r\n"))))
    (should (eq (plist-get r :status) 201))
    (should (equal (jaunder--response-header r "location") "/atompub/alice/posts/42"))
    (should (equal (plist-get r :body) ""))))

(ert-deftest jaunder-parse-http-404 ()
  (let ((r (jaunder--parse-http-response "HTTP/1.1 404 Not Found\r\n\r\nnope")))
    (should (eq (plist-get r :status) 404))
    (should (equal (plist-get r :body) "nope"))))

(ert-deftest jaunder-response-header-is-case-insensitive-and-missing-nil ()
  (let ((r (jaunder--parse-http-response "HTTP/1.1 200 OK\r\nX-A: 1\r\n\r\n")))
    (should (equal (jaunder--response-header r "x-a") "1"))
    (should (equal (jaunder--response-header r "X-A") "1"))
    (should (null (jaunder--response-header r "x-missing")))))
```

- [x] **Step 2: Run; expect failure** — `emacs --batch -Q -l elisp/scripts/run-tests.el` → fails (`jaunder--parse-http-response` undefined).

- [x] **Step 3: Implement the parser** — in `elisp/jaunder.el`, replace the `jaunder--http-request` stub (the `(defun jaunder--http-request (&rest _args) … )` form under "Seams") with:

```elisp
(defun jaunder--parse-http-response (raw)
  "Parse RAW HTTP response text into a plist (:status :headers :body).
Header names are downcased.  RAW is the full response (status line,
headers, blank line, body), as left in a `url.el' response buffer."
  (let* ((sep (or (string-match "\r\n\r\n" raw)
                  (string-match "\n\n" raw)))
         (head (substring raw 0 sep))
         (body (if sep (substring raw (match-end 0)) ""))
         (lines (split-string head "\r?\n" t))
         (status (when (string-match "\\`HTTP/[0-9.]+ \\([0-9]+\\)" (car lines))
                   (string-to-number (match-string 1 (car lines)))))
         (headers
          (delq nil
                (mapcar (lambda (l)
                          (when (string-match "\\`\\([^:]+\\):[ \t]*\\(.*\\)\\'" l)
                            (cons (downcase (match-string 1 l)) (match-string 2 l))))
                        (cdr lines)))))
    (list :status status :headers headers :body body)))

(defun jaunder--response-header (response name)
  "Return the value of header NAME (case-insensitive) in RESPONSE, or nil."
  (cdr (assoc (downcase name) (plist-get response :headers))))
```

- [x] **Step 4: Run; expect PASS** — `emacs --batch -Q -l elisp/scripts/run-tests.el` → the 4 new tests pass.

- [x] **Step 5: Format + gate** — `emacs --batch -Q -l elisp/scripts/format.el -f jaunder-fmt-fix`; then `cargo xtask check` (elisp-fmt + ert + coverage all green).

- [x] **Step 6: Commit**

```bash
git add elisp/jaunder.el elisp/test/jaunder-test.el
git commit -m "feat(elisp): pure HTTP response parser for the AtomPub transport (#159)"
```

---

### Task 3: `jaunder--http-request` + live transport test

**Files:**
- Modify: `elisp/jaunder.el` (add `jaunder--http-request` after the parser)
- Test: `elisp/test/jaunder-transport-integration.el` (new)

**Interfaces:**
- Consumes: `jaunder--parse-http-response`, `jaunder--response-header` (Task 2); `jaunder--basic-auth-header`, `jaunder--auth-secret`, `jaunder--build-url` (existing); `jaunder-base-url`/`jaunder-username` (bound by the harness); `jaunder-test--with-live-server` (#137).
- Produces: `jaunder--http-request (method url &optional body content-type)` → the parser's `(:status :headers :body)` plist. Adds the Basic-auth header from `jaunder--auth-secret`; sends `body`/`content-type` for writes; signals on a transport-level failure (no response). HTTP error statuses (4xx/5xx) are returned in `:status` for the caller, not signalled.

- [x] **Step 1: Write the failing live test** — create `elisp/test/jaunder-transport-integration.el`:

```elisp
;;; jaunder-transport-integration.el --- C1 live transport test -*- lexical-binding: t; -*-
;;; Commentary:
;; Exercises `jaunder--http-request' end-to-end against a real server (#137 harness).
;;; Code:
(require 'ert)
(require 'jaunder)
(require 'jaunder-integration-helper)

(ert-deftest jaunder-transport-authed-get-collection ()
  "An authed GET of the posts collection through `jaunder--http-request' is 200."
  (jaunder-test--with-live-server
    (let ((r (jaunder--http-request
              "GET"
              (jaunder--build-url jaunder-base-url "atompub" jaunder-username "posts"))))
      (should (eq (plist-get r :status) 200))
      (should (string-match-p "<feed" (plist-get r :body))))))

(provide 'jaunder-transport-integration)
;;; jaunder-transport-integration.el ends here
```

- [x] **Step 2: Run; expect failure** — `cargo build -p jaunder` then `JAUNDER_TEST_BINARY=$PWD/target/debug/jaunder emacs --batch -Q -l elisp/scripts/run-integration-tests.el` → fails (`jaunder--http-request` still errors as a stub or is undefined).

- [x] **Step 3: Implement `jaunder--http-request`** — add to `elisp/jaunder.el` immediately after `jaunder--response-header`:

```elisp
(defun jaunder--http-request (method url &optional body content-type)
  "Make an authenticated METHOD request to URL, returning a response plist.
METHOD is an HTTP verb string; URL an absolute URL.  BODY (a string) and
CONTENT-TYPE apply to write requests.  Basic-auth credentials come from
`jaunder--auth-secret' for `jaunder-username'.  Returns the
`jaunder--parse-http-response' plist; signals on a transport-level failure.
HTTP error statuses are reported in :status, not signalled."
  (require 'url)
  (let* ((url-request-method method)
         (url-request-extra-headers
          (append (list (jaunder--basic-auth-header jaunder-username
                                                     (jaunder--auth-secret)))
                  (when content-type (list (cons "Content-Type" content-type)))))
         (url-request-data (and body (encode-coding-string body 'utf-8)))
         (buf (url-retrieve-synchronously url t t 30)))
    (unless buf
      (error "jaunder: no response from %s %s" method url))
    (unwind-protect
        (with-current-buffer buf
          (jaunder--parse-http-response
           (buffer-substring-no-properties (point-min) (point-max))))
      (kill-buffer buf))))
```

- [x] **Step 4: Run; expect PASS** — `JAUNDER_TEST_BINARY=$PWD/target/debug/jaunder emacs --batch -Q -l elisp/scripts/run-integration-tests.el` → `jaunder-transport-authed-get-collection` passes (plus the #137 smoke tests).

- [x] **Step 5: Format + full gate** — `emacs --batch -Q -l elisp/scripts/format.el -f jaunder-fmt-fix`; then `cargo xtask validate` (runs the `e2e-elisp-integration` nixosTest, which now includes the transport test).

- [x] **Step 6: Commit**

```bash
git add elisp/jaunder.el elisp/test/jaunder-transport-integration.el
git commit -m "feat(elisp): authenticated AtomPub HTTP transport via url.el (#159)"
```

---

## Self-Review

**Spec coverage (C1 row):** `jaunder--http-request` (Task 3); url.el request/response (Task 3); Basic-auth wiring via `jaunder--auth-secret` (Task 3); status/ETag/Location header parsing (Task 2 `jaunder--parse-http-response` + `jaunder--response-header`); error surfacing (Task 3 — transport failure signals, HTTP status returned). Pure ERT for parsing (Task 2); live ERT through `jaunder--http-request` (Task 3). Shared spec landed (Task 1). ✓

**Placeholder scan:** none — all steps carry concrete elisp + commands.

**Type consistency:** `jaunder--parse-http-response`/`jaunder--response-header` (Task 2) are consumed with the same names + plist shape (`:status`/`:headers`/`:body`) by `jaunder--http-request` (Task 3) and its test. The live test uses the harness-bound `jaunder-base-url`/`jaunder-username` exactly as #137 provides them.
