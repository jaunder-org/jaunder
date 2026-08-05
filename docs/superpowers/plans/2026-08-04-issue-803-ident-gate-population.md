# Plan — #803: collapse `ident_gate::Population` into a plain ident set

Spec:
[`docs/superpowers/specs/2026-08-04-issue-803-ident-gate-population.md`](../specs/2026-08-04-issue-803-ident-gate-population.md)
Issue: [#803](https://github.com/jaunder-org/jaunder/issues/803) Branch:
`worktree-issue-803-ident-gate-population` Fork-point tag: `wt-base-issue-803`

## Review header

**Goal.** Delete the `Population` trait and its sole implementor `AnyOf`,
replacing both with a plain `&'static [&'static str]` field on `Gate`, so the
three XSS gates stop paying a type parameter for zero variation.
Behavior-preserving: no gate's verdict on any file changes.

**Scope — in:** `xtask/src/steps/ident_gate.rs` and the three gate modules
(`html_sink_check.rs`, `raw_html_door_check.rs`,
`rendered_html_from_trusted_check.rs`).

**Scope — out:** the marker mechanism (`classify`, `markers.rs`, orphan
detection), test-code handling, the ident sets, report prose, policed roots, and
`run_scan` (which never sees the population). No ADR is edited — see the spec's
"ADR-0094 is not edited".

**Separable concerns:** none surfaced during the design interview, so there is
no issue-filing task. The one adjacent observation — that #803 sits in the _Web:
canonical Leptos CSR convergence_ milestone despite being an xtask change — is a
metadata quirk, not work.

**Tasks:**

1. Capture the **baseline** census on the unchanged tree with a temporary
   harness.
2. Collapse the trait and degenericize the three types, carrying both doc
   paragraphs across — one change across all four files.
3. Re-run the harness, compare against the baseline, then delete it.
4. Run the full local gate and review the branch against the spec.

**Key risks / decisions:**

- _The real risk is silent population shrinkage, not a compile error._ A gate
  whose population shrank still passes green — ADR-0085 principle 6, named at
  `ident_gate.rs:63-65`. Tasks 1 and 3 bracket the change to close exactly that,
  and they are the tasks not to skip. Criterion 10 demands the comparison be
  **measured**, not asserted, which is why the baseline is captured _before_ the
  collapse rather than transcribed from a table.
- _The harness must live in the gate modules, not in `ident_gate.rs`._ Each
  gate's `POLICED_ROOTS`, `GATE`, `SINKS`/`DOORS` are **private** to its own
  module (`html_sink_check.rs:57,69,85`; `raw_html_door_check.rs:52,63,74`;
  `rendered_html_from_trusted_check.rs:71,82,92`) — siblings under `steps`, so a
  test in `ident_gate.rs` cannot see them.
- _The harness must fix its own cwd._ `POLICED_ROOTS` are workspace-relative,
  and `run_scan` only gets away with `Path::new(root)` because `cargo xtask`
  runs from the repo root. A unit test binary runs from `xtask/`, so the harness
  must use the repo's existing idiom,
  `Path::new(env!("CARGO_MANIFEST_DIR")).join("..")`
  (`server_fn_registrar_check.rs:677`, `server_fn_coverage_check.rs:536`). An
  empty sweep reporting "0 marked" is the dangerous outcome, so the harness
  asserts `marked > 0` before comparing anything.
- _`Scanner` keeps `'p`, loses only `P`._ `scan` takes a non-`'static`
  `&[&str]`, so `Scanner` must borrow (`population: &'p [&'p str]`).
  `&'a [&'b str]` coerces to `&'a [&'a str]` by covariance, so the
  single-lifetime form compiles; `walk_macro_tokens`'
  `let population = self.population;` (`:341`) still works, since a slice ref is
  `Copy`.
- _Task 2 is atomic by choice, not by necessity._ A compiling split does exist —
  add `impl Population for &'static [&'static str]`, migrate the three gates one
  at a time, then delete the trait and the bridge. It is rejected because the
  bridge impl is scaffolding that must itself be deleted, and the split does
  nothing about the risk that actually matters (population shrinkage, which
  tasks 1/3 cover). Not worth splitting — not impossible to split.
- _Two doc paragraphs are load-bearing and easy to lose_ (spec §"`AnyOf`'s doc
  survives the deletion of `AnyOf`"): the principle-3 argument on `AnyOf`, and
  the trait doc's "say what you don't look at". Both are rescued in task 2, not
  deferred.

**For agentic workers:** execute with **`jaunder-iterate`**, delegating a task
via **`jaunder-dispatch`** where useful. Tick checkboxes in this file in real
time.

## Global constraints

- Rust, crate `xtask`. No `Co-Authored-By` trailer on any commit.
- Before each commit run
  `devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-803-ident-gate-population -- cargo xtask check`
  (the pre-commit hook runs it anyway; running it first keeps the commit clean)
  — see **`jaunder-commit`**.
- Stage, then commit. Never `git commit -- <paths>`.
- No placeholders, no `todo!()`, no commented-out code left behind.
- `xtask` unit tests are in-file `#[cfg(test)]`; that convention does not change
  here. No storage/dual-backend template applies — this crate touches no
  backend.

---

## Task 1 — capture the baseline census (before any edit)

Criterion 10's "before" half, measured rather than assumed. **Nothing else in
the tree changes during this task.**

**Files**

- `xtask/src/steps/html_sink_check.rs` (temporary `#[cfg(test)]` harness)
- `xtask/src/steps/raw_html_door_check.rs` (same)
- `xtask/src/steps/rendered_html_from_trusted_check.rs` (same)

One harness per module, because that is where `POLICED_ROOTS` and `GATE` are in
scope.

**Interfaces**

```rust
// Appended to each gate module's existing `#[cfg(test)] mod tests`.
// TEMPORARY — deleted in task 3.
#[test]
fn census_snapshot() {
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let token = GATE.marker_token();
    let (mut marked, mut unexempt, mut orphans) = (0usize, 0usize, 0usize);
    let mut rows: Vec<String> = Vec::new();
    for root in POLICED_ROOTS {
        for path in crate::files::with_extension(&repo.join(root), "rs").unwrap() {
            let source = std::fs::read_to_string(&path).unwrap();
            let found = ident_gate::scan(&source, &GATE.population).unwrap();
            let c = ident_gate::classify(&source, &found, &token);
            marked += c.marked.len();
            unexempt += c.unexempt.len();
            orphans += c.orphans.len();
            for m in &c.marked {
                let rel = path.strip_prefix(&repo).unwrap_or(&path);
                rows.push(format!("{}:{}", rel.display(), m.line));
            }
        }
    }
    rows.sort();
    println!("== {} ==", GATE.step);
    for r in &rows {
        println!("{r}");
    }
    println!("marked={marked} unexempt={unexempt} orphans={orphans}");
    assert!(marked > 0, "empty sweep — the harness is not finding the roots");
}
```

(In task 1 the population argument is `&GATE.population`, i.e. `&AnyOf`; in task
3 it becomes `GATE.population`. That difference is expected — the **output** is
what gets compared, not the harness source.)

**Steps**

- [x] Add the harness to each of the three gate modules.
- [x] Run it and **save the full output** to `/tmp/census-before.txt`. This file
      is the baseline artifact for criterion 10 and for the PR body.
- [x] Sanity-check the totals across the three gates: expected **12 marked, 0
      unexempt, 0 orphans**. If they differ, stop and investigate before
      touching anything — the premise of the whole change is that the tree is
      currently clean. — **Confirmed: 5 (html-sink) + 1 (raw-html-door) + 6
      (rendered-html-from-trusted) = 12 marked, 0 unexempt, 0 orphans.**
- [x] Do **not** commit the harness.

**Note for the record:** `xtask` is **not** a workspace member (the flake
excludes it), so `cargo nextest run -p xtask` fails with "did not match any
packages". The working invocation is
`cargo nextest run --manifest-path xtask/Cargo.toml <filter>`. The plan's other
`-p xtask` Run blocks are corrected to match.

**Run**

```
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-803-ident-gate-population -- cargo nextest run --manifest-path xtask/Cargo.toml census_snapshot --no-capture
```

Expected: **PASS**, with `marked > 0` on every gate.

---

## Task 2 — collapse the trait

Delete the abstraction and carry its documentation onto what replaces it. The
tree does not compile part-way through, so this lands as one commit.

**Files**

- `xtask/src/steps/ident_gate.rs` (edit)
- `xtask/src/steps/html_sink_check.rs` (edit)
- `xtask/src/steps/raw_html_door_check.rs` (edit)
- `xtask/src/steps/rendered_html_from_trusted_check.rs` (edit)

**Interfaces — the exact shapes after this task**

```rust
// xtask/src/steps/ident_gate.rs

/// Every **non-test** mention of `population` in the source, in line order, plus
/// the line ranges of the test code that was skipped. `Err` on a `syn` parse
/// failure (fail-loud). Pure given the source, so gates unit-test through it.
pub fn scan(source: &str, population: &[&str]) -> Result<Scan, String> { … }

struct Scanner<'p> {
    population: &'p [&'p str],
    test_depth: usize,
    fn_stack: Vec<String>,
    hits: Vec<Mention>,
    test_ranges: Vec<(usize, usize)>,
}

