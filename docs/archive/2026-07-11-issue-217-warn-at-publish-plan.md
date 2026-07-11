# Plan — three "warn at publish" warnings (#217, #206, #216)

**Spec:**
[`2026-07-11-issue-217-warn-at-publish-spec.md`](./2026-07-11-issue-217-warn-at-publish-spec.md).
All behavior, edge cases, and acceptance criteria (AC-\*) live there; this plan
is **how**, not what/why.

## Review header

**Goal.** Add three soft, suppressible, non-blocking publish-time warnings to
the emacs client, establishing the client's `display-warning` idiom. Elisp only.

**Scope — in:** `elisp/jaunder-config.el`, `jaunder-datetime.el`,
`jaunder-media.el`, `jaunder-publish.el`, a new `jaunder-service.el`,
`jaunder.el` (requires), and `elisp/test/jaunder-test.el` (ERT). **Out:** any
server/Rust change; non-image media (#25); cross-session cache persistence; zone
alias normalization; the Rust coverage gate (elisp is exempt — ADR-0031 — but
every new pure fn still gets an ERT test).

**Tasks (one commit per issue, in order):**

1. **#217 zone-mismatch warning** — shared `jaunder--warn` helper +
   `jaunder--zone-offset-p`
   - `jaunder--warn-zone-mismatch`, `jaunder-warn-zone-mismatch` defcustom,
     wired into `jaunder-publish`. (Commit 1.)
2. **#206 untracked-media warning** — `jaunder--git-toplevel`,
   `jaunder--git-tracked-p`, `jaunder--warn-untracked-media`,
   `jaunder-warn-untracked-media` defcustom, called inside
   `jaunder--localize-media`. (Commit 2.)
3. **#216 missing-format-media-type warning** — new `jaunder-service.el`
   (`jaunder--service-doc-cache`, `jaunder--parse-service-features`,
   `jaunder--fetch-service-features`,
   `jaunder--warn-missing-format-media-type`),
   `jaunder-warn-missing-format-media-type` defcustom, required by `jaunder.el`,
   called in `jaunder-publish`. (Commit 3.)

**Key risks / decisions:**

- **Byte-compile is warnings-as-errors** (`scripts/byte-compile.el`). Every
  cross-file function call needs a `(require …)` in the calling module, every
  new fn needs a docstring, and no unused lexical vars — or the gate fails. This
  is the main footgun.
- **The `records` list doesn't escape `jaunder--localize-media`** — #206's check
  lives **inside** that function (spec §#206), not in `jaunder-publish`.
- **`jaunder--http-request` re-signals transport failures** — #216's fetch is
  wrapped in `condition-case` so a down server never aborts the publish (spec
  §#216, AC-216d).
- **Require ordering:** `jaunder--warn` lives in `jaunder-config` (loaded before
  media & publish) so both can call it without a circular require.
  `jaunder-service` is required after `jaunder-transport` in `jaunder.el`, and
  `jaunder-publish` requires `jaunder-service` + `jaunder-datetime` directly
  (byte-compile visibility).

## Global constraints

- **Language:** Emacs Lisp, `lexical-binding: t` (all modules already use it).
- **Warning idiom (spec "Shared design"):**
  `(display-warning 'jaunder MSG :warning)`, every message `jaunder: `-prefixed,
  gated by a per-warning boolean `defcustom` (default `t`), best-effort (never
  errors, never blocks). Centralized in `jaunder--warn`.
- **Tests:** ERT in `elisp/test/jaunder-test.el` (the runner globs
  `test/*-test.el`; this file already matches). Stub side-effecting fns with
  `cl-letf`/`symbol-function` as the existing suite does; capture
  `display-warning` calls into a list.
- **Per-task run (fast):** `emacs --batch -Q -l elisp/scripts/run-tests.el` from
  the worktree root.
- **Commit gate (each task):** `devtool run -- cargo xtask check` — runs `ert`,
  `elisp-fmt` (auto-fix), and `byte-compile` (warnings-as-errors) among the host
  static checks. Must be clean before the commit. **No `Co-Authored-By`
  trailer.** Follow `jaunder-commit`.
- **No placeholders:** every task lands complete, compiling, tested code.

## Test helper (add once, in Task 1)

A capture helper for `display-warning`, at the top of the new test block in
`jaunder-test.el`:

```elisp
(defmacro jaunder-test--capturing-warnings (&rest body)
  "Run BODY with `display-warning' captured; return the list of (TYPE MSG LEVEL)."
  (declare (indent 0))
  `(let (jaunder-test--warnings)
     (cl-letf (((symbol-function 'display-warning)
                (lambda (type message &optional level &rest _)
                  (push (list type message level) jaunder-test--warnings))))
       ,@body)
     (nreverse jaunder-test--warnings)))
```

---

## Task 1 — #217 zone-mismatch warning (Commit 1)

### Files / interfaces

**`elisp/jaunder-config.el`** — add the shared helper and the defcustom:

```elisp
(defcustom jaunder-warn-zone-mismatch t
  "When non-nil, warn at publish if the recorded JAUNDER_DATE_TZ differs from
the machine's current zone.  See `jaunder--warn-zone-mismatch'."
  :type 'boolean
  :group 'jaunder)

(defun jaunder--warn (format-string &rest args)
  "Emit a soft jaunder publish warning; never blocks the publish.
FORMAT-STRING and ARGS are passed to `format'; the message is prefixed
\"jaunder: \" and reported via `display-warning' with type `jaunder'."
  (display-warning 'jaunder
                   (apply #'format (concat "jaunder: " format-string) args)
                   :warning))
```

**`elisp/jaunder-datetime.el`** — add, near `jaunder--current-zone-name`:

```elisp
(defun jaunder--zone-offset-p (zone)
  "Return non-nil when ZONE is a numeric offset like \"-0400\", not an IANA name."
  (and (stringp zone) (string-match-p "\\`[+-][0-9]" zone)))

(defun jaunder--warn-zone-mismatch (recorded)
  "Warn when RECORDED zone differs from the machine's current zone.
RECORDED is the pre-existing JAUNDER_DATE_TZ (nil when unset).  No warning
when RECORDED is nil, equals the current zone, or when both are numeric
offsets (a same-machine DST offset flip is not a real zone change).
Gated by `jaunder-warn-zone-mismatch'."
  (when (and jaunder-warn-zone-mismatch recorded)
    (let ((current (jaunder--current-zone-name)))
      (unless (or (string= recorded current)
                  (and (jaunder--zone-offset-p recorded)
                       (jaunder--zone-offset-p current)))
        (jaunder--warn
         "recorded timezone %s differs from this machine's zone %s; #+DATE: will be interpreted in the recorded zone %s"
         recorded current recorded)))))
```

**Add** `(require 'jaunder-config)` to `jaunder-datetime.el` (it currently
requires only `org`). This is mandatory, not conditional: without it, the
cross-file `jaunder--warn` call and the `jaunder-warn-zone-mismatch` reference
are exactly the undefined-function / free-variable warnings that byte-compile's
warnings-as-errors rejects. No require cycle — `jaunder-config` requires only
`url-parse`.

**`elisp/jaunder-publish.el`** — at the create/update choke point (immediately
before the `(let* ((xml …) (resp (if id …))))` PUT/POST, ~L173), call the
warning with the **pre-existing** recorded `JAUNDER_DATE_TZ` — i.e. the
`tz`-shaped local bound from the buffer property at the top of `jaunder-publish`
(the value read **before** `jaunder--ensure-date-tz` runs). Add
`(jaunder--warn-zone-mismatch <recorded-tz>)`. Confirm `jaunder-publish.el`
requires `jaunder-datetime` (add if the byte-compiler flags
`jaunder--warn-zone-mismatch` as unknown).

### Test (`elisp/test/jaunder-test.el`) — add `jaunder-test--capturing-warnings` + tests

- `jaunder-warn-zone-mismatch-fires-on-difference` (AC-217a): stub
  `jaunder--current-zone-name` → `"Europe/London"`; call
  `(jaunder--warn-zone-mismatch "America/New_York")` inside the capture macro;
  assert one warning, type `jaunder`, message contains both zones.
- `jaunder-warn-zone-mismatch-silent-when-unset` (AC-217b):
  `(jaunder--warn-zone-mismatch nil)` → no warning.
- `jaunder-warn-zone-mismatch-silent-when-equal` (AC-217c, IANA): recorded ==
  stubbed current → no warning.
- `jaunder-warn-zone-mismatch-silent-both-offsets` (AC-217c, offset): recorded
  `"-0500"`, stubbed current `"-0400"` → no warning.
- `jaunder-warn-zone-mismatch-suppressed` (AC-217d): `let`
  `jaunder-warn-zone-mismatch` nil, difference present → no warning.
- `jaunder-zone-offset-p` truth table: `"-0400"`/`"+0000"` → non-nil;
  `"America/New_York"`/`nil` → nil.

### Run

`emacs --batch -Q -l elisp/scripts/run-tests.el` — the five zone tests FAIL
before the datetime/config code, PASS after.

### Commit

`devtool run -- cargo xtask check` clean, then commit **only** Task 1's files
(`jaunder-config.el`, `jaunder-datetime.el`, `jaunder-publish.el`,
`jaunder-test.el`):
`emacs: warn at publish when machine zone differs from recorded JAUNDER_DATE_TZ (#217)`.

---

## Task 2 — #206 untracked-media warning (Commit 2)

### Files / interfaces

**`elisp/jaunder-config.el`** — add:

```elisp
(defcustom jaunder-warn-untracked-media t
  "When non-nil, warn at publish for referenced local media not tracked by git
in the document's repository.  See `jaunder--warn-untracked-media'."
  :type 'boolean
  :group 'jaunder)
```

**`elisp/jaunder-media.el`** — add (git helpers + the check):

```elisp
(defun jaunder--git-toplevel (dir)
  "Return the git work-tree toplevel containing DIR, or nil.
Best-effort: nil when DIR is nil, git is unavailable, or DIR is not in a repo."
  (when (and dir (executable-find "git"))
    (let ((default-directory dir))
      (with-temp-buffer
        (when (zerop (call-process "git" nil (list t nil) nil
                                   "rev-parse" "--show-toplevel"))
          (string-trim (buffer-string)))))))

(defun jaunder--git-tracked-p (toplevel path)
  "Return non-nil when PATH is tracked by git in the TOPLEVEL work tree."
  (let ((default-directory toplevel))
    (zerop (call-process "git" nil nil nil
                         "ls-files" "--error-unmatch" "--" path))))

(defun jaunder--warn-untracked-media (records)
  "Warn once per distinct untracked media `:path' in RECORDS.
Anchored on the repo containing the current buffer's file; skips entirely when
that buffer is not in a git work tree or git is unavailable.  Gated by
`jaunder-warn-untracked-media'."
  (when jaunder-warn-untracked-media
    (let ((toplevel (jaunder--git-toplevel
                     (and buffer-file-name
                          (file-name-directory buffer-file-name)))))
      (when toplevel
        (let (seen)
          (dolist (r records)
            (let ((path (plist-get r :path)))
              (when (and path (not (member path seen)))
                (push path seen)
                (unless (jaunder--git-tracked-p toplevel path)
                  (jaunder--warn
                   "referenced media %s is not tracked by git in this document's repository; a fresh clone will lack local preview"
                   path))))))))))
```

Ensure `jaunder-media.el` requires `jaunder-config` (for `jaunder--warn` +
defcustom).

**Call site:** inside `jaunder--localize-media`, immediately after the
`(jaunder--media-preflight records)` call, add
`(jaunder--warn-untracked-media records)` (same `records` local; the media still
uploads unchanged afterward).

### Test (`elisp/test/jaunder-test.el`)

Stub `jaunder--git-toplevel` and `jaunder--git-tracked-p` (unit-isolate from
real git), build `records` as `(list (list :path "/repo/a.png") …)`:

- `jaunder-warn-untracked-media-one-per-untracked` (AC-206a): toplevel
  `"/repo"`, `git-tracked-p` returns t for `a.png`, nil for `b.png`; two records
  → exactly one warning naming `b.png`.
- `jaunder-warn-untracked-media-all-tracked` (AC-206d): tracked-p → t for all →
  no warning.
- `jaunder-warn-untracked-media-skips-non-repo` (AC-206e): `git-toplevel` → nil
  → no warning even with an untracked record.
- `jaunder-warn-untracked-media-suppressed` (AC-206f): defcustom nil → no
  warning.
- `jaunder-warn-untracked-media-dedups` (AC-206g): two records with the **same**
  untracked `:path` → exactly one warning.
- `jaunder-git-tracked-p-real-repo` (AC-206b/c, deterministic — guarded by
  `(skip-unless (executable-find "git"))`): build a real temp git repo via
  `call-process` (`git init`, commit one file `tracked.png`, add a `.gitignore`
  entry for `ignored.png` and create it, and create a file **outside** the repo
  tree). Assert `jaunder--git-tracked-p` → non-nil for `tracked.png`, and
  **nil** for the gitignored file (AC-206b) and the outside-tree file (AC-206c).
  This pins that `git ls-files --error-unmatch` actually exits non-zero for
  both, rather than assuming it. Also assert `jaunder--git-toplevel` resolves a
  subdirectory of the repo to the repo root.

### Run

`emacs --batch -Q -l elisp/scripts/run-tests.el` — FAIL before, PASS after.

### Commit

`devtool run -- cargo xtask check` clean, commit Task 2's files
(`jaunder-config.el`, `jaunder-media.el`, `jaunder-test.el`):
`emacs: warn at publish when referenced local media isn't tracked in git (#206)`.

---

## Task 3 — #216 missing-format-media-type warning (Commit 3)

### Files / interfaces

**`elisp/jaunder-config.el`** — add:

```elisp
(defcustom jaunder-warn-missing-format-media-type t
  "When non-nil, warn at publish if the server's service document does not
advertise the `format-media-type' feature.  See
`jaunder--warn-missing-format-media-type'."
  :type 'boolean
  :group 'jaunder)
```

**`elisp/jaunder-service.el`** (new module; `provide` + requires
`jaunder-config`, `jaunder-transport`, and `dom`):

```elisp
(defvar jaunder--service-doc-cache nil
  "Session-scoped alist of BASE-URL -> list of advertised feature tokens.
Populated on first successful service-doc fetch per base-url; failures are not
cached so a later publish may retry.")

(defun jaunder--parse-service-features (body)
  "Parse service-doc BODY; return the list of feature tokens, `()' when the doc
parses but advertises none, or the symbol `unknown' when BODY is not parseable
XML.  Matches the extension element by local name (libxml folds the `j:' prefix,
as `jaunder--harvest-response-fields' already relies on) and splits its
`features' attribute on whitespace."
  (with-temp-buffer
    (insert (or body ""))
    (let ((dom (libxml-parse-xml-region (point-min) (point-max))))
      (if (null dom)
          ;; libxml returns nil (no signal) on empty/garbage bodies — best-effort
          ;; treats an unparseable service doc as unknown, not "feature absent".
          'unknown
        (let* ((ext (car (dom-by-tag dom 'extension)))
               (features (and ext (dom-attr ext 'features))))
          (if features (split-string features) '()))))))

(defun jaunder--fetch-service-features (base-url)
  "Fetch + parse the service doc for BASE-URL; return a feature-token list, `()',
or the symbol `unknown' on any transport/non-2xx/parse failure (never signals)."
  (condition-case nil
      (let* ((resp (jaunder--http-request
                    "GET" (jaunder--build-url base-url "atompub" "service")))
             (status (plist-get resp :status)))
        (if (and (integerp status) (<= 200 status 299))
            (jaunder--parse-service-features (plist-get resp :body))
          'unknown))
    (error 'unknown)))

(defun jaunder--warn-missing-format-media-type (base-url)
  "Warn once per session per BASE-URL if the service doc lacks
`format-media-type'.  Fetches + caches on first call per base-url; a cache hit
does nothing (no fetch, no warning).  Gated by
`jaunder-warn-missing-format-media-type'."
  (when (and jaunder-warn-missing-format-media-type
             (not (assoc base-url jaunder--service-doc-cache)))
    (let ((features (jaunder--fetch-service-features base-url)))
      (unless (eq features 'unknown)
        (push (cons base-url features) jaunder--service-doc-cache)
        (unless (member "format-media-type" features)
          (jaunder--warn
           "server at %s does not advertise the format-media-type feature; it may store this post's org source verbatim instead of rendering it"
           base-url))))))
```

Verify the exact `jaunder--http-request` arg count/signature at implementation
and match it (GET with nil body/content-type/headers).

**`elisp/jaunder.el`** — add `(require 'jaunder-service)` after
`(require 'jaunder-transport)`.

**`elisp/jaunder-publish.el`** — add `(require 'jaunder-service)`; at the choke
point (~L173, alongside the Task 1 call), add
`(jaunder--warn-missing-format-media-type (jaunder--active-base-url))`.

### Test (`elisp/test/jaunder-test.el`)

- `jaunder-parse-service-features-*`: feed a service-doc XML string with
  `<j:extension features="format-media-type slug"/>` →
  `("format-media-type" "slug")`; a doc that parses but lacks the attr/element →
  `()` (empty list, **not** `unknown`); a doc where `format-media-type` appears
  only in an unrelated text node → **not** in the returned list (AC-216e); a
  **non-XML / garbage body** → `'unknown` (AC-216d, the libxml-returns-nil
  case).
- `jaunder-warn-missing-format-media-type-fires` (AC-216a): reset
  `jaunder--service-doc-cache`; stub `jaunder--fetch-service-features` →
  `("slug")` (no format-media-type); one warning naming the base-url.
- `jaunder-warn-missing-format-media-type-caches` (AC-216b): stub `fetch` to
  also `cl-incf` a counter; call the warn twice for the same base-url →
  warning + fetch happen once.
- `jaunder-warn-missing-format-media-type-present` (AC-216c): stub `fetch` →
  `("format-media-type" "slug")` → no warning.
- `jaunder-warn-missing-format-media-type-unknown-not-cached` (AC-216d): stub
  `fetch` → `'unknown` → no warning **and** `jaunder--service-doc-cache` still
  lacks the base-url (retry allowed).
- `jaunder-fetch-service-features-catches-signal` (AC-216d seam): stub
  `jaunder--http-request` to `(error "boom")` → returns `'unknown`, does not
  signal.
- `jaunder-warn-missing-format-media-type-suppressed` (AC-216f): defcustom nil →
  no fetch (stub `fetch` with a counter; assert 0 calls) and no warning.

Each cache-touching test `let`-binds or resets `jaunder--service-doc-cache` to
nil so tests don't leak state.

**Cross-cutting shared-idiom tests (added here, since all three warnings now
exist):**

- `jaunder-publish-request-identical-with-warnings` (AC-S1): in a
  `with-temp-buffer` org fixture wired into a `let`-bound
  `jaunder--active-blog`, stub `jaunder--http-request` to capture the request
  `body` and return a canned 201; stub the warning inputs so every warning
  _would_ fire (recorded-tz differs, an untracked media path, service features
  lacking format-media-type). Run `jaunder-publish` once with all three
  defcustoms `t` and once with all three `nil`; assert the captured request
  bodies are **byte-identical** and the return value/`JAUNDER_*` write-back
  match. (This is the concrete AC-S1 assertion the isolated helper tests do not
  provide.)
- `jaunder-publish-warnings-are-independent` (AC-S2): with all three enabled,
  suppress exactly one defcustom and assert the other two still fire (drive the
  three warn helpers under one `jaunder-test--capturing-warnings`, asserting the
  surviving two messages).

### Run

`emacs --batch -Q -l elisp/scripts/run-tests.el` — FAIL before, PASS after.
Optionally add a live assertion in `elisp/test/jaunder-publish-integration.el`
that publishing against the harness (which **advertises** format-media-type)
yields no such warning (AC-216c live).

### Commit

`devtool run -- cargo xtask check` clean, commit Task 3's files
(`jaunder-config.el`, `jaunder-service.el`, `jaunder.el`, `jaunder-publish.el`,
`jaunder-test.el`):
`emacs: warn at publish when service-doc omits format-media-type (#216)`.

## Self-review

- Every AC in the spec maps to at least one named test above. AC-S1 is the
  dedicated `jaunder-publish-request-identical-with-warnings` publish-path test
  (not the isolated helper tests); AC-S2 is
  `jaunder-publish-warnings-are-independent` plus the three per-warning
  suppression tests; AC-S3 by asserting type `jaunder` / `:warning` /
  `jaunder: ` prefix in the Task 1 fire test. AC-216d covers both the
  transport-signal path (`jaunder-fetch-service-features-catches-signal`) and
  the unparseable-2xx-body path (the `'unknown` parse test); AC-206b/c are the
  deterministic real-git-repo test.
- Each task is independently verifiable (its own ERT tests +
  `cargo xtask check`) and lands as one commit; the order 1→2→3 introduces
  `jaunder--warn` before its reuse.
- No separable follow-on concerns surfaced during design that need filing (the
  epic already tracks #25 non-image media, #82 elisp coverage).
