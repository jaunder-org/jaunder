# Plan — Coverage gate: `cov:ignore` + `#[component]` exemption + CRAP threshold

**Spec:**
[`2026-07-04-issue-231-cov-ignore-coverage-gate.md`](../specs/2026-07-04-issue-231-cov-ignore-coverage-gate.md)
· Closes **#231** (supersedes #100, folds #37) · Milestone _Verify-gate
hardening_

---

## Review header (approve this layer)

**Goal.** Replace the stateful baseline/anchor coverage ratchet with a
**stateless** gate: an uncovered line fails unless it's inside a
`#[component]`/`view!` body (structural exempt) or carries `cov:ignore`. CRAP →
threshold. Delete the subsystem → #100 dissolves.

**Scope.**

- **In:** syn-based `#[component]` recognition (fail-closed) +
  covered-in-component tripwire; tightened + block-form `cov:ignore`; new
  stateless gate; CRAP threshold; migration of the ~700 remainder; deletion of
  baseline/classifier/merge-driver/CLI/hook; #37 source-filter; docs + ADRs.
- **Out:** raising coverage on the remainder (that's the _Code quality
  improvement_ grind-down, #232+); moving coverage to nightly;
  e2e/report-generation changes.

**Tasks (one line each):**

1. `syn` recognition of `#[component]`/`view!` spans (fail-closed), new
   `exempt.rs`.
2. Tighten `cov:ignore` line matcher + add block form (`report.rs`), hard-error
   unmatched.
3. New stateless gate + **A1-guard tripwire**, built _alongside_ the old
   classifier.
4. **Critical checkpoint:** real `cargo llvm-cov` — prove the guard starts green
   (exemption is a wash).
5. Migrate: structural exempt (auto) + `cov:ignore` the ~700 remainder → new
   gate green.
6. CRAP threshold `T=30` + `crap:allow` override; apply override to
   `test-support::main` (#232).
7. Swap the gate: make the stateless gate authoritative.
8. Delete the subsystem: baseline/classify/diffmap/reanchor + merge-driver +
   CLI + hook.
9. #37 fold-in: bound Nix coverage `src`, re-admit `csr/index.html`, drvPath
   probe.
10. Docs: rewrite CONTRIBUTING coverage section (honest tradeoffs); draft ADR
    (supersede 0030, amend 0029).

**Key risks / decisions:**

- **Safety ordering (load-bearing).** Build + prove the new gate green
  **before** deleting the old baseline safety net (Tasks 1–7 precede Task 8).
  Task 4 is the go/no-go: if any _covered_ line sits inside a `#[component]`
  span, the exemption is _not_ a wash and the approach must be revisited before
  anything is deleted.
- **Fail-closed recognition.** An unrecognized `#[component]` form must leave
  its body measured → gate FAIL (safe), never silent exempt (unsafe). Pinned by
  tests.
- **`cov:ignore` false-suppression.** Bare `contains` matches markers in
  strings/comments; anchored-comment match required before it becomes the
  primary mechanism.
- **Accepted weakenings** (per spec "Protection tradeoffs"): blanket component
  exemption, sub-T CRAP drift, permanent `cov:ignore`. Documented, not sold as
  stricter.

**For agentic workers:** execute with `jaunder-iterate`, delegating tasks via
`jaunder-dispatch` where useful; tick checkboxes in real time. **Do not reorder
Task 8 before Task 7.**

---

## Global constraints

- **Language/tests:** Rust; logic in `xtask/src/coverage/`, in-file
  `#[cfg(test)]` (crate convention; not ADR-0019 dialect files). Add
  `syn`/`proc-macro2` with the exact features pinned in Task 1 (incl.
  `span-locations`) to `xtask/Cargo.toml` — xtask is host-only and excluded from
  every derivation (`flake.nix:1211`), so it touches no Nix source. No
  `Co-Authored-By` trailer.
- **Gate:** `devtool run -- cargo xtask check` before each commit
  (`jaunder-commit`). During Tasks 1–6 the **old** baseline gate still runs and
  stays **green** — but it _heals_ (shrinks `coverage-baseline.json`) as lines
  are `cov:ignore`'d, so expect per-commit baseline churn + fail-and-restage
  during migration (see Task 5). The new gate is shadow-only until Task 7.
- **No placeholders:** every task compiles, is tested, and lands complete.
- **Report source:** all new logic consumes the existing
  `cargo llvm-cov report --text` output via `report::parse_text_report`
  (`tools/devtool/src/coverage/emit.rs:100`); no new report plumbing.

---

## Task 1 — `syn`-based `#[component]` recognition (fail-closed)

**Files:** new `xtask/src/coverage/exempt.rs`; `xtask/Cargo.toml`;
`xtask/src/coverage/mod.rs` (`mod exempt;`). Tests: in-file.

> **Load-bearing dependency detail.** `proc_macro2::Span::start()/end()` return
> line **0** unless proc-macro2 is built with the **`span-locations`** feature —
> without it this whole task silently maps everything to line 0. Pin it
> explicitly:
>
> ```toml
> syn = { version = "2", features = ["full", "visit", "extra-traits"] }
> proc-macro2 = { version = "1", features = ["span-locations"] }
> ```
>
> `LineColumn.line` is 1-based (matches our line numbering); columns are unused.
> A first test must assert a known fixture's component body maps to the
> _correct_ line range, not line 0 — this is the canary that the feature is
> actually enabled.

**Behavior.** For a source file, return the set of 1-based line numbers that are
**structurally exempt**: the body span of any `fn` carrying a `#[component]`
attribute. Parse with `syn` (robust to args/generics/nesting/strings) —
**never** a brace-scanner. **`#[component]`-only** — do NOT add a standalone
`view!` rule: a `view!` inside a component is already covered by the fn-body
span, and exempting `view!` _outside_ components (it occurs in `web/src/lib.rs`,
`web/src/feed_discovery.rs`) would both deviate from the spec's
`#[component]`-key decision and risk tripping the Task-4 A1-guard RED if such a
`view!` is natively covered. Note `f.block.span()` covers only `{`..`}`; a
multi-line component _signature_ stays measured (fail-closed/safe → those lines
join the Task 5 remainder).

```rust
use std::collections::BTreeSet;
use syn::spanned::Spanned;

/// 1-based line numbers structurally exempt from coverage in `src`.
/// Returns Err if the file cannot be parsed — the caller treats a parse
/// failure as "nothing exempt" (fail-closed: lines stay measured → gate can FAIL,
/// never silently exempt).
pub fn exempt_lines(src: &str) -> syn::Result<BTreeSet<u32>> {
    let file = syn::parse_file(src)?;
    let mut out = BTreeSet::new();
    let mut v = ExemptVisitor { out: &mut out };
    syn::visit::visit_file(&mut v, &file);
    Ok(out)
}

struct ExemptVisitor<'a> { out: &'a mut BTreeSet<u32> }

impl<'a, 'ast> syn::visit::Visit<'ast> for ExemptVisitor<'a> {
    fn visit_item_fn(&mut self, f: &'ast syn::ItemFn) {
        if has_component_attr(&f.attrs) {
            add_span(self.out, f.block.span()); // whole body exempt
        }
        syn::visit::visit_item_fn(self, f);
    }
    // NB: no visit_macro / standalone `view!` rule — see Behavior. view! inside a
    // component is already inside f.block.span(); view! elsewhere stays measured.
}

fn has_component_attr(attrs: &[syn::Attribute]) -> bool {
    // matches `#[component]` AND `#[component(...)]`; path-anchored, not substring.
    attrs.iter().any(|a| a.path().is_ident("component"))
}

fn add_span(out: &mut BTreeSet<u32>, s: proc_macro2::Span) {
    for l in s.start().line..=s.end().line { out.insert(l as u32); }
}
```

**Fail-closed contract:** `exempt_lines` returning `Err` (or the caller catching
a panic) maps to "no exemptions for this file" — the gate then sees the
uncovered lines and can fail. A missed/unknown component form is safe (FAIL),
never a false PASS.

**Tests (in-file):** real fixtures — `exempts_plain_component_body`,
`exempts_component_with_args` (`#[component(transparent)]`),
`exempts_view_inside_component` (covered via the body span),
`does_not_exempt_view_in_plain_fn` (standalone `view!` stays measured),
`does_not_exempt_server_fn` (a `#[server]` fn body stays measured),
`does_not_exempt_plain_fn`, `component_body_maps_to_correct_line_range_not_zero`
(the span-locations canary), `parse_error_yields_empty` (garbage → `Err` →
caller measures). `cargo nextest run -p xtask exempt::` → **PASS**.

**Commit:**
`feat(coverage): syn-based #[component]/view! exemption recognition (#231)`.

---

## Task 2 — Tighten `cov:ignore` + block form

**Files:** `xtask/src/coverage/report.rs`. Tests: in-file.

**B1 — anchored matcher.** Replace the bare `text.contains("// cov:ignore")`
(`report.rs:37-40`) with a match on the line's **actual trailing `//` comment**
(strip string/char literals first, then check the comment tail), so a marker
inside a string/doc comment no longer suppresses.

**B2 — block form.** Track `// cov:ignore-start` / `// cov:ignore-stop` in the
line loop; lines strictly between a matched pair are dropped (non-executable).
Rules: nesting not allowed; an unmatched `-start` at EOF or a `-stop` with no
open `-start` is a **hard error** (`parse_text_report` returns `Result`, gate
fails loudly). Ensure the start/stop marker lines are themselves consumed by the
block path, not double-handled by the line path (note
`"// cov:ignore-start".contains("// cov:ignore")` is true).

**Caller ripple (must update or it won't compile).** Making `parse_text_report`
return `Result` (for the hard-error markers) changes its two callers —
`mod.rs:284` and `mod.rs:161` — which must propagate the `Result`. Existing real
`cov:ignore` markers (9 in `storage/src/test_support.rs`, all genuine trailing
comments) must still be honored by the tightened matcher; add a regression test
asserting so.

**Tests:** `line_marker_ignored_only_as_real_comment`,
`marker_in_string_literal_does_not_suppress` (the current fixture at
`report.rs:59-66`, inverted to assert it's NOT dropped),
`block_drops_interior_lines`, `unmatched_block_start_is_error`,
`stray_block_stop_is_error`, `nested_block_is_error`.
`cargo nextest run -p xtask report::` → **PASS**.

**Commit:** `fix(coverage): anchored cov:ignore matcher + block form (#231)`.

---

## Task 3 — New stateless gate + A1-guard (alongside the old classifier)

**Files:** new `xtask/src/coverage/gate.rs`; `xtask/src/coverage/mod.rs`
(`mod gate;`, wire as a **shadow** computation — computed and reportable but NOT
yet the authoritative gate; the baseline classifier still gates until Task 7).
Tests: in-file.

**Behavior.** Given the parsed report (post-`cov:ignore`/block from Task 2) and
per-file `exempt_lines` (Task 1):

```rust
pub struct Verdict { pub failures: Vec<Fail>, pub guard_violations: Vec<Fail> }
pub struct Fail { pub file: String, pub line: u32, pub text: String }

/// Stateless verdict: an executable line fails iff uncovered AND not exempt.
/// Guard (tripwire): a COVERED line inside an exempt span means our "components
/// are never rendered natively" invariant is violated — fail loudly.
pub fn evaluate(files: &[FileCoverage], exempt_of: impl Fn(&str) -> BTreeSet<u32>) -> Verdict {
    let mut v = Verdict { failures: vec![], guard_violations: vec![] };
    for f in files {
        let ex = exempt_of(&f.path);
        for l in &f.lines {
            let exempt = ex.contains(&l.line);
            if !l.covered && !exempt {
                v.failures.push(Fail { file: f.path.clone(), line: l.line, text: l.text.clone() });
            } else if l.covered && exempt {
                v.guard_violations.push(Fail { file: f.path.clone(), line: l.line, text: l.text.clone() });
            }
        }
    }
    v
}
```

Report prints `file:line:text` for both buckets (reuse the `#87` recovery-hint
format).

**Tests:** `uncovered_unexempt_fails`, `uncovered_in_component_passes`,
`covered_in_component_trips_guard`, `covered_unexempt_passes`,
`uncovered_with_cov_ignore_passes` (end-to-end via `parse_text_report`).
`cargo nextest run -p xtask gate::` → **PASS**.

**Commit:**
`feat(coverage): stateless gate + covered-in-component tripwire, shadow mode (#231)`.

---

## Task 4 — CRITICAL CHECKPOINT: prove the guard green on a real report

**Files:** none (verification task). **No commit unless it forces a fix.**

Run a real instrumented build and evaluate the **shadow** gate's guard:

```
devtool run -- cargo xtask check     # produces the llvm-cov text report
# then evaluate shadow verdict over the emitted report + exempt_lines
```

**Expected:** `guard_violations` is **empty** — i.e. no _covered_ line falls
inside any `#[component]`/`view!` span, confirming component bodies are never
exercised natively and the exemption discards no coverage (the load-bearing
premise from spec §A1-guard / the SSR analysis).

**If RED (any guard violation):** STOP — the exemption is not a wash. Do not
proceed to deletion. Surface the violating `file:line`s; the design (blanket
component exemption) needs revisiting (e.g. exempt only 0%-covered components,
or measure the rendered ones). This is the go/no-go for the whole
re-architecture.

Also record the shadow `failures` count here — it should be ≈ the ~700 remainder
Task 5 must clear.

---

## Task 5 — Migrate: structural exempt (auto) + `cov:ignore` the remainder

**Files:** source across `web/src/pages/*` (page-helpers) + the ~32 scattered
files listed in #231's evidence. Also a throwaway helper is fine but not
required.

`#[component]` bodies are exempt automatically (Task 1) — no edits. The
**remainder** = shadow `failures` from Task 4 (uncovered ∧ not-exempt ∧
not-ignored), ≈700 lines. Apply `cov:ignore` **faithfully** (preserving today's
acceptance — not new debt; the baseline already accepts these):

- Contiguous runs → wrap in `// cov:ignore-start` / `// cov:ignore-stop` blocks
  (Task 2).
- Isolated lines that can take a trailing comment → line-form `// cov:ignore`.
- Note: syn recognition (Task 1) is more accurate than the earlier approximate
  scan, so the real remainder may be **smaller** than 700 (some "page-helper"
  lines are inside `view!` and now auto-exempt). Drive by the actual Task 4
  `failures` list.

**Verify:** re-run `cargo xtask check`; the **shadow** gate's `failures` and
`guard_violations` are empty.

**Expect baseline churn (corrected).** The OLD baseline gate stays _green_ (it
does not fail), but it does **not** stay inert: `cov:ignore`'ing a baselined
line removes it from `current` (`report.rs`), so Fix-mode `check` **heals
`coverage-baseline.json` smaller** (`heal_baseline`, `mod.rs:103-127`) and
rewrites it — dirtying the tree, which fires the pre-commit **fail-and-restage**
(`.githooks/pre-commit:26-33`) on essentially every migration commit. So each
migration commit legitimately carries a shrinking `coverage-baseline.json` diff
(and block-form insertions shift lines, absorbed by the re-anchor net). This is
expected and benign — stage the baseline delta with each commit; it all
disappears at Task 8. (`validate`, Mode::Check, does not heal and is also
green.)

**Commit(s):** per-area, e.g.
`chore(coverage): cov:ignore accepted-uncovered remainder in web/pages (#231)`,
`… in server/storage (#231)`.

---

## Task 6 — CRAP threshold + `crap:allow` override (shadow-add)

**Files:** `xtask/src/coverage/crap.rs` (**add**, don't remove); apply override
marker to `test-support/src/main.rs`. Tests: in-file.

**Shadow-add, mirroring Task 3 — do NOT touch the old path here.** `mod.rs:292`
still calls `crap::compare`, and Fix-mode (`mod.rs:304-319`) still calls
`crap::normalize_without_line` + `crap::pretty_manifest` and rewrites
`crap-manifest.json`. Removing any of those now **won't compile** (still called)
or **resurrects the manifest** on the next `check`. So here we only _add_ the
new threshold check and wire it into the **shadow** verdict; removal of
`compare`/`normalize_without_line`/`pretty_manifest` and deletion of
`crap-manifest.json` happen at Task 7 (swap) / Task 8 (delete).

Add a threshold check over the existing parsed
`Entry { crap, function, file, line }` (`crap.rs:36-48`; no `cargo crap` change
— it already emits JSON, `emit.rs:114`):

```rust
const CRAP_THRESHOLD: f64 = 30.0;
/// Fail any function whose CRAP exceeds T, unless a `crap:allow` override marks it.
pub fn evaluate_crap(entries: &[Entry], allow: &AllowSet) -> Vec<CrapFail> { … }
```

**`crap:allow` mapping — pin it.** The override is a reviewable in-source marker
(`// crap:allow: <reason>`), **not** hidden config. Before implementing, verify
`Entry.line`'s semantics against a **real** `crap-report.json` (fn line vs. LCOV
FN-record vs. opening brace — and `#[component]`/attrs sit a line above). Match
the marker **anywhere within the function's line span** (attribute lines through
body), not an exact signature-line hit, so it is robust to `Entry.line` pointing
at any of those. Apply it to `test-support/src/main.rs::main` with a rationale
referencing **#232** (real fix tracked there, milestone _Code quality
improvement_).

**Tests:** `crap_over_threshold_fails`, `crap_at_threshold_passes`,
`crap_allow_overrides_single_fn`, `crap_allow_requires_reason`,
`crap_allow_matched_within_span`. `cargo nextest run -p xtask crap::` →
**PASS**. (The old `compare` still runs and gates until Task 7; this task's
`evaluate_crap` is shadow-only.)

**Commit:**
`feat(coverage): CRAP threshold check (T=30) + crap:allow, shadow mode (#231, #232)`.

---

## Task 7 — Swap the gate authoritative

**Files:** `xtask/src/coverage/mod.rs` (the check/validate entrypoint);
`xtask/src/steps/nix.rs` (:40) + `xtask/src/result.rs` (:60) for the envelope.

Make the new checks authoritative:

- `gate::evaluate` (Task 3): fail on any `failures` or `guard_violations`; drop
  the call into `classify_against_anchor`/baseline.
- `crap::evaluate_crap` (Task 6): replace the `crap::compare` call at
  `mod.rs:292`; stop the Fix-mode manifest regen (`mod.rs:304-319`).
- **Reshape the report envelope:** `CoverageReport`/`CoverageVerdict`
  (`mod.rs:51-86`) and the `result.coverage` field (`result.rs:60`, populated at
  `nix.rs:40`) currently carry regression/baseline shapes — retype them to the
  new gate's `failures` / `guard_violations` / crap-threshold results so
  `status.json` and the report stay coherent.

The gate is now green (Tasks 5–6). Fix-mode no longer heals a coverage baseline
or CRAP manifest (there are none) — it still formats (fmt/leptosfmt/prettier,
`static_checks.rs`); `cov:ignore`/`crap:allow` are manual. **Keep `Mode::Fix`
itself** — it drives formatting; only the coverage/CRAP heal branches keyed on
it are removed.

**Verify:** `devtool run -- cargo xtask validate --no-e2e` green; deliberately
un-`cov:ignore` one remainder line → confirm it now FAILS via the new gate (not
the old classifier); make one `#[component]` line covered by a throwaway test →
confirm the A1-guard trips.

**Commit:** `refactor(coverage): make stateless gate authoritative (#231)`.

---

## Task 8 — Delete the subsystem

**Files (delete):** `coverage-baseline.json`, `crap-manifest.json`;
`xtask/src/coverage/{baseline,classify,diffmap,reanchor}.rs`; the
anchor/heal/re-anchor logic in `mod.rs` and the now-dead
`crap::compare`/`normalize_without_line`/`pretty_manifest`. **Merge-driver
subsystem:** `ensure_merge_driver_installed()` call (`xtask/src/main.rs:9`), the
merge-driver fns `xtask/src/lib.rs:396-462`, `.gitattributes:6-7`. **CLI:**
`coverage reanchor` / `coverage refresh-crap` subcommands `lib.rs:178-205` + run
arms `lib.rs:307-320` + parse tests `lib.rs:525-567`. **Tests:** the
merge-driver tests live in the module at `lib.rs:697-815` — delete **only** the
merge-driver cases and **preserve `git_at_scrubs_repo_redirecting_env`
(`lib.rs:712-737`)**, which tests `git::at` env-scrubbing and is unrelated.
**Hook:** in `.githooks/pre-commit`, **retain the `pre != post` fail-and-restage
mechanism** (`:26-33`) — `check` still reformats, so that capture is still
needed — and remove only the baseline/CRAP-heal _rationale/comments_.

**Verify:**
`rg -n 'coverage-baseline|baseline_anchor|reanchor|refresh-crap|coverage-keepours|crap-manifest|heal_baseline'`
returns only intended references (docs/ADR history). `git grep` for dead `mod`
decls and dangling `crap::` calls. `devtool run -- cargo xtask validate` green
end-to-end; make a trivial reformattable edit → confirm the hook still
fail-and-restages (formatting intact).

**Commit:**
`refactor(coverage): delete baseline/anchor subsystem + merge-driver + CLI (#231)`.

---

## Task 9 — #37 fold-in: bound the Nix coverage source

**Files:** `flake.nix` (coverage `src`, ~1207-1216); a drvPath probe step.

Change the coverage `src` from bare
`cleanSourceWith { src = ./.; <exclusions only> }` to `craneLib.path ./.` + the
cargo-source admission clause (mirroring `commonArgs.src` :272-289) **plus an
explicit admission for `csr/index.html`** (a compile-time `include_str!` at
`web/src/render/mod.rs:738` that `filterCargoSources` would otherwise drop,
breaking the build). Add a `drvPath` probe (ephemeral `git worktree` +
`nix eval --raw .#checks.<sys>.coverage.drvPath`, eval-only): untracked junk →
drvPath invariant; untracked `.rs` → drvPath changes.

**Verify:** `nix build .#checks.<sys>.coverage` compiles (proves
`csr/index.html` admitted); probe passes both directions.

**Commit:**
`fix(coverage): bound Nix coverage source to cargo sources, re-admit csr/index.html (#37, #231)`.

---

## Task 10 — Docs + ADR

**Files:** `CONTRIBUTING.md` (coverage section ~354-446); new draft ADR
(`docs/adr/drafts/`, `jaunder-adr` flow); update ADR-0029, mark ADR-0030
superseded at promote.

Rewrite CONTRIBUTING to the stateless model: `#[component]`/`view!` structurally
exempt, `cov:ignore` (line + block) as the only manual acceptance, CRAP
threshold + `crap:allow`, **no** baseline/anchor/regen ritual. State the
**honest protection tradeoffs** (spec section of the same name) — do not call it
"stricter." Draft ADR records the stateless-gate decision; cross-ref the
forward-compat path to native `#[coverage(off)]` (spec A3). Reconcile ADR-0029
(merge-driver / single-pass heal removed); ADR-0030 (text-identity re-anchor) →
superseded.

**Commit:** `docs(coverage): stateless-gate contract + draft ADR (#231)`.

---

## Self-review

- **Safety order enforced:** new gate built + proven green (Tasks 1–6), guard
  validated on a real report (Task 4 go/no-go), gate swapped (7), **then**
  deletion (8). The old baseline net is never removed before the new gate is
  authoritative and green.
- **Fail-closed** recognition (Task 1) and **hard-error** unmatched markers
  (Task 2) keep every ambiguity biased to FAIL, never silent PASS.
- **Cold-review fixes** all sequenced: A1 syn/fail-closed (T1), A1-guard
  (T3+T4), matcher/block (T2), deletion completeness incl. merge-driver/CLI/hook
  (T8), honest tradeoffs (T10).
- Acceptance mapped: #231 A/B/C/D/E across T1–T3,T5–T10; #37 at T9; #232
  override at T6.
- No spec restatement — tasks reference spec sections A1/A1-guard/B/C/D by name.
