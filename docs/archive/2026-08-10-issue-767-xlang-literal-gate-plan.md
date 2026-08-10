# `xlang-literal` Gate Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with `jaunder-iterate`
> (delegating individual tasks to a subagent via `jaunder-dispatch` when
> useful). Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a host static check that fails `cargo xtask check` when a
cross-language duplicated literal drifts, instead of letting a 25-minute e2e
matrix discover it.

**Architecture:** One new xtask step module holding a `PAIRS` table of
`{key, a: Site, b: Site}`. A `Site` is `{file, anchor, quote}`; extraction finds
the anchor (exactly once), requires the quote immediately after it, and reads to
the closing quote. `problems(root)` loops the table and reports every read
error, extraction error, and disagreement. Registered in `Command::Check` and
`Command::Validate` beside `e2e_scaffold_check`.

**Tech Stack:** Rust (edition 2024), `xtask` workspace. **No new dependency** —
`tempfile` 3 is already present for the fixture-root test.

**Spec:**
[`2026-08-10-issue-767-xlang-literal-gate-spec.md`](2026-08-10-issue-767-xlang-literal-gate-spec.md)

## Review header

**Scope — in:** the `xlang-literal` step module, its registration in `check` and
`validate`, the two shipped pairs, the counterpart-comment updates on all four
sites, and the ADR draft.

**Scope — out:** discovering undeclared duplicated literals; collapsing either
duplication; any change to e2e behaviour or to the literals' values.

**Tasks:**

1. Extraction primitive — `Site` + `literal_in`, with the failure modes pinned.
2. The `PAIRS` table + `problems(root)`, with a fixture root.
3. Wire the step into `check` and `validate`; assert the table resolves against
   the real tree (AC9).
4. Update the four counterpart comments (AC11) and verify the ADR draft against
   what shipped (AC12 — the draft stays in the gitignored pen; `jaunder-ship`
   numbers it).

**Key risks / decisions:**

- **The anchor must be the declaration form, never the value.** Task 4 rewrites
  the very comments that sit beside the matched literals; an anchor matching
  `data-mounted` would make a prose edit flip the gate. Task 3's real-tree test
  is what catches this, and it runs _before_ Task 4 edits the comments — so if
  Task 4 breaks an anchor, Task 4's own gate run fails.
- **Zero matches must fail.** The single most important behaviour: a gate that
  matches nothing must never look like a gate that passed.
- `xtask` is its own workspace, so its tests run in the `xtask-tests` step, not
  under the Nix coverage gate (`flake.nix:1190` excludes `/xtask/`).
- **Every commit must pass `cargo xtask check`, and `mod steps` is private**
  (`xtask/src/lib.rs:27`), so a `pub` item inside it is `dead_code` until
  something non-test calls it. `xtask-clippy` runs
  `--all-targets -- -D warnings` (`static_checks.rs:117-128`), which builds the
  lib without `cfg(test)` — so items reached only from `#[cfg(test)] mod tests`
  are denied.

  **Resolved by merging Tasks 1–3 into one commit** (revised during execution).
  The plan first tried a transient module-level `#![expect(dead_code)]`, removed
  in Task 3. That does not work: `--all-targets` also builds the **lib test**
  target, where the items _are_ used, so the expectation is unfulfilled there
  and `-D unfulfilled-lint-expectations` denies it. No single attribute
  satisfies both targets, and `#![allow(dead_code)]` would need user approval to
  suppress a lint rather than fix it. Making the module reachable immediately
  needs no suppression at all, so Tasks 1–3 are one deliverable with one commit.
  Their TDD steps still run in order.

## Global Constraints

- Edition 2024, `xtask` workspace (`xtask/Cargo.toml`). **Add no dependency.**
- Every failure message names the issue `(#767)`, matching the
  `e2e_scaffold_check` house style.
- Test command for every task: `cargo test --manifest-path xtask/Cargo.toml`.
- Per-commit gate: `cargo xtask check` must pass before each commit
  (**`jaunder-commit`**). **No `Co-Authored-By` trailer.**

---

### Task 1: The extraction primitive — DONE