pub struct Gate {
    pub step: &'static str,
    pub roots: &'static [&'static str],
    /// The idents this gate polices — its population, read structurally
    /// (ADR-0085 principle 1).
    ///
    /// Matching the ident rather than a call shape is what keeps such a gate an
    /// enumeration instead of a search for the spelling someone anticipated — a
    /// builder call, a struct field and a bare reference are all inside the
    /// population rather than silently outside it (ADR-0085 principle 3).
    pub population: &'static [&'static str],
    pub report: Report,
}
```

**Steps**

- [x] Delete `pub trait Population` **together with its doc comment** —
      `ident_gate.rs:149-171`, not just `157-171`; the doc runs from `:149` and
      leaving it dangling is a compile error.
- [x] Delete `pub struct AnyOf` and `impl Population for AnyOf` (`:180-195`).
- [x] Move `AnyOf`'s principle-3 paragraph (`:173-179`) onto the new
      `Gate::population` field doc, alongside the principle-1 citation. It must
      not die with the type.
- [x] Rewrite the trait doc's "say what you don't look at" argument as a
      property of the scan in the **module** doc: every gate reads idents
      everywhere — in ordinary code and inside macro tokens alike — **by
      construction**, so there is no hook a gate can silently fail to implement.
      Add the signpost: a future gate needing positional context re-introduces a
      seam in `walk_macro_tokens`, which already holds `trees` and `idx`.
- [x] Fix the two intra-doc links ``[`Population`]`` at `ident_gate.rs:10` and
      `:17` — they will otherwise point at a deleted item, and no rustdoc lint
      gate exists in this repo (`Cargo.toml:110-115` is clippy-only) to catch
      it.
- [x] Degenericize: `scan<P>` → `scan` (signature above); `Scanner<'p, P>` →
      `Scanner<'p>`; `Gate<P>` → `Gate`. Drop the bound from the three
      `impl<P: Population>` blocks (`:315`, `:355`, `:453`).
