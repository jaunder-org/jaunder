# Plan — issue #292: structurally exempt `unreachable!("msg")` from the coverage gate

Spec:
[`2026-07-05-issue-292-unreachable-coverage-exempt.md`](../specs/2026-07-05-issue-292-unreachable-coverage-exempt.md)
· Issue: [#292](https://github.com/jaunder-org/jaunder/issues/292) · Blocks #245

## Review header

**Goal.** Add a third structural coverage exemption to `xtask`: a literal
`unreachable!("msg")` invocation (non-empty message) drops from the executable
set with no `cov:ignore` marker — self-enforcing, message-required, fail-closed.

**Scope.**

- _In:_ `xtask/src/coverage/exempt.rs` (new `visit_macro` arm + tests), doc-only
  rewords in `gate.rs` and `mod.rs::failure_report`, and updates to
  `CONTRIBUTING.md` and `docs/adr/0050-stateless-coverage-gate.md`.
- _Out:_ #245's dead-line burn-down; any reason-tagging refactor of
  `exempt_lines`; `panic!`/`todo!`/`unimplemented!`/`std::unreachable!` (stay
  measured, by design).

**Tasks.**

1. TDD the `unreachable!` exemption in `exempt.rs` (tests first, then the
   visitor arm).
2. Generalize the A1-guard wording in `gate.rs` docs and
   `mod.rs::failure_report`.
3. Update `CONTRIBUTING.md` coverage section (two → three exemptions).
4. Amend `docs/adr/0050-stateless-coverage-gate.md` (rationale + honest
   trade-off).
5. Full-gate verification (`cargo xtask check`) and self-coverage confirmation.

**Key risks / decisions.**

- **Span breadth:** exempt both `mac.span()` and `mac.tokens.span()` so a
  multi-line message is fully covered even if `mac.span()` degrades to the path
  line on stable proc-macro2. Task 1 has an explicit multi-line test to lock
  this.
- **Recognition boundary:** `mac.path.is_ident("unreachable")` — single-segment
  literal only. `std::unreachable!` and aliases stay measured (fail-closed);
  locked by a test.
- **No behavior change in the gate:** `evaluate` already treats every exempt
  line identically, so the A1-guard "just works" for the new kind; tasks 2 touch
  only doc comments and report strings, preserving substrings existing tests
  assert (`"A1-guard"`, `"revisit the exemption"`).
- **Self-coverage:** the new visitor arm is exercised by task 1's tests; the doc
  rewords sit inside already-tested branches. Task 5 confirms the coverage gate
  stays green.

**For agentic workers.** Execute with **`jaunder-iterate`** (delegate an
individual task via **`jaunder-dispatch`** when useful), ticking checkboxes in
real time. Commit per **`jaunder-commit`** after `cargo xtask check` passes
clean.

## Global constraints

- **Language/crate:** Rust, crate `xtask`. Tests are **in-file** `#[cfg(test)]`
  (the existing convention in `exempt.rs` / `gate.rs` / `mod.rs`) — not
  `server/tests/*`, not dual-backend (no storage involved).
- **Per-task loop:** write/adjust test → run it (expected FAIL) → implement →
  run (expected PASS) → `cargo xtask check` → commit (`jaunder-commit`, **no
  `Co-Authored-By` trailer**).
- **Run via `devtool run`** (worktree-aware, honest exit): e.g.
  `devtool run -- cargo nextest run -p xtask coverage::exempt`. Filter the
  parked log in a second step; never pipe in one shot.
- **Fail-closed is sacred:** every unrecognized/unparseable form must leave
  lines measured. No task may add a path that silently exempts.
- **Docs:** run `prettier -w` on edited Markdown before staging (pre-commit
  prettier restages prose otherwise).

---

## Task 1 — `unreachable!("msg")` exemption in `exempt.rs` (TDD)

**Files.**

- `xtask/src/coverage/exempt.rs` — extend `impl Visit for ExemptVisitor` with
  `visit_macro`; update the module doc header (no longer `#[component]`-only);
  add tests.

**Interfaces.** `exempt_lines(src: &str) -> syn::Result<BTreeSet<u32>>` —
**unchanged**. `unreachable!` lines join the same returned set.

**Step 1a — tests first (expected FAIL).** Add to the `#[cfg(test)] mod tests`
block:

```rust
#[test]
fn exempts_unreachable_with_message() {
    let src = "\
fn pick(n: u8) -> u8 {
    match n {
        0 => 1,
        _ => unreachable!(\"caller guarantees n == 0\"),
    }
}
";
    let ex = exempt_lines(src).unwrap();
    // The `unreachable!(\"...\")` line (line 4) must be exempt.
    assert!(ex.contains(&4), "unreachable! with message exempt: {ex:?}");
}

#[test]
fn does_not_exempt_bare_unreachable() {
    let src = "\
fn pick(n: u8) -> u8 {
    match n {
        0 => 1,
        _ => unreachable!(),
    }
}
";
    let ex = exempt_lines(src).unwrap();
    // Message-required: bare unreachable!() stays measured.
    assert!(!ex.contains(&4), "bare unreachable!() must stay measured: {ex:?}");
}