> Committed together with Tasks 2 and 3 (see Key risks: the `dead_code`
> constraint makes them one landable unit). The `#![expect(dead_code)]` scaffold
> described below was **not** used — it is unfulfilled in the lib test target
> and denied there. Red-green was replaced by a mutation check: flipping
> `occurrences != 1` to `== 0` failed exactly the two duplicate-anchor tests.

**Files:**

- Create: `xtask/src/steps/xlang_literal_check.rs`
- Modify: `xtask/src/lib.rs:56` (add `pub mod xlang_literal_check;` as the last
  entry of the `steps` block, after `pub mod wasm_budget;` at `:55` — the block
  is alphabetical)
- Test: in-file `#[cfg(test)] mod tests` (the convention every `*_check.rs` in
  this directory follows)

**Interfaces:**

- Consumes: nothing.
- Produces:
  - `pub struct Site { pub file: &'static str, pub anchor: &'static str, pub quote: char }`
  - `pub fn literal_in(site: &Site, source: &str) -> Result<String, String>` —
    `Ok` is the extracted literal; `Err` is a human-readable message that always
    names `site.file`.

- [ ] **Step 1: Write the failing tests**

Write the module with the doc comment, the transient lint scaffold, the `Site`
struct, a `literal_in` signature whose body is `todo!()`, and this test module.

The scaffold goes at the very top of the file, immediately after the module doc
comment:

```rust
// Transient: nothing outside `#[cfg(test)]` calls into this module until Task 3
// adds `run`, and `mod steps` is private, so every item here is `dead_code` under
// `xtask-clippy`'s `--all-targets -- -D warnings`. `expect`, not `allow`: when
// `run` lands the lint stops firing and this attribute becomes an error, which is
// what forces its removal (#767).
#![expect(dead_code)]
```

```rust
#[cfg(test)]
mod tests {
    use super::{Site, literal_in};

    fn ts_site() -> Site {
        Site {
            file: "end2end/tests/mount.ts",
            anchor: "MOUNTED_ATTR = ",
            quote: '"',
        }
    }

    fn rust_site() -> Site {
        Site {
            file: "csr/src/lib.rs",
            anchor: "setAttribute(",
            quote: '\'',
        }
    }

    #[test]
    fn extracts_a_double_quoted_literal() {
        let src = "export const MOUNTED_ATTR = \"data-mounted\";\n";
        assert_eq!(literal_in(&ts_site(), src).unwrap(), "data-mounted");
    }

    #[test]
    fn extracts_a_single_quoted_literal() {
        let src = "        document.body.setAttribute('data-mounted', 'true');\n";
        assert_eq!(literal_in(&rust_site(), src).unwrap(), "data-mounted");
    }

    /// The whole point of the gate: a locator that has stopped locating anything
    /// must be loud, never a pass.
    #[test]
    fn a_missing_anchor_is_an_error_naming_the_file_and_anchor() {
        let e = literal_in(&ts_site(), "export const OTHER = \"z\";\n").unwrap_err();
        assert!(e.contains("end2end/tests/mount.ts"), "{e}");
        assert!(e.contains("MOUNTED_ATTR = "), "{e}");
        assert!(e.contains("not found"), "{e}");
        assert!(e.contains("#767"), "{e}");
    }

    #[test]
    fn a_repeated_anchor_is_an_error_naming_the_count() {
        let src = "export const MOUNTED_ATTR = \"a\";\nexport const MOUNTED_ATTR = \"b\";\n";
        let e = literal_in(&ts_site(), src).unwrap_err();
        assert!(e.contains("2 times"), "{e}");
        assert!(e.contains("end2end/tests/mount.ts"), "{e}");
    }

    /// Counting occurrences rather than matching lines — two anchors on one line
    /// must not resolve to a silent first-wins.
    #[test]
    fn two_anchors_on_one_line_also_fail() {
        let src = "MOUNTED_ATTR = \"a\"; MOUNTED_ATTR = \"b\";\n";
        let e = literal_in(&ts_site(), src).unwrap_err();
        assert!(e.contains("2 times"), "{e}");
    }