- [x] Update the two population call sites to pass the slice: `visit_ident`
      (`:409-413`) and `walk_macro_tokens` (`:340-352`). Both now ask the same
      question — membership in `self.population` — which is the point. **Keep
      `walk_macro_tokens`' hand-rolled token walk and its `Group` recursion
      exactly as they are;** only the membership test changes.
- [x] Update **both** `scan` call sites on `Gate`, dropping the now-stale `&`:
      `Gate::problems` (`:499`, production) and `Gate::violations` (`:461-476`,
      `#[cfg(test)]`) — `scan(source, &self.population)` →
      `scan(source,     self.population)`. `:499` is the one that fails soft:
      `&&'static [&'static str]` deref-coerces, so the tests still pass with the
      stale borrow in place and it surfaces only as `clippy::needless_borrow` at
      the commit gate.
- [x] Adapt the four test sites `ident_gate.rs:597`, `:605`, `:617`, `:622` —
      note these are **two imports and two constructions**, not four of a kind:
      `:597` (`use super::{scan, AnyOf};`) and `:617`
      (`use super::{classify, scan, AnyOf, Classified, Why};`) drop `AnyOf` from
      the import list, while `:605` and `:622` change `AnyOf(&["GUARDED"])` →
      `&["GUARDED"]`. **No test is renamed, deleted, or has its assertions
      weakened.**
- [x] In each of the three gate modules: drop `AnyOf` from the
      `use crate::steps::ident_gate::{…}` import, change
      `const GATE: Gate<AnyOf>` to `const GATE: Gate`, and
      `population: AnyOf(SINKS)` → `population: SINKS`
      (`html_sink_check.rs:50,85,88`), `AnyOf(DOORS)` → `DOORS`
      (`raw_html_door_check.rs:44,74,77`;
      `rendered_html_from_trusted_check.rs:67,92,95`).
