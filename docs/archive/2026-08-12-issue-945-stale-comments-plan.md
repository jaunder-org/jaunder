# Plan — issue #945: five stale in-source comments

**Spec:**
[`docs/superpowers/specs/2026-08-12-issue-945-stale-comments.md`](../specs/2026-08-12-issue-945-stale-comments.md)
**Issue:** [#945](https://github.com/jaunder-org/jaunder/issues/945) **Branch:**
`issue-945-stale-comments` (base tag `issue-945-stale-comments-base`) **For
agentic workers:** drive with **`jaunder-iterate`**; delegate an individual task
via **`jaunder-dispatch`** when useful. Commit per **`jaunder-commit`**.

## Review header

**Goal.** Correct five stale in-source comments (spec items A–E) plus two extra
prose copies of item B's error, and fix the one real behaviour defect the
comments exposed (item E, the elisp retry handler).

**Scope — in:** `host/src/metrics.rs`,
`xtask/src/server_fn_coverage/extract.rs`, `web/src/audiences/component.rs`,
`flake.nix`, `elisp/jaunder-publish.el`, `elisp/test/jaunder-test.el`,
`docs/observability.md`, `docs/adr/0081-empirical-server-fn-flow-coverage.md`.

**Scope — out:** `docs/ARCHITECTURE.md`, ADR-0011/ADR-0052 (corrected by #927),
`CONTRIBUTING.md` and `tools/devtool/src/check.rs` counts (correct today),
`docs/archive/`, the wider #930 population. **No separable concerns to file** —
the spec review surfaced none beyond the two extra item-B copies, which are
folded in rather than deferred.

**Tasks.**

- [x] **1** Item E — narrow the retry handler to `plz-error`, tests first
      (AC-7…AC-10).
- [x] **2** Item A — `host/src/metrics.rs` module doc states the real mechanism
      (AC-1).
- [x] **3** Item B code — the four `extract.rs` sites credit `#[macros::server]`
      (AC-2/AC-3).
- [x] **4** Item B prose — `docs/observability.md` edit + ADR-0081 dated
      annotation (AC-2/AC-4).
- [x] **5** Items C + D — `component.rs` names a symbol that exists; `flake.nix`
      drops the count (AC-5/AC-6).
- [x] **6** Full gate + acceptance sweep (AC-11).

**Key risks / decisions.**

- **Task 1 is the only behaviour change.** Everything else is comment text. Its
  risk is over-narrowing: `condition-case` on `plz-error` must catch `plz`'s
  `plz-curl-error` / `plz-http-error` subtypes by hierarchy. Task 1's first step
  proves this with a test rather than assuming it.
- **Task 3's replacement text must not narrate the change.**
  `CONTRIBUTING.md:898-907` forbids "used to" / "no longer" — the very rule this
  issue exists to enforce. Writing "the gate no longer writes the name" would
  create a fresh instance of the defect being fixed.
- **Comments cite a name or a file, never a line number.** New text says
  `atompub_op` in `server/src/atompub/mod.rs`, not `…/mod.rs:72`. A line number
  in a comment is the same drift shape as item D's count. Not a Rust path
  either: `atompub_op` is private and `host` does not depend on `server`, so
  `server::atompub::atompub_op` would resolve for nobody — see Task 2.

## Global constraints

- **No `Co-Authored-By` trailer** on any commit.
- Before each commit run `devtool run -- cargo xtask check` (the pre-commit hook
  runs it anyway; running first keeps the hook clean). **Stage, then commit** —
  never `git commit -- <paths>`.
- Elisp checks are `devtool check ert` and `devtool check byte-compile`;
  markdown is `devtool check prettier`. All three are inside
  `cargo xtask check`.
- Tasks 2–5 change **no** behaviour. If any of them makes a test fail, the edit
  went beyond a comment — stop and re-read.

---

## Task 1 — Item E: narrow the retry handler to `plz-error`

**Files**

- Test: `elisp/test/jaunder-test.el` (in-file ert, the elisp convention)
- Impl: `elisp/jaunder-publish.el`

**Step 1.1 — rewrite the exhaustion test to signal a real `plz-error` (RED).**

`jaunder-create-retry-exhausts-on-transport-error` (`:1201`) currently signals
`(error "boom")`. Replace its stub's body so it signals the condition a
transport failure actually raises:

```elisp
(ert-deftest jaunder-create-retry-exhausts-on-transport-error ()
  ;; AC-C5: after 3 transport failures the publish errors. The stub signals a
  ;; `plz-error' subtype because that is what `jaunder--http-request' re-signals
  ;; when a transport failure carries no response.
  (let ((calls 0))
    (cl-letf (((symbol-function 'sleep-for) (lambda (&rest _) nil))
              ((symbol-function 'jaunder--http-request)
               (lambda (&rest _)
                 (setq calls (1+ calls))
                 (signal 'plz-curl-error (list "Curl error" (make-plz-error))))))
             (should-error (jaunder--create-with-retry "http://x/posts" "<xml/>"))
             (should (= calls 3)))))
```

Two deliberate choices here:

- **`plz-curl-error`, not `plz-error`** — a subtype, so the test proves the
  hierarchy match the risk note calls out. It is also what really arrives:
  `jaunder-transport.el:123` re-signals `(car err)`, i.e. whichever subtype plz
  raised, never bare `plz-error`.
- **The data shape mirrors plz's own** (`plz.el:621` signals
  `(list "Curl error" <plz-error struct>)`). `jaunder--create-with-retry` only
  passes `(car err)` / `(cdr err)` through, so a bare `'("curl failed")` would
  pass too — but a fixture that lies about the shape misleads the next person to
  extend the handler to inspect `plz-error-response`. If `make-plz-error` needs
  slot arguments to construct, give it none and let the defaults stand.

**Step 1.2 — add the config-error test (RED).** Append after it:

```elisp
(ert-deftest jaunder-create-retry-does-not-retry-a-config-error ()
  ;; #945 item E: a non-transport error (e.g. no auth-source entry) cannot
  ;; succeed on retry, so it surfaces on the first attempt with no backoff.
  (let ((calls 0)
        (sleeps 0))
    (cl-letf (((symbol-function 'sleep-for) (lambda (&rest _) (setq sleeps (1+ sleeps))))
              ((symbol-function 'jaunder--http-request)
               (lambda (&rest _)
                 (setq calls (1+ calls))
                 (error "jaunder: no auth-source entry for a@b"))))
             (should-error (jaunder--create-with-retry "http://x/posts" "<xml/>"))
             (should (= calls 1))
             (should (= sleeps 0)))))
```

**Run (expect FAIL):** `devtool run -- devtool check ert`

Step 1.2's test fails (3 calls, 2 sleeps). Step 1.1's should already pass —
`(error …)` and `plz-curl-error` are both caught by the current bare handler; it
is a guard against the coming narrowing, so a PASS here is correct.

**Step 1.3 — narrow the handler (GREEN).** In `jaunder--create-with-retry`
(`elisp/jaunder-publish.el:165-168`), change the `condition-case` handler
condition from `error` to `plz-error`:

```elisp
      (let ((r (condition-case err
                   (jaunder--http-request "POST" url xml jaunder--entry-content-type
                                          (list (cons "Idempotency-Key" key)))
                 (plz-error (if (< attempt 3) 'retry (signal (car err) (cdr err)))))))
```

Nothing else in the function changes. **The docstring is not edited** — AC-10:
narrowing is what makes the existing docstring true.

**Run (expect PASS):** `devtool run -- devtool check ert`

All four retry tests pass, including the untouched 5xx (`:1171`) and 4xx
(`:1189`) cases (AC-9).

**`plz-error` is already visible — no require to add.** `jaunder-publish.el:31`
requires `jaunder-transport`, which requires `plz` at `:28`; the test file
already loads `plz` transitively (it calls `make-plz-response` at `:45`). And
`plz.el:108-110` defines the hierarchy with `define-error`: `plz-curl-error` and
`plz-http-error` both have parent `plz-error`, whose own parent is `error` — so
`condition-case` catches both subtypes and `should-error`'s default `:type`
still matches.

Do **not** try to prove this with `byte-compile`: a `condition-case` handler
condition is not a variable reference, so the byte-compiler emits no warning for
an unknown condition symbol. A green byte-compile would prove nothing. The
require chain above is the proof; `ert` is the check.

**Run:** `devtool run -- devtool check byte-compile`

**Commit:** `fix(elisp): retry only transport errors, not configuration errors`

---

## Task 2 — Item A: `host/src/metrics.rs` module doc

**Files:** `host/src/metrics.rs` (lines 1–10, the `//!` block)

Replace the mechanism sentence at `:4-5`. Current:

> Helper arguments are bounded enums, so a call site can never emit an unbounded
> attribute.

Replacement (mirrors `docs/ARCHITECTURE.md:1175-1179`):

```rust
//! Helper arguments are bounded enums, or a `&'static str` drawn from a closed
//! set the call site cannot widen — `atompub_request`'s `op` comes from
//! `atompub_op` in `server/src/atompub/mod.rs`, a matched-route-plus-method
//! lookup, not from an enum. Either way no call site can attach
//! caller-supplied text as a label.
```

**Not** `server::atompub::atompub_op`: `atompub_op` is private and `host` does
not depend on `server`, so that path resolves for nobody — naming an
unfollowable path is the defect class item C is fixing. A file reference is what
`docs/ARCHITECTURE.md:1177` itself uses. It is a path without a line number, so
it does not carry the drift the risk note warns about.

Leave the rest of the module doc (the ADR-0058 / issue #345 paragraph) intact.

**Run:** `devtool run -- cargo xtask check --no-test` — expect PASS, no
behaviour change.

**Commit:**
`docs(host): state the real cardinality mechanism in the metrics doc`

---

## Task 3 — Item B (code): the four `extract.rs` sites

**Files:** `xtask/src/server_fn_coverage/extract.rs`

Four edits. In each, the subject changes from the gate to the macro; the
surrounding reasoning is preserved.

**3a — module doc, `:28-29`.** Currently: "`server-fn-tracing` writes
`web.<vertical>.<ident>` today (#511)". Replace the attribution only:

> `#[macros::server]` derives `web.<vertical>.<ident>` (#714); omitting the
> explicit `name` derives `__server_<ident>`, since …

The paragraph's opening claim — "**The name is matched forward, never
inverted**, because this repo has already had two naming regimes and could have
a third" — **stays verbatim**. It is a present-tense robustness argument, the
spec's explicit carve-out.

**3b — const doc, `:71-72`** (`EXPLICIT_SPAN_PREFIX`). "The prefix on the span
names `server-fn-tracing` writes (#511, ADR-0011)" →

> The prefix on the span names `#[macros::server]` derives (#714):
> `web.<vertical>.<ident>`, where the vertical is the module's first segment.

**3c — fn doc, `:109-110`.** "`web.<vertical>.<ident>` — what
`server-fn-tracing` writes today (#511, ADR-0011)." →

> `web.<vertical>.<ident>` — what `#[macros::server]` derives (#714).

The bullet's continuation at `:110-112` ("The vertical is the module's first
segment, so `posts::api::listing` and `posts::api` both yield `web.posts.…`;
that collapse is why the module check below, not the name, is what actually
disambiguates") is **unchanged** — only the attribution clause moves.

**3d — inline comment in the test at `:454-456`.** "Today `server-fn-tracing`
writes `web.<vertical>.<ident>` (#511)" → "Today `#[macros::server]` derives
`web.<vertical>.<ident>` (#714)". The rest of that comment — the three regimes
and why matching one shape only killed the signal — is unchanged.

**Citation rule (AC-2).** Each site drops `ADR-0011` for the _name_ or points at
its #714 addendum; none may say "used to write", "no longer writes", or "the
gate stopped" (AC-3).

**Run:** `devtool run -- cargo xtask check --no-test` — expect PASS.

**Self-check before commit:**
`rg -n 'server-fn-tracing' xtask/src/server_fn_coverage/extract.rs` — every
surviving hit must describe the gate's _current_ three rules or its name only,
never span-name authorship.

**Commit:** `docs(xtask): credit the macro, not the gate, for the span name`

---

## Task 4 — Item B (prose): `docs/observability.md` + ADR-0081

**Files:** `docs/observability.md`,
`docs/adr/0081-empirical-server-fn-flow-coverage.md`

**4a — `docs/observability.md:246-247`.** A live doc, so edit directly:
"`server-fn-tracing` writes `web.<vertical>.<ident>` today (#511, ADR-0011)" →
"`#[macros::server]` derives `web.<vertical>.<ident>` today (#714)". The
surrounding paragraph — forward matching, the silent-failure history, the
`each_signal_finds_fns_on_its_own_in_the_real_capture` pointer — is unchanged.

**4b — ADR-0081, after `:78`.** The bullet whose `:64` carries the stale claim
runs to `:69`, and its continuation paragraph (the cautionary tale, "The first
implementation compared the bare ident…") runs `:71-78`. Insert **after `:78`**
— inserting after `:69` would split the bullet from its own continuation. A
decision record: **annotate, do not rewrite** (AC-4). Follow the form of
`docs/adr/0052-devtool-unifies-static-checks.md:55-61`:

```markdown
> **Annotation (2026-08-12).** `server-fn-tracing` no longer authors the span
> name; `#[macros::server]` derives it (#714, ADR-0011's 2026-07-30 addendum).
> The gate survives as the recordable-type default-deny and its siblings. The
> decision this ADR records — match the name forward from the inventory, never
> invert it — is unchanged, and the reason is stronger: the regime moved again.
```

The narrative tense is correct **here**; `CONTRIBUTING.md:898-907` governs code
comments, and an ADR annotation exists precisely to record that the world moved.

**Run:** `devtool run -- devtool check prettier` — expect PASS (run
`prettier -w` on both files first if it reflows).

**Commit:**
`docs: correct the span-name attribution in observability.md and ADR-0081`

---

## Task 5 — Items C + D: two one-line factual corrections

**Files:** `web/src/audiences/component.rs`, `flake.nix`

**5a — `web/src/audiences/component.rs:48`.** `Invalidator::patched` does not
exist. Replace with the real symbol, which is the free function called at `:54`:

```rust
    // and `patch`ed in place on success (`client::reactive::patched` owns the plumbing) — so
```

**5b — `flake.nix:1146`.** "The 7 non-compiling static checks (#188), unified
behind one `devtool …`" → "The non-compiling static checks (#188), unified
behind one `devtool …`". Drop the number; keep the issue reference (AC-6).

**Run:** `devtool run -- cargo xtask check --no-test` — expect PASS.

**Self-check:**
`rg -n '7 non-compiling' flake.nix tools/ xtask/ CONTRIBUTING.md` returns
nothing.

**Commit:** `docs: name a symbol that exists, drop a count that drifts`

---

## Task 6 — Full gate and acceptance sweep

**Run:** `devtool run -- cargo xtask validate --no-e2e` (Bash background mode —
this is a long, possibly cold run).

Expect green, including `ert`, `byte-compile`, `prettier`, `fmt`, and clippy
(AC-11). On failure, read `.xtask/last-result.json`'s `steps[]` rather than
scraping the log.

**Then walk AC-1 … AC-11 in the spec and confirm each**, in particular:

- AC-2 —
  `rg -n 'server-fn-tracing' xtask/src/server_fn_coverage/ docs/observability.md`:
  no span-name authorship claim survives. Expect **zero** hits in `extract.rs`
  and **one** in `docs/observability.md` (the corrected sentence, which still
  names the gate).
  - **ADR-0081 is excluded from this grep on purpose.** Its `:64` bullet still
    reads "`server-fn-tracing` … _writes_ `web.<vertical>.<ident>`" and **must**
    — AC-4 preserves decision prose, and the annotation is what corrects it.
    Grepping it here would force a false choice between AC-2 and AC-4. Verify
    ADR-0081 by AC-4 (the annotation exists, in the ADR-0052 form) instead.
- AC-3 — no replacement comment contains "used to", "no longer", or "stopped".
- AC-6 — `rg -n '7 non-compiling' flake.nix tools/ xtask/ CONTRIBUTING.md` is
  empty.
- AC-9 — the 5xx and 4xx retry tests are byte-identical to their fork-point
  versions:
  `git diff issue-945-stale-comments-base..HEAD -- elisp/test/jaunder-test.el`
  shows changes only to the exhaustion test plus the new one.

No commit — this task gates the handoff to **`jaunder-ship`**.

---

## Self-review

- **Every AC maps to a task.** AC-1→2; AC-2→3,4; AC-3→3; AC-4→4; AC-5→5; AC-6→5;
  AC-7…AC-10→1; AC-11→6.
- **Each task is independently verifiable** by the run step named in it, and
  each is one commit.
- **Nothing smuggled in.** Tasks 2–5 touch comment and prose text only; task 1
  changes one `condition-case` condition and two tests. No task edits a file
  listed as out of scope.