    /// The anchor ends immediately before the opening quote, so anything else
    /// there means the declaration's shape changed and the table is stale.
    #[test]
    fn an_anchor_not_immediately_followed_by_the_quote_is_an_error() {
        let e = literal_in(&ts_site(), "export const MOUNTED_ATTR = someIdent;\n").unwrap_err();
        assert!(e.contains("not immediately followed"), "{e}");
        assert!(e.contains("end2end/tests/mount.ts"), "{e}");
    }

    #[test]
    fn an_unterminated_literal_is_an_error() {
        let e = literal_in(&ts_site(), "export const MOUNTED_ATTR = \"data-mounted;\n").unwrap_err();
        assert!(e.contains("unterminated"), "{e}");
    }

    #[test]
    fn an_escaped_quote_does_not_end_the_literal() {
        let src = "export const MOUNTED_ATTR = \"a\\\"b\";\n";
        assert_eq!(literal_in(&ts_site(), src).unwrap(), "a\\\"b");
    }

    #[test]
    fn an_empty_literal_extracts_as_empty_rather_than_erroring() {
        let src = "export const MOUNTED_ATTR = \"\";\n";
        assert_eq!(literal_in(&ts_site(), src).unwrap(), "");
    }
}
```

- [ ] **Step 2: Run the tests, verify they fail**

Run: `cargo test --manifest-path xtask/Cargo.toml xlang_literal` Expected: FAIL
— every test panics at `todo!()` in `literal_in`.

- [ ] **Step 3: Implement `literal_in`**

Write the body to signature
`pub fn literal_in(site: &Site, source: &str) -> Result<String, String>`. Every
branch is pinned by a Step 1 test — count `!= 1` (zero and repeated, including
same-line), quote not immediately after the anchor, unterminated literal,
backslash escape, empty literal, and the two success paths — so the tests
determine the body.

Two details the tests constrain but do not spell out:

- Count with `source.matches(site.anchor).count()`, and locate with
  `source.find(site.anchor)`. The count check runs **first**, so `find` is
  reached only when exactly one occurrence exists.
- The escape rule preserves the backslash in the output (`a\"b`, per
  `an_escaped_quote_does_not_end_the_literal`): the gate compares literals to
  each other, not to a decoded value, and both sides are compared the same way.

Also write the module doc comment now, including the AC10 honesty limit — that
the gate polices exactly the declared pairs and cannot discover an undeclared
duplicated literal — and a pointer to
`docs/adr/0109-cross-language-literal-agreement.md`.

- [ ] **Step 4: Run the tests, verify they pass**

Run: `cargo test --manifest-path xtask/Cargo.toml xlang_literal` Expected: PASS
— 9 tests.

- [ ] **Step 5: Commit**

Run `cargo xtask check` first (**`jaunder-commit`**), then:

```bash
git add xtask/src/steps/xlang_literal_check.rs xtask/src/lib.rs
git commit -m "feat(xtask): extract a declared literal from a source file (#767)"
```

---

### Task 2: The pair table and `problems` — DONE

> Committed with Tasks 1 and 3. All 7 tests written as planned and passing.

**Files:**

- Modify: `xtask/src/steps/xlang_literal_check.rs`
- Test: same in-file `#[cfg(test)] mod tests`

**Interfaces:**

- Consumes: `Site`, `literal_in` from Task 1.
- Produces:
  - `pub struct Pair { pub key: &'static str, pub a: Site, pub b: Site }`
  - `pub const PAIRS: &[Pair]` — the two shipped pairs.
  - `pub fn problems(root: &Path) -> Option<String>` — `None` when every pair
    agrees; `Some(detail)` listing every read error, extraction error, and
    disagreement, one per line.

- [ ] **Step 1: Write the failing tests**

Add `Pair`, `PAIRS` (values below), and a `problems` signature with a `todo!()`
body. Keep the `#![expect(dead_code)]` scaffold — Task 3 removes it.

**Widen the test module's import** rather than adding a second `use super::`
line; `-D warnings` denies an unused one, so import exactly what is referenced:

```rust
    use super::{PAIRS, Site, literal_in, problems};
```

`Pair` is not imported — no test names the type, only `PAIRS`. `std::path::Path`
is not imported either: `problems(dir.path())` needs no `Path` in scope, and
Task 3 adds it when its test first uses `Path::new`.

Then add these tests to the existing test module:

```rust
    /// Build a fixture root holding every file `PAIRS` names, with the literal of
    /// each site set from `values` (indexed the same way `PAIRS` is walked:
    /// pair 0 side a, pair 0 side b, pair 1 side a, pair 1 side b).
    fn fixture_root(values: [&str; 4]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut values = values.into_iter();
        for pair in PAIRS {
            for site in [&pair.a, &pair.b] {
                let value = values.next().expect("one value per site");
                let path = dir.path().join(site.file);
                std::fs::create_dir_all(path.parent().expect("site files are nested"))
                    .expect("mkdir");
                let q = site.quote;
                std::fs::write(&path, format!("prefix {}{q}{value}{q} suffix\n", site.anchor))
                    .expect("write");
            }
        }
        dir
    }

    #[test]
    fn agreeing_pairs_report_no_problem() {
        let dir = fixture_root(["data-mounted", "data-mounted", "jaunder.", "jaunder."]);
        assert_eq!(problems(dir.path()), None);
    }

    /// AC8 — the gate asserts agreement, not a value. A consistent rename passes.
    #[test]
    fn a_consistent_rename_on_both_sides_passes() {
        let dir = fixture_root(["data-ready", "data-ready", "jaunder.", "jaunder."]);
        assert_eq!(problems(dir.path()), None);
    }

    /// AC3 — the message must carry everything needed to fix it.
    #[test]
    fn mount_marker_drift_names_the_key_both_files_and_both_values() {
        let dir = fixture_root(["data-mounted", "data-mountd", "jaunder.", "jaunder."]);
        let detail = problems(dir.path()).expect("a problem");
        assert!(detail.contains("mount-marker"), "{detail}");
        assert!(detail.contains("csr/src/lib.rs"), "{detail}");
        assert!(detail.contains("end2end/tests/mount.ts"), "{detail}");
        assert!(detail.contains("data-mounted"), "{detail}");
        assert!(detail.contains("data-mountd"), "{detail}");
    }

    /// AC4 — the same, for the second pair, proving the table is a loop and not
    /// a special case wrapped around one comparison.
    #[test]
    fn mark_prefix_drift_names_the_key_both_files_and_both_values() {
        let dir = fixture_root(["data-mounted", "data-mounted", "jaunder.", "jaunder-"]);
        let detail = problems(dir.path()).expect("a problem");
        assert!(detail.contains("mark-prefix"), "{detail}");
        assert!(detail.contains("client/src/perf/mod.rs"), "{detail}");
        assert!(detail.contains("end2end/tests/capture-trace.ts"), "{detail}");
        assert!(detail.contains("jaunder."), "{detail}");
        assert!(detail.contains("jaunder-"), "{detail}");
    }

    /// AC7 — a missing site file is a hard failure, never a skip or a pass. This
    /// is the arm `e2e_scaffold_check` leaves untested; making `root` a parameter
    /// is what lets it be tested at all.
    #[test]
    fn a_missing_site_file_fails_and_names_the_path() {
        let dir = fixture_root(["data-mounted", "data-mounted", "jaunder.", "jaunder."]);
        std::fs::remove_file(dir.path().join("csr/src/lib.rs")).expect("remove");
        let detail = problems(dir.path()).expect("a problem");
        assert!(detail.contains("csr/src/lib.rs"), "{detail}");
        assert!(detail.contains("cannot read"), "{detail}");
    }

    /// An extraction failure on one side must not be reported as a disagreement —
    /// there is no second value to disagree with, and saying so would send the
    /// reader looking for a drift that is not there.
    #[test]
    fn an_unlocatable_site_reports_the_anchor_not_a_mismatch() {
        let dir = fixture_root(["data-mounted", "data-mounted", "jaunder.", "jaunder."]);
        std::fs::write(dir.path().join("end2end/tests/mount.ts"), "nothing here\n")
            .expect("write");
        let detail = problems(dir.path()).expect("a problem");
        assert!(detail.contains("not found"), "{detail}");
        assert!(!detail.contains("disagree"), "{detail}");
    }

    #[test]
    fn every_pair_key_is_unique() {
        let mut keys: Vec<&str> = PAIRS.iter().map(|p| p.key).collect();
        keys.sort_unstable();
        let before = keys.len();
        keys.dedup();
        assert_eq!(keys.len(), before, "duplicate key in PAIRS");
    }
```