- [x] Update the task-1 harness in each gate module to the new call shape
      (`&GATE.population` → `GATE.population`) so the tree keeps compiling. It
      is still temporary and still uncommitted.
- [x] Rewrite the test doc comment at
      `rendered_html_from_trusted_check.rs:216-221` ("Under `AnyOf` the door's
      own declaration is in the population — a deliberate behavior change…"). It
      records a real #778 behavior change, so preserve that meaning while naming
      the mechanism (ident matching everywhere) rather than the deleted type.
      Not a find-and-replace.
- [x] Leave the module's six numbered "Unreadable classes inherent to this scan"
      (`ident_gate.rs:33-61`) intact — none added, none removed; touch wording
      only where it referenced the trait.

**Run**

```
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-803-ident-gate-population -- cargo nextest run --manifest-path xtask/Cargo.toml
```

Expected: **PASS**, with every pre-existing test present. Two of them are
criterion 9's regression lock on the seam being removed and must be named in the
result: `mentions_come_back_in_line_order` (`ident_gate.rs:603`) and
`a_site_inside_a_macro_body_is_exempted_from_the_line_above` (`:767`). A test
that vanished is a failure of this task, not a simplification.

**Check (criteria 1–2)**

```
rg -n '\bPopulation\b' xtask/src/     # only //! or /// doc lines; no `trait `, `impl … for `, `<P`, `: Population`
rg -n '\bAnyOf\b' xtask/src/          # nothing (the harness is not committed, but must not reference it either)
```

Criteria 3–8 are established by the Steps above being executed as written;
criterion 9 by the two named tests passing.

**Commit** — exclude the temporary harness from the commit. Run
`cargo xtask check` first, then stage the four files' real changes and commit:
`refactor(xtask): collapse ident_gate::Population into a plain ident set (#803)`

---

## Task 3 — prove the census is unchanged, then remove the harness

Criterion 10's "after" half and the comparison.

**Steps**

- [x] Re-run the harness; save the output to `/tmp/census-after.txt`.
- [x] `diff /tmp/census-before.txt /tmp/census-after.txt` — must be **empty**.
      Same 12 marked sites at the same `path:line`, same
      `marked=12 unexempt=0 orphans=0` totals across the three gates.
- [x] If the diff is non-empty, **stop**. The collapse changed the population,
      which is the failure this bracket exists to catch. Do not adjust the
      harness or the expectations to match the new output.
- [x] Keep both files' contents for the PR body — this is the evidence for
      criterion 10.
- [x] **Delete the harness** from all three gate modules. It is not kept: the
      literal count is brittle (it changes whenever anyone adds a legitimate
      marker) and the gates themselves already assert 0 unexempt / 0 orphans on
      every run. Its job was the before/after comparison, which is now done.
- [x] Confirm `git status` shows a clean tree against the task-2 commit — the
      harness leaving no trace is the proof it was fully removed.

**Run**

```
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-803-ident-gate-population -- cargo nextest run --manifest-path xtask/Cargo.toml census_snapshot --no-capture
```

Expected: **PASS** before deletion; after deletion
`cargo nextest run --manifest-path xtask/Cargo.toml` still passes with no
`census_snapshot` test present.

---

## Task 4 — the full local gate

- [ ] `devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-803-ident-gate-population -- cargo xtask validate --no-e2e`

Expected: **green** (criterion 11). This runs the three gates against the real
tree, so it independently re-confirms 0 unexempt / 0 orphans. Long/cold run —
use the Bash tool's background mode.

- [ ] Review the whole branch against the spec:
      `git diff wt-base-issue-803..HEAD`. Walk the spec's eleven acceptance
      criteria and check each off explicitly.

## Self-review

- Every spec acceptance criterion maps to a task: 1–2 → task 2's **Check**
  block, 3–8 → task 2's Steps, 9 → task 2's two named regression tests, 10 →
  tasks 1+3 (measured on both halves, not transcribed), 11 → task 4.
- No task smuggles work the spec did not authorize. `run_scan`, `markers.rs`,
  the ident sets, the roots and the report prose are untouched, as the spec's
  Scope requires. The harness is temporary and provably removed before the
  branch lands.
- Task 2 is large but lands as one commit by choice; the rejected alternative (a
  bridging `impl Population for &'static [&'static str]`) is recorded in the
  header so the decision can be re-examined rather than re-discovered.
- The riskiest failure mode (silent population shrinkage) is bracketed by a
  measured before/after with an explicit "do not adjust the expectations"
  instruction.