#[test]
fn does_not_exempt_panic_or_todo() {
    let src = "\
fn a() { panic!(\"boom\"); }
fn b() { todo!(); }
fn c() { unimplemented!(\"later\"); }
";
    let ex = exempt_lines(src).unwrap();
    // panic!/todo!/unimplemented! are NOT unreachable! — stay measured.
    assert!(ex.is_empty(), "panic!/todo!/unimplemented! stay measured: {ex:?}");
}

#[test]
fn exempts_multiline_unreachable_message_span() {
    let src = "\
fn pick(n: u8) -> u8 {
    match n {
        0 => 1,
        _ => unreachable!(
            \"caller guarantees n == 0 for this arm\",
        ),
    }
}
";
    let ex = exempt_lines(src).unwrap();
    // Every line of the multi-line invocation (4..=6) must be exempt.
    assert!(ex.contains(&4), "macro-open line exempt: {ex:?}");
    assert!(ex.contains(&5), "message line exempt: {ex:?}");
    assert!(ex.contains(&6), "macro-close line exempt: {ex:?}");
}

#[test]
fn does_not_exempt_std_unreachable() {
    let src = "\
fn pick(n: u8) -> u8 {
    match n {
        0 => 1,
        _ => std::unreachable!(\"path-qualified\"),
    }
}
";
    let ex = exempt_lines(src).unwrap();
    // Fail-closed boundary: only the single-segment literal matches.
    assert!(!ex.contains(&4), "std::unreachable! must stay measured: {ex:?}");
}
```

(`parse_error_yields_empty` already asserts fail-closed on unparseable input; no
new parse-error test needed.)

**Run (FAIL):** `devtool run -- cargo nextest run -p xtask coverage::exempt` —
the five new tests fail (visitor has no `unreachable!` arm yet); grep the parked
log for `exempts_unreachable_with_message`.

**Step 1b — implement (expected PASS).** Add the arm to
`impl<'ast> syn::visit::Visit<'ast> for ExemptVisitor<'_>`:

```rust
/// A literal `unreachable!(<non-empty message>)` invocation is dropped from the
/// executable set — self-enforcing (reaching it panics ⇒ test fails ⇒ no report),
/// message-required (bare `unreachable!()` stays measured), fail-closed
/// (`std::unreachable!`/aliases/macro-generated forms are not `is_ident("unreachable")`
/// → stay measured).
fn visit_macro(&mut self, mac: &'ast syn::Macro) {
    if mac.path.is_ident("unreachable") && !mac.tokens.is_empty() {
        add_span(self.out, mac.span());        // path + bang + delimiters
        add_span(self.out, mac.tokens.span()); // the (possibly multi-line) message
    }
    syn::visit::visit_macro(self, mac);
}
```

Update the module `//!` header: the exemption set is now `#[component]` bodies
**and** message-carrying `unreachable!` invocations; keep the fail-closed
framing.

**Run (PASS):** same nextest command; all `coverage::exempt` tests pass.

**Commit** (after `cargo xtask check` clean):
`feat(coverage): exempt unreachable!("msg") from the gate (#292)`.

## Task 2 — generalize A1-guard wording (`gate.rs` + `mod.rs`)

**Files.**

- `xtask/src/coverage/gate.rs` — `//!` module doc + `evaluate` `///` doc: name
  both kinds ("a `#[component]` body or an `unreachable!` assertion"). **Logic
  untouched.**
- `xtask/src/coverage/mod.rs` — `failure_report`: reword the guard **header**
  (line ~181) and **guidance** (lines ~208–213) to cover both kinds, preserving
  the substrings `"A1-guard"` and `"revisit the exemption"` that existing tests
  assert. Also generalize the module `//!` guard sentence (line ~6) and the
  `uncovered (...)` label if it names the exemption kinds.

**Interfaces.** No signature changes. `Verdict` / `Fail` / `evaluate` unchanged.

**Guard header reword (illustrative):**

```rust
s.push_str("\n  A1-guard — covered line inside a #[component] or unreachable! span:");
```

**Guidance reword (illustrative, keeps `revisit the exemption`):**

```rust
s.push_str(
    "\n  → a #[component] body or an `unreachable!` assertion is being exercised — the\
     \n    exemption is discarding REAL coverage (or an 'unreachable' line was reached);\
     \n    revisit the exemption (spec §A1-guard).",
);
```

**Run:** `devtool run -- cargo nextest run -p xtask coverage::mod` (or the `mod`
test module) and `coverage::gate` — existing tests
(`failure_report_lists_uncovered_guard_and_crap`,
`failure_report_guidance_is_category_conditional`,
`failure_report_caps_long_lists`) must still PASS unchanged. If the reword adds
no new branch, **no new test**; if the build/coverage flags any new uncovered
line in `failure_report`, add a targeted `failure_report` assertion so the
change covers its own lines.

**Commit:** `docs(coverage): A1-guard names both exemption kinds (#292)` (source
doc-comment/string change; keep it a separate, reviewable commit from the
ADR/CONTRIBUTING prose).

## Task 3 — CONTRIBUTING coverage section

**Files.** `CONTRIBUTING.md` "Coverage and dependency policy" (§ ~399–436).

**Changes.**

- Line ~412: "unless one of **two** things exempts it" → **three**.
- After the `#[component]` bullet (before `// cov:ignore`), add a bullet:
  **"Structural exemption — `unreachable!("msg")`."** A message-carrying
  `unreachable!` invocation is dropped from the executable set with no marker:
  self-enforcing (reaching it panics ⇒ the test fails ⇒ `cargo llvm-cov` exits
  non-zero ⇒ no report), and message-required (bare `unreachable!()` stays
  measured, forcing an explicit reason). Fail-closed like `#[component]`:
  `std::unreachable!`, aliases, and macro-generated forms stay measured.
  Contrast with `cov:ignore`: the marker path is permanent and prose-only, while
  `unreachable!` re-flags itself if the line ever becomes live.
- Optionally update the "A few PostgreSQL storage error branches …
  `cov:ignore`'d" example note (line ~513) to mention that provably-dead lines
  may instead use `unreachable!("msg")` — keep light, defer the actual migration
  to #245.

**Run:** `prettier -w CONTRIBUTING.md` before staging.

**Commit:**
`docs(contributing): document the unreachable! coverage exemption (#292)`.

## Task 4 — amend ADR-0050

**Files.** `docs/adr/0050-stateless-coverage-gate.md` — **direct edit** of the
promoted, accepted ADR (the draft-out-of-git flow is for _new_ ADRs; amending an
existing one is a direct edit).

**Changes.**

- **Decision** section: add a point (or extend point 1) recording the third
  structural exemption — literal `unreachable!(<non-empty message>)`,
  message-required, fail-closed, self-enforcing; scope limited to `unreachable!`
  (not `panic!`/`todo!`/`unimplemented!`, not `std::unreachable!`).
- **Consequences** section: add the **honest trade-off** — moving lines from a
  text `cov:ignore` (immune to `syn` parse errors) to a `syn` structural
  exemption **concentrates fail-closed risk**: a single parse error in a file
  drops _all_ of that file's `unreachable!` exemptions at once. This is loud and
  safe (those lines revert to _measured_, so the gate can only FAIL, never
  silently pass) but is a robustness downgrade versus the pure-text path. Tie it
  to consequence #3's `cov:ignore`-permanence point: `unreachable!` is the
  self-re-flagging alternative for provably-dead lines.
- Do **not** change the ADR's `Status`/`Date`/number; this is an in-place
  amendment.

**Run:** `prettier -w docs/adr/0050-stateless-coverage-gate.md` before staging.

**Commit:**
`docs(adr): ADR-0050 records the unreachable! coverage exemption + trade-off (#292)`.

## Task 5 — full-gate verification & self-coverage

**Steps.**

- `devtool run -- cargo xtask check` — full host static + clippy + Nix
  coverage/tests. Must exit green (`ok:true`, `xtask-done: … ok=true`). Read
  `.xtask/last-result.json` `steps[]` if any step is unclear.
- Confirm **self-coverage**: the new `visit_macro` arm is exercised by task 1's
  tests; no new uncovered line is reported in `exempt.rs`, `gate.rs`, or
  `mod.rs`. If the gate flags a new uncovered line introduced by tasks 1–2, add
  a covering test (do **not** `cov:ignore` new gate code).
- Sanity smoke (optional, cheap): a throwaway fixture with `unreachable!("x")`,
  `unreachable!()`, and `std::unreachable!("x")` confirms only the first is
  exempt — but the in-file tests already assert this; skip if `check` is green.

**No separate commit** — this task is verification. If it surfaces a fix, fold
it into the relevant task's commit or add a follow-up commit referencing #292.

## Self-review

- Signature stability: `exempt_lines`, `evaluate`, `Verdict`, `Fail`,
  `failure_report` all unchanged — confirmed against spec "no signature change".
- Fail-closed preserved: only `is_ident("unreachable")` + non-empty tokens
  exempts; everything else stays measured; parse errors still `Err` → empty set.
- Every task ends green under `cargo xtask check`; commits are small and
  reviewable, one concern each; no `Co-Authored-By` trailer.
- Docs match code: CONTRIBUTING "three exemptions" and ADR-0050 amendment
  describe exactly what `exempt.rs` now does, including the
  concentrated-fail-closed-risk trade-off.