The `PAIRS` value, written now so the tests compile:

```rust
/// The cross-language literal pairs this gate polices. Adding a pair is a row
/// here; see the module doc for what this table does **not** claim.
pub const PAIRS: &[Pair] = &[
    Pair {
        key: "mount-marker",
        a: Site {
            file: "csr/src/lib.rs",
            anchor: "setAttribute(",
            quote: '\'',
        },
        b: Site {
            file: "end2end/tests/mount.ts",
            anchor: "MOUNTED_ATTR = ",
            quote: '"',
        },
    },
    Pair {
        key: "mark-prefix",
        a: Site {
            file: "client/src/perf/mod.rs",
            anchor: "MARK_PREFIX: &str = ",
            quote: '"',
        },
        b: Site {
            file: "end2end/tests/capture-trace.ts",
            anchor: "MARK_PREFIX = ",
            quote: '"',
        },
    },
];
```

- [ ] **Step 2: Run the tests, verify they fail**

Run: `cargo test --manifest-path xtask/Cargo.toml xlang_literal` Expected: FAIL
— the seven new tests panic at `todo!()` in `problems`; Task 1's nine still
pass.

- [ ] **Step 3: Implement `problems`**

Write the body to signature `pub fn problems(root: &Path) -> Option<String>`.
Every branch is pinned: agreement, disagreement per pair, unreadable file,
extraction error, and the "extraction error is not reported as a disagreement"
rule (which is what forces the disagreement check to be conditional on having
collected **two** values, not on the two files having been visited).

Accumulate one message per problem into a `Vec<String>` and return
`(!lines.is_empty()).then(|| lines.join("\n"))` — the
`e2e_scaffold_check::problems` shape.

- [ ] **Step 4: Run the tests, verify they pass**

Run: `cargo test --manifest-path xtask/Cargo.toml xlang_literal` Expected: PASS
— 16 tests.

- [ ] **Step 5: Commit**

Run `cargo xtask check` first, then:

```bash
git add xtask/src/steps/xlang_literal_check.rs
git commit -m "feat(xtask): compare the declared cross-language literal pairs (#767)"
```

---

### Task 3: Wire the step in, and pin it to the real tree — DONE

> 17/17 tests pass. The real-tree test was proved to bite by temporarily
> breaking the `MOUNTED_ATTR = ` anchor (it failed, naming the anchor). AC3 was
> verified end to end by hand: changing `csr/src/lib.rs`'s literal to
> `data-mountd` made `cargo xtask check` fail with
> `mount-marker: cross-language literals disagree — csr/src/lib.rs says "data-mountd", end2end/tests/mount.ts says "data-mounted"`.
> Reverted; gate green.

**Files:**

- Modify: `xtask/src/steps/xlang_literal_check.rs` (add `run`, add the real-tree
  test)
- Modify: `xtask/src/lib.rs:474` and `xtask/src/lib.rs:519` (one call each,
  immediately after `steps::e2e_scaffold_check::run(&mut result);`)
- Test: same in-file test module

**Interfaces:**

- Consumes: `PAIRS`, `problems` from Task 2.
- Produces: `pub fn run(result: &mut CommandResult)` — pushes a `StepResult`
  named `"xlang-literal"`.

- [ ] **Step 1: Write the real-tree test**

Add `use std::path::Path;` to the test module — Task 3 is where it is first
needed — then add this test, the one that makes a future refactor of
`csr/src/lib.rs`, `mount.ts`, `perf/mod.rs` or `capture-trace.ts` fail here
rather than silently disarm the gate (AC9):

```rust
    /// AC9 — every table entry must resolve against the **real** tree. Every other
    /// test in this file feeds `problems` a fixture, which by construction cannot
    /// notice that an anchor no longer matches the source it was written for.
    ///
    /// The test binary's cwd is the `xtask` package, not the repo root — hence
    /// `CARGO_MANIFEST_DIR`, the `server_fn_registrar_check.rs:677` precedent.
    #[test]
    fn the_real_table_resolves_and_agrees() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        for pair in PAIRS {
            for site in [&pair.a, &pair.b] {
                let path = root.join(site.file);
                let src = std::fs::read_to_string(&path)
                    .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
                literal_in(site, &src).unwrap_or_else(|e| panic!("{e}"));
            }
        }
        assert_eq!(problems(&root), None);
    }
```

- [ ] **Step 2: Run the test — it is expected to PASS, and that is the point**

This step deliberately breaks the red-green rhythm: AC9 is a **regression pin**,
not new behaviour, so a passing first run is correct. Do not "fix" it.

Run: `cargo test --manifest-path xtask/Cargo.toml xlang_literal` Expected:
**PASS** for `the_real_table_resolves_and_agrees` — the anchors were verified
against the real tree while the spec was written, so this test is a regression
pin, not a red-to-green step. If it fails, an anchor is wrong: fix the anchor,
not the test.

Confirm it is genuinely exercising the tree by temporarily breaking one anchor
(e.g. `"MOUNTED_ATTR = "` → `"MOUNTED_ATTRX = "`), re-running to see it fail,
then reverting. A test that would pass against an empty tree is worth nothing.

- [ ] **Step 3: Implement `run`, register it, and delete the lint scaffold**

Add the module's non-test import — `e2e_scaffold_check.rs:20`'s form:

```rust
use crate::result::{CommandResult, StepResult};
```

Write the body to signature `pub fn run(result: &mut CommandResult)`, following
`e2e_scaffold_check::run` exactly: call `problems(Path::new("."))` — xtask
always runs from the repo root — and push `StepResult::ok("xlang-literal")` or
`StepResult::fail("xlang-literal").detail(detail)`. `run` needs `Path` outside
the test module, so move `use std::path::Path;` up to the module's imports.

Then add `steps::xlang_literal_check::run(&mut result);` to `xtask/src/lib.rs`
immediately after the `e2e_scaffold_check::run` call in **both**
`Command::Check` and `Command::Validate` (at HEAD, `:474` and `:519`; Task 1's
`mod` line shifts both by one, so match on the `e2e_scaffold_check` call rather
than the number).

**Delete the `#![expect(dead_code)]` scaffold.** The module is now reachable, so
leaving it is itself a `-D warnings` error — which is exactly why `expect` was
chosen over `allow`.

- [ ] **Step 4: Verify the step runs and passes**

Run:
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-767-mounted-attr-gate -- cargo xtask check`
Expected: PASS. Then confirm the step is present and green:

```bash
jq '.steps[] | select(.name == "xlang-literal")' .xtask/last-result.json
```

Expected: `{"name": "xlang-literal", "ok": true, "skipped": false}`.

- [ ] **Step 5: Verify the gate actually catches drift end to end (AC3 by
      hand)**

Edit `csr/src/lib.rs:18`'s literal from `data-mounted` to `data-mountd`, run
`cargo xtask check`, and confirm the `xlang-literal` step fails with a detail
naming `mount-marker`, both paths, and both values. **Revert the edit** and
re-run to confirm green.

- [ ] **Step 6: Commit**

```bash
git add xtask/src/steps/xlang_literal_check.rs xtask/src/lib.rs
git commit -m "feat(xtask): run the xlang-literal gate in check and validate (#767)"
```

---

### Task 4: Point the counterpart comments at the gate, and land the ADR — DONE

> All four comments updated; `capture-trace.ts`'s false `MOUNTED_ATTR` claim
> rewritten. The 17 tests still pass after the edits, so no comment introduced a
> second anchor occurrence. ADR draft verified against what shipped (heading
> `# ADR-DRAFT:`, status `proposed`, principle 5 addressed, `regex` present only
> as the rejected option). Full `cargo xtask check` green.

**Files:**

- Modify: `csr/src/lib.rs:11-14`
- Modify: `end2end/tests/mount.ts:4-9`
- Modify: `client/src/perf/mod.rs:22-24` (the doc comment on `MARK_PREFIX`)
- Modify: `end2end/tests/capture-trace.ts:185-188`
- Review only (already written, **not** committed — gitignored until ship):
  `docs/adr/0109-cross-language-literal-agreement.md`

**Every range above stops one line short of the declaration** — `perf/mod.rs:25`
and `capture-trace.ts:189` are the anchor lines themselves. Editing into them is
how this task would break its own gate.

**Interfaces:**

- Consumes: the `xlang-literal` step name from Task 3.
- Produces: nothing consumed by a later task.

- [ ] **Step 1: Update the four comments**

Each currently says the two literals "must agree" and names its counterpart.
Each must now also name **what enforces it**, so a reader editing one side
learns where they will be caught. Keep the counterpart reference — it is still
how a reader finds the other side.

`csr/src/lib.rs:11-14` — replace "The two literals must agree; if they drift,
every e2e test times out." with wording that keeps that consequence and adds
that the agreement is enforced by the `xlang-literal` gate
(`xtask/src/steps/xlang_literal_check.rs`, #767).

`end2end/tests/mount.ts:4-9` and `client/src/perf/mod.rs:22-24` — the same
addition.

`end2end/tests/capture-trace.ts:185-188` — this one carries a claim that becomes
**false**. It currently reads "the property that keeps the two sides from
drifting the way `MOUNTED_ATTR` can." `MOUNTED_ATTR` can no longer drift
undetected, and this file's own `MARK_PREFIX` is now policed by the same gate.
Rewrite the comment so it still explains why discovery-by-prefix is the better
property (adding a mark needs no change here) without asserting that
`MOUNTED_ATTR` is unprotected.

**Do not put any anchor string into a comment.** The anchors are declaration
forms (`MOUNTED_ATTR = `, `setAttribute(`, `MARK_PREFIX = `,
`MARK_PREFIX: &str = `); writing one in prose adds a second occurrence and fails
the gate. Refer to the constants by bare name.

- [ ] **Step 2: Verify the gate still resolves**

Run: `cargo test --manifest-path xtask/Cargo.toml xlang_literal` Expected: PASS
— in particular `the_real_table_resolves_and_agrees`. A failure here means a
comment edit introduced a second anchor occurrence; fix the comment.

- [ ] **Step 2b: Verify the ADR draft against AC12 and against what shipped**

The draft was written before the implementation, so read
`docs/adr/0109-cross-language-literal-agreement.md` and confirm it still
describes what actually landed. Specifically:

- It states the decision — cross-language literal agreement is enforced by one
  gate over a declared table of pairs, each site a file + anchor + quote.
- It states the zero/duplicate/unreadable hard-failure rule, and the
  locates-a-site-not-a-violation argument.
- It addresses ADR-0085 principle 5 and states 0085's type-safety scope
  honestly.
- It states the honesty limit — the table's completeness is not claimed.
- Its heading is exactly `# ADR-DRAFT: …` and its status is `proposed`, so
  `cargo xtask adr promote` can number it at ship.

Correct any claim the implementation diverged from. Do **not** number it or move
it out of `drafts/`.

- [ ] **Step 3: Format the touched TypeScript and markdown**

Run:
`devtool run -- prettier -w end2end/tests/mount.ts end2end/tests/capture-trace.ts docs/adr/0109-cross-language-literal-agreement.md`

- [ ] **Step 4: Run the full gate**

Run:
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-767-mounted-attr-gate -- cargo xtask check`
Expected: PASS.

Then AC13 — the step must also be green in the pre-push gate, and this is the
only place the plan proves `Command::Validate`'s registration is real rather
than just written:

Run (background; it is slow):
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-767-mounted-attr-gate -- cargo xtask validate --no-e2e`
Expected: PASS, with `xlang-literal` present and `ok: true` in
`.xtask/last-result.json`'s `steps[]`. Note the Nix coverage gate does **not**
cover the new module — `flake.nix:1190` excludes `/xtask/` — so its tests are
gated by the `xtask-tests` step instead.

Note the ADR draft is gitignored in `docs/adr/drafts/` and is numbered at ship
by `cargo xtask adr promote` (`jaunder-ship`) — it is **not** committed by this
task. Only the four comment files are.

- [ ] **Step 5: Commit**

```bash
git add csr/src/lib.rs end2end/tests/mount.ts client/src/perf/mod.rs end2end/tests/capture-trace.ts
git commit -m "docs: point the cross-language literal comments at the xlang-literal gate (#767)"
```
