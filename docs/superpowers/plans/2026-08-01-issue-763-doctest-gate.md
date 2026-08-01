# Doctest Gate Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with jaunder-iterate
> (delegating individual tasks to a subagent via jaunder-dispatch when useful).
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Run the repo's doctests under a Nix check, and reconcile the fences
found in the source against the fences the run reported, so the gate cannot
report green over a population it never looked at.

**Architecture:** A new `tools/doctests` lib crate holds the pure seams — a
`syn` fence scanner, a libtest output parser, a vocabulary/companion rule set, a
bidirectional reconciler, and the shared scan-root list. `devtool doctests emit`
drives them inside a Nix producer derivation and writes `status.json`; a
`doctests-gate` consumer fails on it; xtask pushes both steps. A host-side xtask
step covers `xtask/` and `tools/`, which the flake `src` filter cannot see.

**Tech Stack:** Rust 2021, `syn` 2 (`full`/`visit`/`extra-traits`) +
`proc-macro2` (`span-locations`), `serde`/`serde_json`, `tempfile`, clap 4,
crane/Nix, `cargo xtask`.

**Spec:**
[`docs/superpowers/specs/2026-08-01-issue-763-doctest-gate.md`](../specs/2026-08-01-issue-763-doctest-gate.md)
— "what" and "why" live there; this plan is "how". Decisions are cited as
_(D1)_…_(D11)_, acceptance criteria as _(AC1)_…_(AC20)_.

## Scope

**In:** the `tools/doctests` crate; a fixture-crate harness;
`devtool doctests emit`; the `doctests` + `doctests-gate` derivations; xtask
wiring for both halves; cleanup of all 5 `ignore` blocks; hidden-fixture
restructure and companions for all 26 uncompanioned `compile_fail` blocks; the
`Borrow<str>` proof; making the three ordering proofs discriminate; the ADR
draft.

**Out:** re-enabling `cargo test` in the package build; feeding doctests to the
coverage gate; #716; a census lock on the `text` population (D4's accepted
limitation).

**Separable concerns:** none warranting their own issue. Two small in-place
corrections are authorized by _this plan_ rather than by the spec, and are
called out where they occur: the stale "separate `nextest` check" comment at
`flake.nix:315-318` (Task 13, which edits that file anyway) and a missing
`git::tracked_files` helper (Task 15).

## Task list

| #   | Task                                                 | Deliverable                                               |
| --- | ---------------------------------------------------- | --------------------------------------------------------- |
| 1   | `tools/doctests` crate, scan roots, fence scanner    | `fence::fences`, `roots::*`                               |
| 2   | Vocabulary + companion rules                         | `check::{Violation, Kind, fence_violations}`              |
| 3   | libtest output parser                                | `libtest::{RunEntry, run_entries}`                        |
| 4   | Bidirectional reconciler + status type               | `check::{ScannedFile, problems}`, `status::DoctestStatus` |
| 5   | `devtool doctests emit --out`                        | producer writing `status.json`                            |
| 6   | Fixture-crate harness + end-to-end tests             | _(AC4, AC6, AC13, AC14)_                                  |
| 7   | Clean the 5 `ignore` blocks                          | zero `ignore` in the tree _(AC12)_                        |
| 8   | Companions: `token.rs`, `etag.rs`, `post_body.rs`    | 3 doc comments _(AC15)_                                   |
| 9   | Companions: `media.rs`                               | 4 doc comments _(AC15)_                                   |
| 10  | Companions: `render.rs` + repair `:505`              | 2 doc comments _(AC15)_                                   |
| 11  | Hidden preludes for `macros` `:43`, `:220`, `:274`   | 3 doc comments _(AC15)_                                   |
| 12  | `Borrow<str>` proof + discriminating ordering proofs | _(AC16, AC17)_                                            |
| 13  | Nix `doctests` + `doctests-gate` derivations         | _(AC1)_                                                   |
| 14  | xtask wiring: `steps::nix::doctests()`               | _(AC1, AC5)_                                              |
| 15  | Host-side `xtask/`+`tools/` step + scan-root test    | _(AC8, AC9)_                                              |
| 16  | ADR draft + module docs                              | _(AC18, AC19)_                                            |
| 17  | Full validate + coverage-unchanged proof             | _(AC20)_                                                  |

## Key risks and decisions

- **Ordering is load-bearing.** `.githooks/pre-commit:21` runs
  `cargo xtask check`. If the gate is wired in (Tasks 13–15) before the tree is
  clean (Tasks 7–12), every subsequent commit is blocked by the gate itself.
  Build the tool first, use it by hand to drive the cleanup, wire it last.
- **The population is 26 uncompanioned blocks, not 20.** Beyond `common/`'s 20,
  `macros/src/lib.rs:43`, `:220`, `:274` carry all-visible fixtures with no
  hidden prelude (Task 11), and `:145`, `:158`, `:173` are the ordering trio,
  which Task 12 makes discriminating rather than exempt — no exemption marker
  ships (D6). Task 10's "zero violations" checkpoint is only reachable after
  Tasks 11–12.
- **The scanner lives in `tools/`, not `xtask/`.** It is needed inside the Nix
  derivation (via `devtool`) _and_ host-side (via `xtask`).
  `xtask/Cargo.toml:18` already depends on `tools/coverage` — the exact
  precedent.
- **Scan roots live in `tools/doctests`,** not duplicated between `devtool` and
  `xtask`. AC9's "the union covers every `.rs`" is worthless if the list it
  checks can diverge from the list the producer scans.
- **Lock files must be staged with their manifests.** `devtoolBin` is
  `craneLib.buildPackage { src = craneLib.path ./tools; … }`
  (`flake.nix:343-354`) and vendors offline from `tools/Cargo.lock`; a lock
  without the new `doctests` package fails the Nix build. Same for
  `xtask/Cargo.lock` in Task 14.
- **Companion fences RUN.** The `compile_fail` blocks never executed, so their
  fixture values were never validated. A companion built from the same literal
  will execute — and `ContentHash::from_str("abc")` returns `Err`, so
  `.unwrap()` panics and fails the doctest run. Every companion needs a value
  its type's validator actually accepts (Tasks 8–10).
- **Deviation from AC8's wording, flagged.** AC8 says the host-side half
  "extends the existing `host_tests.rs` steps rather than duplicating them."
  Task 15 adds a _separate_ step file, because `host_tests` uses `sh::step`,
  which does not capture output, and reconciliation needs the captured run.
  `cargo test` in `host_tests` does still _execute_ those doctests; the new step
  is the only thing that _reconciles_ them. No reconciliation is duplicated.
- **Deviation from AC14's wording, flagged.** AC14 asks for "a dedicated fixture
  tree". Task 6 provides one (`tools/doctests/testdata/`) for the end-to-end
  rustdoc-behaviour tests that genuinely need real crates; the pure scanner/rule
  tests use inline `r#"…"#` sources instead, which is the repo's established
  convention for gate seams (`sqlx_newtype_decode_check.rs:600-619`; there is no
  `tests/` directory in `xtask` or `tools`).

## Global Constraints

- **Commits:** run `cargo xtask check` before every commit so the pre-commit
  gate passes clean (**jaunder-commit**). **No `Co-Authored-By` trailer.**
  Conventional-commit subjects ending `(#763)`.
- **Stage the lock with the manifest.** Any task touching `tools/Cargo.toml`
  stages `tools/Cargo.lock`; any task touching `xtask/Cargo.toml` stages
  `xtask/Cargo.lock`.
- **`xtask/` and `tools/` are each their own workspace** — they do not inherit
  `[workspace.dependencies]`, so versions are spelled literally in their own
  `Cargo.toml`.
- **Rust edition 2021** for the new crate, matching `tools/coverage`.
- **Gate invocation:**
  `devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-763-doctest-gate -- <cmd>`,
  then read the parked log. Never pipe a gated command into a filter.
- **Do not wire the gate into `check`/`validate` before Task 13.**
- Pure-seam tests are in-file `#[cfg(test)] mod tests` with inline `r#"…"#`
  sources.

---

### Task 1: `tools/doctests` crate, scan roots, and the fence scanner

**Files:**

- Create: `tools/doctests/Cargo.toml`, `tools/doctests/src/lib.rs`,
  `tools/doctests/src/roots.rs`, `tools/doctests/src/fence.rs`
- Modify: `tools/Cargo.toml` (members), `tools/Cargo.lock`

**Interfaces:**

- Consumes: nothing.
- Produces: `doctests::fence::{Fence, Scan, fences}` (Tasks 2, 4, 6) and
  `doctests::roots::{WORKSPACE, HOST, ALL}` (Tasks 5, 15).

- [x] **Step 1: Create the crate, the scan-root module, and register it**

`tools/doctests/Cargo.toml`:

```toml
[package]
name = "doctests"
version = "0.1.0"
edition = "2021"
publish = false

[dependencies]
serde = { version = "1", features = ["derive"] }
serde_json = "1"
anyhow = "1"
syn = { version = "2", features = ["full", "visit", "extra-traits"] }
proc-macro2 = { version = "1", features = ["span-locations"] }

[dev-dependencies]
tempfile = "3"
```

`tools/Cargo.toml` → `members = ["coverage", "devtool", "doctests"]`.

`tools/doctests/src/lib.rs`:

```rust
pub mod fence;
pub mod roots;
```

`tools/doctests/src/roots.rs` — one home for the lists, so the producer and the
coverage assertion can never disagree (AC9):

```rust
//! The scan roots, in one place because two crates consume them: `devtool`
//! scans [`WORKSPACE`] inside the Nix producer, and `xtask` scans [`HOST`] and
//! asserts [`ALL`] covers every tracked `.rs` file. Duplicating the lists would
//! let the population the gate *checks* drift from the population it *scans*.

/// Root-workspace directories the `cargo test --workspace --doc` run covers.
///
/// Explicit rather than "every `.rs` in the derivation source": the source also
/// carries `tools/` (only `xtask/` is filtered out, `flake.nix:272`), which the
/// workspace run does not reach — scanning it there would manufacture `NotRun`
/// violations for fences that are gated host-side instead.
pub const WORKSPACE: &[&str] = &[
    "client", "common", "csr", "host", "macros", "server", "storage",
    "test-support", "web",
];

/// Roots no Nix check can see: `xtask/` is excluded from the flake `src` filter
/// and `tools/` is a separate virtual workspace.
pub const HOST: &[&str] = &["xtask", "tools"];

/// Every scan root. The union must cover every tracked `.rs` file.
pub const ALL: &[&str] = &[
    "client", "common", "csr", "host", "macros", "server", "storage",
    "test-support", "web", "xtask", "tools",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_is_exactly_workspace_plus_host() {
        let mut want: Vec<&str> = WORKSPACE.iter().chain(HOST).copied().collect();
        want.sort_unstable();
        let mut got: Vec<&str> = ALL.to_vec();
        got.sort_unstable();
        assert_eq!(got, want);
    }

    #[test]
    fn no_root_is_a_prefix_of_another() {
        // Otherwise a file could fall under two roots and be reconciled against
        // the wrong run.
        for a in ALL {
            for b in ALL {
                assert!(a == b || !b.starts_with(&format!("{a}/")), "{a} covers {b}");
            }
        }
    }
}
```

- [x] **Step 2: Write the failing tests** in `tools/doctests/src/fence.rs`

````rust
#[cfg(test)]
mod tests {
    use super::*;

    fn scan(src: &str) -> Scan {
        fences(src).expect("parses")
    }

    #[test]
    fn a_doc_comment_fence_keys_to_its_opening_line() {
        // `///` desugars to one `#[doc]` attribute per source line, so the
        // opener's span IS the line libtest prints. Probed 2026-08-01.
        let src = "\n/// Docs.\n///\n/// ```compile_fail\n/// let _: i32 = \"x\";\n/// ```\npub struct A;\n";
        let f = &scan(src).fences[0];
        assert_eq!(f.line, 4);
        assert_eq!(f.info, "compile_fail");
    }

    #[test]
    fn module_docs_are_scanned_too() {
        let src = "//! Module.\n//!\n//! ```\n//! let x = 1;\n//! ```\n";
        let f = &scan(src).fences[0];
        assert_eq!(f.line, 3);
        assert_eq!(f.info, "");
    }

    #[test]
    fn hidden_lines_are_separated_and_stripped() {
        let src = "\n/// ```compile_fail\n/// # use foo::Bar;\n/// # let b = Bar;\n/// let _ = b.nope();\n/// ```\npub struct A;\n";
        let f = &scan(src).fences[0];
        assert_eq!(f.hidden, vec!["use foo::Bar;", "let b = Bar;"]);
        assert_eq!(f.visible, vec!["let _ = b.nope();"]);
    }

    #[test]
    fn fences_in_one_doc_comment_share_a_doc_block() {
        let src = "\n/// ```\n/// let a = 1;\n/// ```\n///\n/// ```compile_fail\n/// let _: i32 = \"x\";\n/// ```\npub struct A;\n\n/// ```\n/// let b = 2;\n/// ```\npub struct B;\n";
        let s = scan(src);
        assert_eq!(s.fences.len(), 3);
        assert_eq!(s.fences[0].doc_block, s.fences[1].doc_block);
        assert_ne!(s.fences[1].doc_block, s.fences[2].doc_block);
    }

    #[test]
    fn fences_inside_nested_items_are_found() {
        let src = "mod m {\n    /// ```\n    /// let x = 1;\n    /// ```\n    pub fn f() {}\n}\n";
        assert_eq!(scan(src).fences.len(), 1);
    }

    #[test]
    fn a_multiline_doc_attribute_is_recorded_not_scanned() {
        // libtest keys a fence inside one of these by attribute-line + markdown
        // offset, which the reconciliation key cannot address (D9).
        let src = "\n#[doc = \"Docs.\\n\\n```\\nlet x = 1;\\n```\\n\"]\npub struct A;\n";
        let s = scan(src);
        assert_eq!(s.multiline_doc_attrs, vec![2]);
        assert!(s.fences.is_empty());
    }

    #[test]
    fn a_single_line_doc_attribute_keys_like_a_slash_comment() {
        // Indistinguishable from `///` and keys correctly, so it is allowed (D9).
        let src = "\n#[doc = \" ```\"]\n#[doc = \" let x = 1;\"]\n#[doc = \" ```\"]\npub struct A;\n";
        let s = scan(src);
        assert!(s.multiline_doc_attrs.is_empty());
        assert_eq!(s.fences[0].line, 2);
    }

    #[test]
    fn an_unparseable_file_is_an_error_not_an_empty_scan() {
        // A file the gate cannot read is a file the gate cannot police (AC10).
        let err = fences("fn f( {").expect_err("must not parse");
        assert!(err.contains("cannot parse as Rust"), "{err}");
    }
}
````

- [x] **Step 3: Run the tests, verify they fail**

Run:
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-763-doctest-gate -- cargo test --manifest-path tools/Cargo.toml -p doctests`
Expected: FAIL — `Fence`, `Scan`, `fences` not defined.

**Done differently, recorded:** tests and implementation were written together, so
there was no red run. Falsification was done instead by mutation — disabling
`hidden_code` to always return `None` failed exactly
`hidden_lines_are_separated_and_stripped` (9 passed, 1 failed), then reverted. That
establishes the same thing the red step does: the tests are not vacuous.

- [x] **Step 4: Implement against the tests**

````rust
/// One rustdoc code fence, keyed the way the doctest runner reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fence {
    /// 1-based source line the opening ``` sits on — the runner's key.
    pub line: usize,
    /// The info string after the opening backticks, trimmed.
    pub info: String,
    /// Body lines rustdoc hides (`# `-prefixed), stored with the `# ` stripped so
    /// a hidden line and a visible line carrying the same code compare equal.
    pub hidden: Vec<String>,
    /// Body lines rustdoc shows, likewise trimmed.
    pub visible: Vec<String>,
    /// Index of the doc comment this fence belongs to. Fences sharing a value sit
    /// in one doc comment — the scope of the companion rule (D7).
    pub doc_block: usize,
}

/// Everything the scanner read out of one file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Scan {
    pub fences: Vec<Fence>,
    /// 1-based lines of `#[doc = "…"]` attributes whose value spans several
    /// markdown lines. Rejected rather than scanned (D9).
    pub multiline_doc_attrs: Vec<usize>,
}

/// Every fence in `source`, or a message describing why it is not readable Rust.
///
/// A file that will not parse is **not** silently skipped: an unparsed file is a
/// file the gate cannot see, and a gate that quietly shrinks its own population is
/// the failure this design exists to prevent (ADR-0085 principle 6).
pub fn fences(source: &str) -> Result<Scan, String>;
````

Implementation notes the tests cannot pin:

- Walk with `syn::visit::Visit`, overriding the `visit_item_*` arms so nested
  items are reached; collect `#[doc]` attributes per item. Follow the `Scanner`
  shape at `xtask/src/steps/sqlx_newtype_decode_check.rs:291-342`.
- For each `#[doc]` attribute: match `syn::Meta::NameValue` →
  `syn::Expr::Lit(syn::ExprLit { lit: syn::Lit::Str(s), .. })`.
  `s.span().start().line` is the source line; `s.value()` is the text. If
  `s.value().contains('\n')`, push the line to `multiline_doc_attrs` and do not
  scan it.
- Strip one leading space from each doc value (rustdoc's convention) before
  fence detection.
- A body line is hidden when, after that strip, it equals `#` or starts with
  `# `; store it with the `# ` removed and trimmed. Visible lines are trimmed.
- A doc comment ends when the attribute run ends; increment `doc_block` per run,
  counting across the whole file so indices are unique.
- Fence state: a line whose trimmed text starts with ` ``` ` toggles. The
  opener's remainder (after the backticks, trimmed) is `info`.

- [x] **Step 5: Run the tests, verify they pass**

Run:
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-763-doctest-gate -- cargo test --manifest-path tools/Cargo.toml -p doctests`
Expected: PASS — 10 tests (8 fence + 2 roots). **Actual: 10 passed.**

- [x] **Step 6: Commit**

```bash
git add tools/Cargo.toml tools/Cargo.lock tools/doctests
git commit -m "feat(doctests): syn-based rustdoc fence scanner and shared scan roots (#763)"
```

---

### Task 2: Vocabulary and companion rules

**Files:**

- Create: `tools/doctests/src/check.rs`
- Modify: `tools/doctests/src/lib.rs` — add `pub mod check;`

**Interfaces:**

- Consumes: `fence::{Fence, Scan}`.
- Produces: `check::{Violation, Kind, fence_violations}` (Task 4).

- [ ] **Step 1: Write the failing tests** in `tools/doctests/src/check.rs`

````rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::fence::fences;

    fn kinds(src: &str) -> Vec<Kind> {
        let scan = fences(src).expect("parses");
        fence_violations("f.rs", &scan).into_iter().map(|v| v.kind).collect()
    }

    #[test]
    fn the_three_accepted_forms_pass() {
        let src = "\n/// ```\n/// # let s = 1;\n/// let _ = s;\n/// ```\n///\n/// ```compile_fail\n/// # let s = 1;\n/// let _: &str = s;\n/// ```\n///\n/// ```text\n/// not rust\n/// ```\npub struct A;\n";
        assert!(kinds(src).is_empty(), "{:?}", kinds(src));
    }

    #[test]
    fn ignore_is_banned() {
        let src = "\n/// ```ignore\n/// let x = 1;\n/// ```\npub struct A;\n";
        assert_eq!(kinds(src), vec![Kind::BannedAttribute]);
    }

    #[test]
    fn no_run_and_should_panic_are_banned() {
        let a = "\n/// ```no_run\n/// let x = 1;\n/// ```\npub struct A;\n";
        let b = "\n/// ```should_panic\n/// panic!();\n/// ```\npub struct B;\n";
        assert_eq!(kinds(a), vec![Kind::BannedAttribute]);
        assert_eq!(kinds(b), vec![Kind::BannedAttribute]);
    }

    #[test]
    fn an_unknown_tag_is_banned_because_rustdoc_drops_it_silently() {
        // Probed: a wholly unrecognized word makes rustdoc skip the block with no
        // warning at all, deleting the proof.
        let src = "\n/// ```rust,nocheck\n/// let x = 1;\n/// ```\npub struct A;\n";
        assert_eq!(kinds(src), vec![Kind::BannedAttribute]);
    }

    #[test]
    fn a_language_tag_is_banned_text_is_the_only_non_rust_form() {
        let src = "\n/// ```sql\n/// SELECT 1;\n/// ```\npub struct A;\n";
        assert_eq!(kinds(src), vec![Kind::BannedAttribute]);
    }

    #[test]
    fn a_compile_fail_with_a_trailing_marker_word_is_banned() {
        // No exemption marker ships (D6). A `compile_fail,<anything>` is an
        // unrecognized form, so adding one back is a deliberate edit here.
        let src = "\n/// ```compile_fail,intent_only\n/// let _ = nope();\n/// ```\npub struct A;\n";
        assert_eq!(kinds(src), vec![Kind::BannedAttribute]);
    }

    #[test]
    fn a_multiline_doc_attribute_is_reported() {
        let src = "\n#[doc = \"```\\nlet x = 1;\\n```\\n\"]\npub struct A;\n";
        assert_eq!(kinds(src), vec![Kind::MultilineDocAttr]);
    }

    #[test]
    fn a_compile_fail_with_no_hidden_prelude_fails() {
        // Forces the fixture to be explicit and therefore matchable (D7). This is
        // the rule that catches macros/src/lib.rs:43, :220 and :274.
        let src = "\n/// ```\n/// let a = 1;\n/// ```\n///\n/// ```compile_fail\n/// let _: i32 = \"x\";\n/// ```\npub struct A;\n";
        assert_eq!(kinds(src), vec![Kind::MissingCompanion]);
    }

    #[test]
    fn a_hidden_line_matched_by_no_companion_fails() {
        let src = "\n/// ```\n/// # use foo::Bar;\n/// let _ = Bar;\n/// ```\n///\n/// ```compile_fail\n/// # use foo::Baz;\n/// let _ = Baz.nope();\n/// ```\npub struct A;\n";
        let v = {
            let scan = fences(src).expect("parses");
            fence_violations("f.rs", &scan)
        };
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].kind, Kind::MissingCompanion);
        assert!(v[0].detail.contains("use foo::Baz;"), "{}", v[0].detail);
    }

    #[test]
    fn a_hidden_line_matching_a_companions_visible_line_passes() {
        // macros/src/lib.rs:51 shows `use macros::StrNewtype;` visibly while the
        // negative at :43 will hide it; both spellings are the same fixture line.
        let src = "\n/// ```\n/// use foo::Bar;\n/// let s = Bar;\n/// ```\n///\n/// ```compile_fail\n/// # use foo::Bar;\n/// # let s = Bar;\n/// let _: i32 = s;\n/// ```\npub struct A;\n";
        assert!(kinds(src).is_empty(), "{:?}", kinds(src));
    }

    #[test]
    fn the_companion_rule_has_no_exemption() {
        // Every `compile_fail` needs a matched hidden prelude, full stop (D6) —
        // the three proofs that would have needed an exemption were made
        // discriminating instead (Task 12).
        let src = "\n/// ```compile_fail\n/// let _ = nope();\n/// ```\npub struct A;\n";
        assert_eq!(kinds(src), vec![Kind::MissingCompanion]);
    }

    #[test]
    fn a_companion_in_a_different_doc_comment_does_not_count() {
        // Otherwise one companion silently covers unrelated negatives elsewhere in
        // the file — the region-scoped exemption ADR-0085 principle 4 forbids.
        let src = "\n/// ```\n/// # use foo::Bar;\n/// let _ = Bar;\n/// ```\npub struct A;\n\n/// ```compile_fail\n/// # use foo::Bar;\n/// let _: i32 = Bar;\n/// ```\npub struct B;\n";
        assert_eq!(kinds(src), vec![Kind::MissingCompanion]);
    }
}
````

- [ ] **Step 2: Run the tests, verify they fail**

Run:
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-763-doctest-gate -- cargo test --manifest-path tools/Cargo.toml -p doctests`
Expected: FAIL — `Violation`, `Kind`, `fence_violations` not defined.

- [ ] **Step 3: Implement against the tests**

```rust
/// One thing wrong with the fence population.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Violation {
    /// Repo-relative path.
    pub file: String,
    /// 1-based line of the fence, or of the offending attribute.
    pub line: usize,
    pub kind: Kind,
    /// Human detail, ending in the recovery instruction.
    pub detail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    /// An info string outside the accepted vocabulary.
    BannedAttribute,
    /// A fence inside a multi-line `#[doc = "…"]` value.
    MultilineDocAttr,
    /// A `compile_fail` with no hidden prelude, or a hidden line no companion
    /// in the same doc comment carries.
    MissingCompanion,
    /// Scanned in the tree, absent from the run.
    NotRun,
    /// The run reported this doctest as FAILED.
    Failed,
    /// Reported by the run, matched by no scanned fence.
    Orphan,
    /// A file under a scan root that could not be read or parsed.
    Unreadable,
}

/// The accepted vocabulary, exhaustively (D4, AC11), compared after removing all
/// whitespace. Anything else fails: the set grows only by a deliberate edit here,
/// never by a fence tagging itself.
const PLAIN: &str = "";
const COMPILE_FAIL: &str = "compile_fail";
const TEXT: &str = "text";

/// Vocabulary and companion violations in one file's scan.
pub fn fence_violations(file: &str, scan: &Scan) -> Vec<Violation>;
```

The tests pin every branch. One detail they cannot express: every `detail` ends
with a recovery instruction naming the fix, following the `"  recovery: …"`
footer convention at `sqlx_newtype_decode_check.rs:539-551`.

- [ ] **Step 4: Run the tests, verify they pass**

Run:
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-763-doctest-gate -- cargo test --manifest-path tools/Cargo.toml -p doctests`
Expected: PASS — 12 new tests.

- [ ] **Step 5: Commit**

```bash
git add tools/doctests/src/check.rs tools/doctests/src/lib.rs
git commit -m "feat(doctests): closed fence vocabulary and hidden-prelude companion rule (#763)"
```

---

### Task 3: libtest output parser

**Files:**

- Create: `tools/doctests/src/libtest.rs`
- Modify: `tools/doctests/src/lib.rs` — add `pub mod libtest;`

**Interfaces:**

- Consumes: nothing.
- Produces: `libtest::{RunEntry, run_entries}` (Task 4).

- [ ] **Step 1: Write the failing tests** in `tools/doctests/src/libtest.rs`

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// Verbatim from `cargo test --workspace --doc` on this tree, 2026-08-01.
    const REAL: &str = "\
running 3 tests
test common/src/token.rs - token::RawToken (line 56) - compile fail ... ok
test common/src/etag.rs - etag::ETag (line 35) - compile fail ... ok
test common/src/post_body.rs - post_body::PostBody (line 15) - compile fail ... ok

test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.31s
";

    #[test]
    fn parses_file_and_line_from_real_output() {
        let e = run_entries(REAL);
        assert_eq!(e.len(), 3);
        assert_eq!(e[0].file, "common/src/token.rs");
        assert_eq!(e[0].line, 56);
        assert!(!e[0].ignored && !e[0].failed);
    }

    #[test]
    fn parses_a_module_doc_entry_with_no_item_path() {
        let out = "test src/lib.rs - (line 4) - compile fail ... ok\n";
        let e = run_entries(out);
        assert_eq!(e[0].file, "src/lib.rs");
        assert_eq!(e[0].line, 4);
    }

    #[test]
    fn records_ignored_and_failed_separately() {
        let out = "\
test a.rs - a::A (line 3) ... ignored
test b.rs - b::B (line 7) ... FAILED
";
        let e = run_entries(out);
        assert!(e[0].ignored && !e[0].failed);
        assert!(e[1].failed && !e[1].ignored);
    }

    #[test]
    fn ignores_summary_and_noise_lines() {
        assert!(run_entries("running 3 tests\ntest result: ok. 3 passed;\n\n").is_empty());
    }

    #[test]
    fn a_path_containing_a_dash_is_not_split_early() {
        // `test-support/src/x.rs` contains the same " - " separator the item path
        // uses, so a naive first-split loses the crate directory.
        let out = "test test-support/src/x.rs - x::Y (line 9) ... ok\n";
        assert_eq!(run_entries(out)[0].file, "test-support/src/x.rs");
    }
}
```

- [ ] **Step 2: Run the tests, verify they fail**

Run:
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-763-doctest-gate -- cargo test --manifest-path tools/Cargo.toml -p doctests`
Expected: FAIL — `RunEntry`, `run_entries` not defined.

- [ ] **Step 3: Implement against the tests**

```rust
/// One doctest as the runner reported it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunEntry {
    /// Path as printed — relative to the invoked manifest's directory, so a
    /// `--manifest-path xtask/Cargo.toml` run prints `src/…`, not `xtask/src/…`.
    pub file: String,
    /// The `(line N)` the runner printed. For `///` and `//!` docs this is the
    /// fence's opening line (verified against this tree, 2026-08-01).
    pub line: usize,
    pub ignored: bool,
    pub failed: bool,
}

/// Every doctest result line in `output`; everything else is skipped.
pub fn run_entries(output: &str) -> Vec<RunEntry>;
```

Parse each line starting with `test ` and containing `...`: split into head and
outcome at the **last** `...`; find the **last** `(line N)` in the head; the
file is the head up to the last `-` preceding that `(line N)`, which the
dash-in-path test pins.

- [ ] **Step 4: Run the tests, verify they pass**

Run:
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-763-doctest-gate -- cargo test --manifest-path tools/Cargo.toml -p doctests`
Expected: PASS — 5 new tests.

- [ ] **Step 5: Commit**

```bash
git add tools/doctests/src/libtest.rs tools/doctests/src/lib.rs
git commit -m "feat(doctests): parse libtest doctest result lines (#763)"
```

---

### Task 4: Bidirectional reconciler and status type

**Files:**

- Create: `tools/doctests/src/status.rs`
- Modify: `tools/doctests/src/check.rs` (add `ScannedFile`, `problems`),
  `tools/doctests/src/lib.rs`

**Interfaces:**

- Consumes: `fence::fences`, `check::fence_violations`, `libtest::run_entries`.
- Produces: `check::{ScannedFile, problems}` and
  `status::{DoctestStatus, StatusCategory}` (Tasks 5, 6, 14).

- [ ] **Step 1: Write the failing tests** for `problems` in `check.rs`'s test
      module

````rust
    fn file(path: &str, run_path: &str, source: &str) -> ScannedFile {
        ScannedFile {
            path: path.to_string(),
            run_path: run_path.to_string(),
            source: source.to_string(),
        }
    }

    /// A companion at line 2 and a matched `compile_fail` at line 7.
    const OK_SRC: &str = "\n/// ```\n/// # use foo::Bar;\n/// let _ = Bar;\n/// ```\n///\n/// ```compile_fail\n/// # use foo::Bar;\n/// let _: i32 = Bar;\n/// ```\npub struct A;\n";

    #[test]
    fn a_tree_matching_its_run_has_no_problems() {
        let out = "test a.rs - a::A (line 2) ... ok\ntest a.rs - a::A (line 7) - compile fail ... ok\n";
        assert!(problems(&[file("a.rs", "a.rs", OK_SRC)], out).is_empty());
    }

    #[test]
    fn a_fence_in_the_tree_but_not_in_the_run_fails() {
        let out = "test a.rs - a::A (line 2) ... ok\n";
        let v = problems(&[file("a.rs", "a.rs", OK_SRC)], out);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].kind, Kind::NotRun);
        assert_eq!(v[0].line, 7);
    }

    #[test]
    fn a_run_entry_matching_no_scanned_fence_fails() {
        // The gate shrinking its OWN population — principle 6 turned inward (AC7).
        let out = "test a.rs - a::A (line 2) ... ok\ntest a.rs - a::A (line 7) ... ok\ntest a.rs - a::A (line 99) ... ok\n";
        let v = problems(&[file("a.rs", "a.rs", OK_SRC)], out);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].kind, Kind::Orphan);
        assert_eq!(v[0].line, 99);
    }

    #[test]
    fn a_text_fence_must_not_appear_in_the_run() {
        let src = "\n/// ```text\n/// prose\n/// ```\npub struct A;\n";
        let v = problems(&[file("a.rs", "a.rs", src)], "test a.rs - a::A (line 2) ... ok\n");
        assert_eq!(v[0].kind, Kind::Orphan);
    }

    #[test]
    fn an_ignored_run_entry_does_not_count_as_run() {
        // `ignore` blocks ARE reported by libtest, so presence alone is not proof
        // the proof was evaluated — which is why `ignore` is banned (D5).
        let src = "\n/// ```ignore\n/// let x = 1;\n/// ```\npub struct A;\n";
        let v = problems(&[file("a.rs", "a.rs", src)], "test a.rs - a::A (line 2) ... ignored\n");
        let kinds: Vec<_> = v.iter().map(|x| x.kind).collect();
        assert!(kinds.contains(&Kind::BannedAttribute), "{kinds:?}");
        assert!(kinds.contains(&Kind::NotRun), "{kinds:?}");
    }

    #[test]
    fn a_failed_doctest_is_named_as_failed_not_as_unrun() {
        // AC4 wants a failing doctest reported AS a failure; folding it into
        // NotRun ("scanned but never evaluated") would be a misleading message.
        let out = "test a.rs - a::A (line 2) ... ok\ntest a.rs - a::A (line 7) ... FAILED\n";
        let v = problems(&[file("a.rs", "a.rs", OK_SRC)], out);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].kind, Kind::Failed);
        assert_eq!(v[0].line, 7);
    }

    #[test]
    fn the_run_path_is_used_for_matching_and_the_repo_path_for_reporting() {
        // A `--manifest-path xtask/Cargo.toml` run prints `src/…` (D10).
        let out = "test src/a.rs - a::A (line 2) ... ok\ntest src/a.rs - a::A (line 7) - compile fail ... ok\n";
        assert!(problems(&[file("xtask/src/a.rs", "src/a.rs", OK_SRC)], out).is_empty());
    }

    #[test]
    fn a_failure_is_reported_against_the_repo_relative_path() {
        let out = "test src/a.rs - a::A (line 2) ... ok\n";
        let v = problems(&[file("xtask/src/a.rs", "src/a.rs", OK_SRC)], out);
        assert_eq!(v[0].file, "xtask/src/a.rs");
    }

    #[test]
    fn an_unparseable_file_is_a_violation_not_a_skip() {
        let v = problems(&[file("a.rs", "a.rs", "fn f( {")], "");
        assert_eq!(v[0].kind, Kind::Unreadable);
    }
````

- [ ] **Step 2: Write the failing tests** in `tools/doctests/src/status.rs`

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::check::{Kind, Violation};

    fn status(category: StatusCategory, violations: Vec<Violation>) -> DoctestStatus {
        DoctestStatus { category, violations, infra_detail: None }
    }

    #[test]
    fn roundtrips_through_json() {
        let s = status(
            StatusCategory::Violations,
            vec![Violation {
                file: "a.rs".to_string(),
                line: 7,
                kind: Kind::NotRun,
                detail: "d".to_string(),
            }],
        );
        assert_eq!(DoctestStatus::from_json(&s.to_json()).unwrap(), s);
    }

    #[test]
    fn category_serializes_kebab_case() {
        assert!(status(StatusCategory::Ok, vec![]).to_json().contains("\"ok\""));
    }

    #[test]
    fn violation_kind_serializes_kebab_case() {
        // The gate derivation's jq prints `\(.kind)` straight into the failure
        // message, and xtask's renderer matches on it.
        let s = status(
            StatusCategory::Violations,
            vec![Violation {
                file: "a.rs".to_string(),
                line: 7,
                kind: Kind::NotRun,
                detail: "d".to_string(),
            }],
        );
        assert!(s.to_json().contains("\"not-run\""), "{}", s.to_json());
    }
}
```

- [ ] **Step 3: Run the tests, verify they fail**

Run:
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-763-doctest-gate -- cargo test --manifest-path tools/Cargo.toml -p doctests`
Expected: FAIL — `ScannedFile`, `problems`, `DoctestStatus`, `StatusCategory`
not defined.

- [ ] **Step 4: Implement against the tests**

In `check.rs`:

```rust
/// One `.rs` file under a scan root, carrying both spellings of its path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScannedFile {
    /// Repo-relative, for the report.
    pub path: String,
    /// As the runner prints it — repo-relative for a workspace run,
    /// manifest-relative for a `--manifest-path` run (D10).
    pub run_path: String,
    pub source: String,
}

/// Every problem with the population, in `(file, line, kind)` order: vocabulary
/// and companion violations, fences the run never evaluated, doctests the run
/// reported as failed, run entries matching no fence, and files that would not
/// parse. Empty means the tree and the run agree.
///
/// Pure given `(scanned, output)`, so it is unit-tested directly.
pub fn problems(scanned: &[ScannedFile], output: &str) -> Vec<Violation>;
```

A fence is _run_ when a `RunEntry` with the same `(run_path, line)` exists and
is neither `ignored` nor `failed`; a `failed` entry becomes `Kind::Failed`, an
`ignored` one leaves the fence `NotRun`. `text` fences must have no entry at
all.

In `status.rs`, mirror `tools/coverage/src/status.rs` exactly — same derives,
kebab-case rename, `to_json` (pretty, trailing newline) / `from_json`. Note
there is deliberately **no** `failed_doctests` field: failures are `Violation`s
with `Kind::Failed`, so the gate's jq has one list to render.

```rust
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StatusCategory {
    Ok,
    Violations,
    Infra,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DoctestStatus {
    pub category: StatusCategory,
    #[serde(default)]
    pub violations: Vec<crate::check::Violation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub infra_detail: Option<String>,
}
```

- [ ] **Step 5: Run the tests, verify they pass**

Run:
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-763-doctest-gate -- cargo test --manifest-path tools/Cargo.toml -p doctests`
Expected: PASS — 12 new tests.

- [ ] **Step 6: Commit**

```bash
git add tools/doctests/src
git commit -m "feat(doctests): bidirectional fence/run reconciler and status sentinel (#763)"
```

---

### Task 5: `devtool doctests emit --out`

**Files:**

- Create: `tools/devtool/src/doctests/mod.rs`,
  `tools/devtool/src/doctests/emit.rs`
- Modify: `tools/devtool/src/main.rs:6-13`, `:24-43`, `:96-104`, `:116-128`
- Modify: `tools/devtool/Cargo.toml`, `tools/Cargo.lock`

**Interfaces:**

- Consumes:
  `doctests::{roots::WORKSPACE, check::{ScannedFile, problems}, status::*}`.
- Produces: `devtool doctests emit --out <dir>` and `$out/status.json` (Tasks 6,
  13, 14).

- [ ] **Step 1: Write the failing tests** in
      `tools/devtool/src/doctests/emit.rs`

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_run_command_is_workspace_scoped_not_package_scoped() {
        // AC2: `-p common -p macros --doc` is exactly the invocation that skips
        // the three `#[cfg(feature = "sanitize")]` fences. `--workspace` picks
        // them up via storage/Cargo.toml:12's `features = ["sqlx", "sanitize"]`.
        // Asserted on the command `run` actually builds, not on a free helper a
        // divergent `run` could ignore.
        let cmd = doctest_command();
        let args: Vec<String> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(args, vec!["test", "--workspace", "--doc"]);
        assert_eq!(cmd.get_program().to_string_lossy(), "cargo");
    }

    #[test]
    fn the_scan_roots_are_the_shared_workspace_list() {
        // Not a local copy — `tools/doctests/src/roots.rs` is the single home, so
        // the population xtask asserts over cannot drift from the one scanned.
        assert_eq!(SCAN_ROOTS, doctests::roots::WORKSPACE);
        assert!(!SCAN_ROOTS.iter().any(|r| *r == "tools" || *r == "xtask"));
    }
}
```

- [ ] **Step 2: Run the tests, verify they fail**

Run:
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-763-doctest-gate -- cargo test --manifest-path tools/Cargo.toml -p devtool`
Expected: FAIL — `doctest_command`, `SCAN_ROOTS` not defined.

- [ ] **Step 3: Implement against the tests**

Add `doctests = { path = "../doctests" }` to `tools/devtool/Cargo.toml`.

`tools/devtool/src/doctests/mod.rs`: `pub mod emit;`

`tools/devtool/src/doctests/emit.rs`, mirroring `coverage/emit.rs:55-93`:

```rust
/// The roots this producer scans — the shared workspace list.
pub const SCAN_ROOTS: &[&str] = doctests::roots::WORKSPACE;

/// The doctest invocation: `--workspace`, never `-p` (AC2).
pub fn doctest_command() -> std::process::Command {
    let mut c = std::process::Command::new("cargo");
    c.args(["test", "--workspace", "--doc"]);
    c
}

/// Run the workspace doctests, reconcile them against the scanned fences, and
/// write `out/status.json` plus a diagnostics log. Always writes a status and
/// returns `Ok` so the Nix producer derivation can always realize `$out`; gating
/// is the `doctests-gate` consumer. Returns `Err` only if the emit could not run
/// at all (e.g. failing to spawn cargo).
pub fn run(out: &str) -> anyhow::Result<()>;
```

`run` walks each root for `.rs` files (here `path` and `run_path` are both
repo-relative, since this is a workspace run), reads each into a `ScannedFile`,
captures combined stdout+stderr from `doctest_command()`, writes it to
`out/diagnostics/doctests.log`, calls `problems`, and writes `DoctestStatus`
with category `Ok` when empty and `Violations` otherwise. A file that cannot be
**read** becomes a `Kind::Unreadable` violation, never a silent drop — same
reasoning as `sqlx_newtype_decode_check.rs:569-583`.

- [ ] **Step 4: Run the tests, verify they pass**

Run:
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-763-doctest-gate -- cargo test --manifest-path tools/Cargo.toml`
Expected: PASS.

- [ ] **Step 5: Run it against the real tree and record the baseline**

Run:
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-763-doctest-gate -- cargo run --manifest-path tools/Cargo.toml -p devtool -- doctests emit --out /tmp/claude-1000/-home-mdorman-src-jaunder/e947b2d0-6d3a-4e7c-9e0a-b41042681c7e/scratchpad/dt-baseline`

Expected: exit 0, `status.json` `category: "violations"`, listing exactly:

- `macros/src/lib.rs:298`, `:353` — `banned-attribute` (`ignore`) + `not-run`
- `web/src/reactive/scope.rs:16` — `banned-attribute` + `not-run`
- 20 `missing-companion` across `common/`
- 3 `missing-companion` at `macros/src/lib.rs:43`, `:220`, `:274`
- 3 `missing-companion` at `macros/src/lib.rs:145`, `:158`, `:173`
- no `unreadable`, no `orphan`, no `failed`

Total 26 `missing-companion`. `xtask/`'s two `ignore` fences are **not** listed
— they are outside `WORKSPACE` and are gated by Task 15. If any _other_
violation appears, stop and reconcile it against the spec's inventory: an
unexpected violation means the scanner and the spec disagree about the
population.

- [ ] **Step 6: Commit**

```bash
git add tools/devtool tools/Cargo.lock
git commit -m "feat(devtool): doctests emit subcommand driving the fence reconciler (#763)"
```

---

### Task 6: Fixture-crate harness and the end-to-end tests

**Files:**

- Create: `tools/doctests/testdata/README.md` and one `.rs` fixture per case
- Create: `tools/doctests/src/harness.rs` (test-only)
- Modify: `tools/doctests/src/lib.rs`

**Interfaces:**

- Consumes: `check::problems`, `libtest::run_entries`.
- Produces: the AC13/AC14/AC6/AC4 evidence. Nothing later depends on it.

**Why real crates:** AC13 asserts that the ordering proofs Task 12 writes really
do **discriminate** — that the same fixture shape without the suppressing option
orders — which is a fact about what the compiler accepts, not about our scanner.
The same applies to AC6's "a crate with no lib target" and to AC4's failing-run
case. These need a crate that is actually compiled and run.

- [ ] **Step 1: Write the harness**

`tools/doctests/src/harness.rs`, gated `#![cfg(test)]`:

```rust
//! Materializes a one-file fixture crate into a tempdir and runs its doctests, so
//! tests can assert what rustdoc *actually does* rather than what we believe it
//! does. Fixture sources live in `testdata/` and are `include_str!`'d, following
//! the `xtask/src/pr/testdata/` convention.

/// Write `source` as a lib crate named `name` under a fresh tempdir, run
/// `cargo test --doc` in it, and return the combined output.
pub fn run_fixture(name: &str, source: &str) -> (tempfile::TempDir, String);

/// As [`run_fixture`], but the crate has only `src/main.rs` — no lib target, so
/// cargo collects no doctests from it at all.
pub fn run_bin_fixture(name: &str, source: &str) -> (tempfile::TempDir, String);
```

- [ ] **Step 2: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::check::{problems, Kind, ScannedFile};
    use crate::libtest::run_entries;

    const ORDERING_CONTROL: &str = include_str!("../testdata/ordering_control.rs");
    const CFG_FEATURE: &str = include_str!("../testdata/cfg_feature.rs");
    const CFG_TEST_MODULE: &str = include_str!("../testdata/cfg_test_module.rs");
    const UNKNOWN_TAG: &str = include_str!("../testdata/unknown_tag.rs");
    const FAILING: &str = include_str!("../testdata/failing.rs");
    const BIN_ONLY: &str = include_str!("../testdata/bin_only.rs");

    fn scanned(source: &str) -> Vec<ScannedFile> {
        vec![ScannedFile {
            path: "src/lib.rs".to_string(),
            run_path: "src/lib.rs".to_string(),
            source: source.to_string(),
        }]
    }

    #[test]
    fn the_ordering_proofs_actually_discriminate() {
        // AC13. Task 12 replaces three self-declared non-discriminating proofs
        // with real ones by giving each fixture `PartialEq, Eq`. That only works
        // if the UNSUPPRESSED shape orders — otherwise `a < b` would fail for the
        // missing PartialOrd either way and the "proof" is as vacuous as before.
        // The control fence in this fixture is what rules that out.
        let (_dir, out) = run_fixture("ordering_control", ORDERING_CONTROL);
        let e = run_entries(&out);
        // 1 control (plain, must pass) + 3 negatives (compile_fail, must pass).
        assert_eq!(e.len(), 4, "{out}");
        assert!(e.iter().all(|x| !x.ignored && !x.failed), "{out}");
        assert!(problems(&scanned(ORDERING_CONTROL), &out).is_empty(), "{out}");
    }

    #[test]
    fn vector_1_a_fence_behind_an_unenabled_feature_is_not_run() {
        // AC6. The `sanitize` case that made the issue's own measurement wrong.
        let (_dir, out) = run_fixture("cfg_feature", CFG_FEATURE);
        let v = problems(&scanned(CFG_FEATURE), &out);
        assert!(v.iter().any(|x| x.kind == Kind::NotRun), "{v:?}\n{out}");
    }

    #[test]
    fn vector_2_a_fence_in_a_cfg_test_module_is_not_run() {
        // rustdoc sets cfg(doctest), not cfg(test) — web/src/reactive/scope.rs.
        let (_dir, out) = run_fixture("cfg_test_module", CFG_TEST_MODULE);
        let v = problems(&scanned(CFG_TEST_MODULE), &out);
        assert!(v.iter().any(|x| x.kind == Kind::NotRun), "{v:?}\n{out}");
    }

    #[test]
    fn vector_3_an_unrecognized_info_string_is_silently_uncollected() {
        // Probed: no warning at all. Both the vocabulary rule and the reconciler
        // must catch it, because either alone would let it through.
        let (_dir, out) = run_fixture("unknown_tag", UNKNOWN_TAG);
        assert!(run_entries(&out).is_empty(), "{out}");
        let v = problems(&scanned(UNKNOWN_TAG), &out);
        assert!(v.iter().any(|x| x.kind == Kind::BannedAttribute), "{v:?}");
        assert!(v.iter().any(|x| x.kind == Kind::NotRun), "{v:?}");
    }

    #[test]
    fn vector_5_a_crate_with_no_lib_target_yields_no_doctests() {
        // AC6. cargo collects doctests from lib targets only — tools/devtool has
        // src/main.rs and no src/lib.rs, so a fence added there can never run.
        let (_dir, out) = run_bin_fixture("bin_only", BIN_ONLY);
        assert!(run_entries(&out).is_empty(), "{out}");
    }

    #[test]
    fn a_failing_doctest_is_reported_as_failed() {
        // AC4, end to end against a real run rather than a synthesized log.
        let (_dir, out) = run_fixture("failing", FAILING);
        let v = problems(&scanned(FAILING), &out);
        assert_eq!(v.len(), 1, "{v:?}\n{out}");
        assert_eq!(v[0].kind, Kind::Failed);
    }
}
```

Vector 4 ("a crate outside every scan root") is structural, not observable in a
fixture crate — it is covered by Task 15's `ALL` coverage assertion instead. Say
so in `testdata/README.md`.

- [ ] **Step 3: Write the fixtures**

Six files under `tools/doctests/testdata/`, each a single lib (or bin) source:

- `ordering_control.rs` — depends on `macros` by path. Four fences in one doc
  comment: a plain **control** asserting `a < b` on an un-suppressed
  `StrNewtype` fixture that derives `PartialEq, Eq`, and three `compile_fail`s
  on the `no_ord`, `secret`, and `secret, serde` variants of the same shape.
  Note a `secret` emits its own redacting `Debug`, so the fixture must **not**
  derive `Debug` (probed: E0119 conflicting implementations).
- `cfg_feature.rs` — a fence inside `#[cfg(feature = "off")] mod gated { }`,
  with the feature declared but not default.
- `cfg_test_module.rs` — a fence inside `#[cfg(test)] mod gated { }`.
- `unknown_tag.rs` — one ` ```nocheck ` fence.
- `failing.rs` — one plain fence whose body is `assert!(false);`.
- `bin_only.rs` — a `fn main() {}` with a documented item carrying a plain
  fence.

`README.md` records that these are _synthesized_, what each pins, and that
vector 4 is covered elsewhere — following the provenance convention of
`xtask/src/server_fn_coverage/testdata/README.md`.

- [ ] **Step 4: Run the tests, verify they pass**

Run:
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-763-doctest-gate -- cargo test --manifest-path tools/Cargo.toml -p doctests`
Expected: PASS — 6 new tests. These compile real crates, so they are slower than
the rest; that cost is the price of asserting rustdoc's behaviour rather than
assuming it.

- [ ] **Step 5: Commit**

```bash
git add tools/doctests
git commit -m "test(doctests): fixture crates pinning rustdoc collection behaviour (#763)"
```

---

### Task 7: Clean the five `ignore` blocks

**Files:**

- Modify: `xtask/src/steps/proffered_filename_check.rs:19`, `:107`;
  `macros/src/lib.rs:298`, `:353`; `web/src/reactive/scope.rs:16`

**Interfaces:**

- Consumes: the gate from Task 5.
- Produces: zero `ignore` fences — a precondition for Tasks 14–15.

- [ ] **Step 1: Convert the four illustration-only fences to `text`**

Change ` ```ignore ` to ` ```text ` at:

- `xtask/src/steps/proffered_filename_check.rs:19` — illustrates a
  `server`-crate struct (`SoftPath<ProfferedFilename>`); those types are not in
  `xtask`'s graph.
- `xtask/src/steps/proffered_filename_check.rs:107` — scanner _input data_, on a
  private `fn violations`; the real assertions are already in this file's
  `mod tests`.
- `macros/src/lib.rs:298` — `#[macros::server]` calls
  `proc_macro::Span::call_site().file()` and hard-fails outside
  `web/src/<vertical>/api.rs`, so it can never be a doctest.
- `web/src/reactive/scope.rs:16` — `invalidator_scope!` is `pub(crate) use`
  only, and the module is `#[cfg(any(target_arch = "wasm32", test))]`, invisible
  to rustdoc.

Add one sentence above each explaining why it is an illustration, so the marker
carries its reason at the site (D4).

- [ ] **Step 2: Promote the `text_enum` fence to a real doctest**

`macros/src/lib.rs:353` becomes a plain fence with concrete variants, `sqlx`
dropped from the attribute (the `sqlx = []` feature carries no deps, so the
bridge cannot expand here), and a following prose sentence noting production
call sites add `sqlx`. `macros`' dev-dependencies already carry `serde`,
`serde_json`, and `strum` with `derive` for exactly this
(`macros/Cargo.toml:28-36`).

- [ ] **Step 3: Verify the three roots the producer can see**

Run the Task 5 command. Expected: zero `banned-attribute` and zero `not-run`;
only `missing-companion` remain (26 of them).

- [ ] **Step 4: Verify the two `xtask` fences separately**

The producer does not scan `xtask/` (that is Task 15). Until then, check
directly:

Run:
`rg -n '```ignore' /home/mdorman/src/jaunder/.claude/worktrees/issue-763-doctest-gate/xtask /home/mdorman/src/jaunder/.claude/worktrees/issue-763-doctest-gate/tools || true`
Expected: no matches.

Also run:
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-763-doctest-gate -- cargo test --workspace --doc`
Expected: PASS, `macros` now reports **0 ignored** and one more passing test.

- [ ] **Step 5: Commit**

```bash
git add xtask/src/steps/proffered_filename_check.rs macros/src/lib.rs web/src/reactive/scope.rs
git commit -m "docs(doctests): retire every ignore fence — text, or a real example (#763)"
```

---

### Task 8: Companions for `token.rs`, `etag.rs`, `post_body.rs`

**Files:**

- Modify: `common/src/token.rs` (doc comment @38, 4 blocks at `:56/:59/:64/:69`)
- Modify: `common/src/etag.rs` (doc comment @23, 2 blocks at `:35/:38`)
- Modify: `common/src/post_body.rs` (doc comment @3, 2 blocks at `:15/:19`)

**Interfaces:**

- Consumes: the gate from Task 5.
- Produces: 3 doc comments with matched hidden preludes; no API change.

- [ ] **Step 1: Pin the fixture values before writing anything**

Companions **execute**, unlike the `compile_fail` blocks they are derived from,
so every constructor call must succeed or the doctest run fails on a panic. Read
each type's validator and pick a literal it accepts:

- `RawToken`/`TokenHash`: `from_str("abc")` is valid — `validate_shape` accepts
  the base64url charset (`common/src/token.rs:27-36`). ✓
- `ETag`: `from_str` requires a double-quoted value
  (`common/src/etag.rs:50-59`), so use `"\"abc\""`.
- `PostBody`: constructed via `From<String>`, **not** `FromStr`, unlike the
  others.
- `RenderedHtml` (needed by `post_body`'s companion): its only doors are
  `RenderedHtml::from_trusted` and the `sanitize`-gated constructor
  (`common/src/render.rs:293`). Use `from_trusted`; the
  `rendered-html-from-trusted` gate scans code via `syn`, not doc-comment text,
  so it will not fire on a doctest.

Run each candidate through `cargo test --workspace --doc` before moving on.

- [ ] **Step 2: Restructure each doc comment to the macros pattern**

For each: add one plain companion fence, then rewrite every `compile_fail` in
the same doc comment so its prelude is `#`-hidden and every hidden line also
appears in the companion. Follow `macros/src/lib.rs:69-139` verbatim in shape.

`common/src/token.rs` — the companion proves both paths resolve and `FromStr` is
in scope, so the four negatives fail for the stated reason rather than a moved
path:

````rust
/// The positive companion shows the identical fixture compiles — both paths resolve
/// and `FromStr` is in scope — so each `compile_fail` below fails for the missing
/// conversion, not an unresolved name. (Fixture lines are hidden with `#`.)
///
/// ```
/// use common::token::{RawToken, TokenHash};
/// use std::str::FromStr;
/// let raw = RawToken::from_str("abc").unwrap();
/// let hash = TokenHash::from_str("abc").unwrap();
/// let _read: &str = raw.as_ref();
/// ```
///
/// No public constructor:
/// ```compile_fail
/// # use common::token::{RawToken, TokenHash};
/// # use std::str::FromStr;
/// let _ = RawToken("abc".to_string()); // private field
/// ```
///
/// No `RawToken` -> `TokenHash` conversion:
/// ```compile_fail
/// # use common::token::{RawToken, TokenHash};
/// # use std::str::FromStr;
/// # let raw = RawToken::from_str("abc").unwrap();
/// let _h: TokenHash = raw.into();
/// ```
///
/// No reverse conversion:
/// ```compile_fail
/// # use common::token::{RawToken, TokenHash};
/// # use std::str::FromStr;
/// # let hash = TokenHash::from_str("abc").unwrap();
/// let _r: RawToken = hash.into();
/// ```
///
/// No cross-type `PartialEq`:
/// ```compile_fail
/// # use common::token::{RawToken, TokenHash};
/// # use std::str::FromStr;
/// # let raw = RawToken::from_str("abc").unwrap();
/// # let hash = TokenHash::from_str("abc").unwrap();
/// let _ = raw == hash;
/// ```
````

`common/src/etag.rs` — same shape, companion
`ETag::from_str("\"abc\"").unwrap()` then `let _read: &str = e.as_ref();`.

`common/src/post_body.rs` — companion constructs both sides of the pair it
separates:

````rust
/// ```
/// use common::post_body::PostBody;
/// use common::render::RenderedHtml;
/// let body = PostBody::from("hello".to_string());
/// let html = RenderedHtml::from_trusted("<p>hello</p>".to_string());
/// let _b: &str = body.as_ref();
/// let _h: &str = html.as_ref();
/// ```
````

with both negatives hiding that prelude.

- [ ] **Step 3: Verify**

Run:
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-763-doctest-gate -- cargo test --workspace --doc`
Expected: PASS — `common` gains 3 tests; every `compile_fail` still passes.

Run the Task 5 command. Expected: no `missing-companion` for `token.rs`,
`etag.rs`, `post_body.rs`.

- [ ] **Step 4: Commit**

```bash
git add common/src/token.rs common/src/etag.rs common/src/post_body.rs
git commit -m "docs(common): positive companions for the token, etag, and post-body proofs (#763)"
```

---

### Task 9: Companions for `media.rs`

**Files:**

- Modify: `common/src/media.rs` — doc comments @67 (`ContentHash`, `:83/:86`),
  @134 (`Filename`, `:183/:186`), @336 (`ProfferedFilename`, `:369/:373`), @750
  (`ContentType`, `:761/:764`)

**Interfaces:**

- Consumes: the gate from Task 5; the pattern from Task 8.
- Produces: 4 doc comments with matched hidden preludes; no API change.

- [ ] **Step 1: Pin the fixture values**

Same hazard as Task 8, and `ContentHash` is the sharpest case: `from_str`
requires 64 lowercase hex digits (`common/src/media.rs:98-108`,
`is_valid_content_hash`), so the existing `"abc"` literal would panic in a
companion. Use
`"0000000000000000000000000000000000000000000000000000000000000000"`.

Read `Filename`, `ContentType`, and `ProfferedFilename`'s validators and their
unit tests in this file's `mod tests`, and take a literal each already accepts.

- [ ] **Step 2: Add a companion to each of the four doc comments**

Three of the four are the recurring private-field / wrong-type pair:

````rust
/// ```
/// use common::media::ContentHash;
/// use std::str::FromStr;
/// let h = ContentHash::from_str(
///     "0000000000000000000000000000000000000000000000000000000000000000",
/// ).unwrap();
/// let _read: &str = h.as_ref();
/// ```
///
/// No public constructor:
/// ```compile_fail
/// # use common::media::ContentHash;
/// # use std::str::FromStr;
/// let _ = ContentHash("abc".to_string()); // private field
/// ```
///
/// A `String` is not a `ContentHash`:
/// ```compile_fail
/// # use common::media::ContentHash;
/// # use std::str::FromStr;
/// fn takes(_: ContentHash) {}
/// takes("abc".to_string());
/// ```
````

`ProfferedFilename`@336 differs: its negatives assert the _absence_ of `Display`
and `Serialize` (the structural half of ADR-0084), so — exactly as
`macros/src/lib.rs:69-79` does — its companion must prove `serde_json` resolves,
or the `Serialize` negative could pass on an unresolved crate. `serde_json` is
already a normal dependency of `common`, so no manifest change is needed:

````rust
/// ```
/// # use common::media::ProfferedFilename;
/// # use std::str::FromStr;
/// let p = ProfferedFilename::from_str("a.png").unwrap();
/// let _read: &str = p.as_ref();
/// let _ = serde_json::to_string(p.as_ref()); // serde_json resolves (a &str serializes)
/// ```
````

- [ ] **Step 3: Verify**

Run:
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-763-doctest-gate -- cargo test --workspace --doc`
Expected: PASS — `common` gains 4 tests.

Run the Task 5 command. Expected: no `missing-companion` for `media.rs`.

- [ ] **Step 4: Commit**

```bash
git add common/src/media.rs
git commit -m "docs(common): positive companions for the media newtype proofs (#763)"
```

---

### Task 10: Companions for `render.rs`, and repair the partial one

**Files:**

- Modify: `common/src/render.rs` — doc comment @47 (`RenderedHtml`, `:72/:75`)
  and @496 (`RenderOutput`, `:511/:518`, plus the existing plain fence at
  `:505`)

**Interfaces:**

- Consumes: the gate from Task 5; the pattern from Tasks 8–9.
- Produces: 2 doc comments with matched hidden preludes; no API change.

- [ ] **Step 1: Confirm the sanitize-gated pair is actually in the run**

`:511`/`:518` sit inside `#[cfg(feature = "sanitize")] mod sanitized`
(`common/src/render.rs:223`) and appear only because `--workspace` unification
enables the feature via `storage/Cargo.toml:12`. Confirm the Task 5 baseline
lists `missing-companion` for both. If they are **absent**, the invocation
regressed to a package-scoped one — fix that before continuing.

- [ ] **Step 2: Add a companion to `RenderedHtml`@47**

Same shape as Task 8, using `RenderedHtml::from_trusted` as the door.

- [ ] **Step 3: Repair the `RenderOutput` companion at `:505`**

The existing plain fence imports `{PostFormat, RenderOutput}` while the
negatives import `{render, PostFormat, RenderOutput}` — so an unresolved
`render` would leave both negatives passing vacuously. Bring the import sets
into line and hide the prelude in the negatives.

The real API (verified):
`render(body: &PostBody, format: &PostFormat) -> RenderedHtml` (`render.rs:309`,
re-exported `:576`); `RenderOutput::render(&PostBody, &PostFormat) -> Self`
(`:541`); `html(&self) -> &RenderedHtml` (`:549`); `media(&self) -> &[MediaRef]`
(`:556`). Every argument is by reference and the free `render` returns
`RenderedHtml`, not `RenderOutput` — so the companion must exercise both to
justify importing both:

````rust
/// ```
/// use common::post_body::PostBody;
/// use common::render::{render, PostFormat, RenderOutput};
/// let body = PostBody::from("hello".to_string());
/// let out = RenderOutput::render(&body, &PostFormat::Markdown);
/// assert!(out.media().is_empty());
/// let _direct = render(&body, &PostFormat::Markdown); // `render` resolves
/// ```
///
/// No struct-literal construction:
/// ```compile_fail
/// # use common::post_body::PostBody;
/// # use common::render::{render, PostFormat, RenderOutput};
/// # let body = PostBody::from("hello".to_string());
/// # let out = RenderOutput::render(&body, &PostFormat::Markdown);
/// let _ = RenderOutput { html: out.html().clone(), media: vec![] };
/// ```
///
/// No field reassignment:
/// ```compile_fail
/// # use common::post_body::PostBody;
/// # use common::render::{render, PostFormat, RenderOutput};
/// # let body = PostBody::from("hello".to_string());
/// # let mut out = RenderOutput::render(&body, &PostFormat::Markdown);
/// out.html = out.html().clone();
/// ```
````

- [ ] **Step 4: Verify**

Run:
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-763-doctest-gate -- cargo test --workspace --doc`
Expected: PASS — `common` gains 1 test (`RenderedHtml`'s companion;
`RenderOutput`'s already existed).

Run the Task 5 command. Expected: `common/` is clean; the only remaining
violations are the six in `macros/src/lib.rs` (Tasks 11 and 12).

- [ ] **Step 5: Commit**

```bash
git add common/src/render.rs
git commit -m "docs(common): companion for RenderedHtml, repair the RenderOutput one (#763)"
```

---

### Task 11: Hidden preludes for `macros` `:43`, `:220`, `:274`

**Files:**

- Modify: `macros/src/lib.rs:43-47`, `:220-224`, `:274-279`

**Interfaces:**

- Consumes: the gate from Task 5.
- Produces: three matched doc comments. Three one-line edits.

**Why these exist:** each is a "the derive rejects a non-tuple struct" proof
whose fixture is entirely visible, so it carries no hidden prelude and cannot be
matched. Each already sits in a doc comment whose companion uses the identical
import line **visibly** — `:51` has `use macros::StrNewtype;`, `:228` has
`use macros::IdNewtype;`, `:257` has `use macros::NumNewtype;` — so hiding that
one line in the negative both satisfies the rule and makes the proof honest: the
negative now demonstrably fails for the named-field struct, not for an
unresolved `macros` import.

- [ ] **Step 1: Hide the import line in each negative**

`macros/src/lib.rs:43-47` becomes:

````rust
/// ```compile_fail
/// # use macros::StrNewtype;
/// #[derive(StrNewtype)]
/// struct NotATuple { s: String }
/// ```
````

`:220-224` and `:274-279` likewise, hiding `use macros::IdNewtype;` and
`use macros::NumNewtype;` respectively.

- [ ] **Step 2: Verify**

Run:
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-763-doctest-gate -- cargo test --workspace --doc`
Expected: PASS, same test count as before (hiding a line does not change
collection).

Run the Task 5 command. Expected: the only remaining violations are
`macros/src/lib.rs:145`, `:158`, `:173`.

- [ ] **Step 3: Commit**

```bash
git add macros/src/lib.rs
git commit -m "docs(macros): hide the import prelude in the non-tuple-struct proofs (#763)"
```

---

### Task 12: The `Borrow<str>` proof and the discriminating ordering proofs

**Files:**

- Modify: `macros/src/lib.rs:69-79` (companion), after `:139` (new negative),
  `:145`, `:158`, `:173` (made discriminating, each with a companion), plus a
  new control fence; and `macros/tests/str_newtype.rs:239-243` (the reciprocal
  claim)

**Interfaces:**

- Consumes: the gate from Task 5.
- Produces: a tree with **zero** violations — the precondition for Tasks 13–15.

- [ ] **Step 1: Add `Borrow` to the positive companion**

`macros/src/lib.rs:69-79` gains `# use std::borrow::Borrow;` to its hidden
prelude and a visible line exercising it on a `&str`, so the new negative cannot
pass on an unresolved import:

````rust
/// ```
/// # use macros::StrNewtype;
/// # use std::str::FromStr;
/// # use std::borrow::Borrow;
/// # #[derive(Clone, StrNewtype)]
/// # #[str_newtype(secret)]
/// # struct Sec(String);
/// # impl FromStr for Sec { type Err = std::convert::Infallible; fn from_str(s: &str) -> Result<Self, Self::Err> { Ok(Sec(s.to_owned())) } }
/// let s = Sec("x".to_owned());
/// let _read: &str = s.as_ref();               // explicit borrowed read is allowed
/// let _ = serde_json::to_string(s.as_ref());  // serde_json resolves (a &str serializes)
/// let _b: &str = Borrow::borrow(s.as_ref());  // Borrow resolves on a &str
/// ```
````

- [ ] **Step 2: Add the `Borrow<str>` negative**

ADR-0063:140 names `Deref<str>` **and** `Borrow<str>` as the secret's omissions;
only `Deref` had a proof. Add, immediately after the `Deref` block at
`:130-139`:

````rust
/// No `Borrow<str>` — the other omission ADR-0063 names:
/// ```compile_fail
/// # use macros::StrNewtype;
/// # use std::str::FromStr;
/// # use std::borrow::Borrow;
/// # #[derive(Clone, StrNewtype)]
/// # #[str_newtype(secret)]
/// # struct Sec(String);
/// # impl FromStr for Sec { type Err = std::convert::Infallible; fn from_str(s: &str) -> Result<Self, Self::Err> { Ok(Sec(s.to_owned())) } }
/// # let s = Sec("x".to_owned());
/// let _: &str = Borrow::borrow(&s);
/// ```
````

- [ ] **Step 3: Verify the new negative actually discriminates**

Hand-write `impl std::borrow::Borrow<str> for Sec` into a scratch crate carrying
the same fixture and confirm the block then **compiles** — i.e. that it would
stop being a `compile_fail`. Revert. A negative that would pass either way is
exactly the vacuity this cycle exists to eliminate; do not skip this step.

- [ ] **Step 4: Make the three ordering proofs discriminate**

`macros/src/lib.rs:145`, `:158` (governed by the prose at `:141-144`) and `:173`
today prove nothing: their fixtures derive no `PartialEq`, so `a < b` fails to
compile whether or not the macro emits ordering. The prose admits it and cites a
unit test as "the actual guard" — while `macros/tests/str_newtype.rs:242` cites
the doctest. **The two claims are circular and neither guards anything.**

Give each fixture `PartialEq, Eq` and the negatives become real: `a < b` can
then only fail for the missing `PartialOrd`. Probed on this branch — the control
(same shape, no suppressing option) orders, and all three negatives fail.

Two constraints the probe established:

- A `secret` emits its own redacting `Debug`, so a secret fixture must **not**
  derive `Debug` — E0119, conflicting implementations.
- The existing `:118` proof ("no value `PartialEq`", `s == "x"`) still
  discriminates, because a derived `PartialEq` gives `Sec == Sec`, not
  `Sec == &str`.

Concretely:

- Change the shared secret fixture to
  `#[derive(Clone, PartialEq, Eq, StrNewtype)]` **uniformly** across doc@16 —
  the companion at `:69` and every hidden prelude that repeats it (`:82`, `:94`,
  `:106`, `:118`, `:130`, the new `Borrow` block, and `:145`). A prelude that
  still says `#[derive(Clone, StrNewtype)]` no longer matches the companion and
  the gate will say so.
- Add `let a` / `let b` bindings to the `:69` companion so `:145`'s prelude is
  fully matched.
- Give `:158` (`secret, serde`) and `:173` (`no_ord`) their own companions, each
  exercising the fixture legally (e.g. `assert!(a == a)`).
- Add a **control** fence in the same doc comment asserting that the
  un-suppressed shape _does_ order, so a reader can see the negatives
  discriminate. This mirrors `testdata/ordering_control.rs` from Task 6.

Rewrite both halves of the circular claim: the `:141-144` prose (it no longer
"documents intent rather than discriminates") and the comment at
`macros/tests/str_newtype.rs:239-243` (the doctest now genuinely locks the
absence of ordering, which is what that comment always claimed). Note in prose
that the fixture derives `PartialEq`/`Eq` to make the proof discriminate, while
the live `no_ord` consumer `RawToken` deliberately does not.

- [ ] **Step 5: Verify**

Run:
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-763-doctest-gate -- cargo test --workspace --doc`
Expected: PASS — `macros` gains the `Borrow` negative, three companions, and one
control fence. Confirm the three ordering blocks still report
`compile fail ... ok`: if any now _passes as a plain test_, the fixture lost its
suppression and the proof is inverted.

Run the Task 5 command. Expected: **zero violations of any kind.** This is the
point at which the tree is clean and the gate may be wired in. Do not start Task
13 until this holds.

- [ ] **Step 6: Commit**

```bash
git add macros/src/lib.rs
git commit -m "test(macros): prove the secret surface omits Borrow<str> (#763)"
```

---

### Task 13: The `doctests` and `doctests-gate` derivations

**Files:**

- Modify: `flake.nix` — two new checks beside `coverage` (`:1115-1220`); correct
  the stale comment at `:315-318`

**Interfaces:**

- Consumes: `devtool doctests emit --out` (Task 5).
- Produces: `checks.<system>.doctests` and `checks.<system>.doctests-gate` (Task
  14).

- [ ] **Step 1: Add the producer derivation**

Mirror `coverage` (`flake.nix:1115-1199`) but far lighter — no PostgreSQL, no
llvm-cov, no CSR bundle. Keep the `LD_LIBRARY_PATH` export: the `common` doctest
binaries link with `sqlx` and `sanitize` enabled (feature unification via
`storage`), so openssl is on the runtime path exactly as it is for the coverage
run (`flake.nix:1182`).

```nix
doctests = craneLib.mkCargoDerivation (
  commonArgs
  // {
    inherit cargoArtifacts;
    pname = "jaunder-doctests";
    # `--doc` runs OUTSIDE any llvm-cov instrumentation, so no profraw from these
    # tests reaches the coverage profile. Doctests deliberately do not feed the
    # ADR-0050 coverage gate: `llvm-cov --doctests` is unstable.
    nativeBuildInputs = commonArgs.nativeBuildInputs ++ [ devtoolBin ];
    buildPhaseCargoCommand = ''
      export LD_LIBRARY_PATH="${pkgs.lib.makeLibraryPath [ pkgs.openssl ]}:''${LD_LIBRARY_PATH:-}"
      mkdir -p emit-out
      # devtool always exits 0 after writing emit-out/status.json; gating is the
      # doctests-gate consumer derivation + host xtask.
      devtool doctests emit --out emit-out
    '';
    installPhaseCommand = ''
      mkdir -p $out
      cp emit-out/status.json $out/status.json
      cp -r emit-out/diagnostics $out/diagnostics
    '';
  }
);
```

- [ ] **Step 2: Add the gate consumer**

Mirror `coverage-gate` (`flake.nix:1205-1220`):

```nix
doctests-gate =
  pkgs.runCommand "jaunder-doctests-gate"
    {
      nativeBuildInputs = [ pkgs.jq ];
    }
    ''
      cat ${self.checks.${system}.doctests}/status.json
      cat=$(jq -r .category ${self.checks.${system}.doctests}/status.json)
      if [ "$cat" != "ok" ]; then
        echo "doctest gate failed: category=$cat" >&2
        jq -r '.infra_detail // (.violations[] | "\(.file):\(.line) [\(.kind)] \(.detail)")' \
          ${self.checks.${system}.doctests}/status.json >&2
        exit 1
      fi
      touch $out
    '';
```

- [ ] **Step 3: Correct the stale comment at `flake.nix:315-318`**

It reads "Tests are covered by the separate `nextest` check". There is no
`nextest` check; the suite runs inside `coverage`. Reword to name `coverage`,
and note that doctests are covered by the new `doctests` check — nextest
structurally cannot run them, which is why they need their own. (Authorized by
this plan, not the spec.)

- [ ] **Step 4: Verify both derivations build**

Run:
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-763-doctest-gate -- nix build -L --accept-flake-config .#checks.x86_64-linux.doctests-gate`
Expected: exit 0. (Long/cold — use Bash background mode.)

Then break one proof (make a `compile_fail` compile), rebuild, and confirm the
gate fails with that fence named on stderr in `file:line [kind] detail` form.
Revert.

- [ ] **Step 5: Commit**

```bash
git add flake.nix
git commit -m "build(nix): doctests producer and gate check derivations (#763)"
```

---

### Task 14: xtask wiring

**Files:**

- Modify: `xtask/src/steps/nix.rs` (new `doctests` + `doctest_sentinel_detail`),
  `xtask/src/lib.rs:91-99`, `:414-416`, `:454`
- Modify: `xtask/Cargo.toml`, `xtask/Cargo.lock`

**Interfaces:**

- Consumes: `checks.<system>.doctests{,-gate}`;
  `doctests::status::DoctestStatus`.
- Produces: `nix-doctests` and `nix-doctests-gate` steps.

- [ ] **Step 1: Write the failing tests** in `xtask/src/steps/nix.rs`'s test
      module (`:466`)

```rust
    #[test]
    fn doctest_sentinel_detail_names_each_violation() {
        use doctests::check::{Kind, Violation};
        use doctests::status::{DoctestStatus, StatusCategory};
        let s = DoctestStatus {
            category: StatusCategory::Violations,
            violations: vec![Violation {
                file: "common/src/token.rs".to_string(),
                line: 56,
                kind: Kind::NotRun,
                detail: "scanned but never evaluated".to_string(),
            }],
            infra_detail: None,
        };
        let d = doctest_sentinel_detail(&s);
        assert!(d.contains("common/src/token.rs:56"), "{d}");
        // The kebab-case spelling, matching the gate derivation's jq output.
        assert!(d.contains("not-run"), "{d}");
    }

    #[test]
    fn doctest_sentinel_detail_is_terse_when_ok() {
        use doctests::status::{DoctestStatus, StatusCategory};
        let s = DoctestStatus {
            category: StatusCategory::Ok,
            violations: vec![],
            infra_detail: None,
        };
        assert_eq!(doctest_sentinel_detail(&s), "in-sandbox: doctests ok");
    }
```

- [ ] **Step 2: Run the tests, verify they fail**

Run:
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-763-doctest-gate -- cargo test --manifest-path xtask/Cargo.toml`
Expected: FAIL — `doctest_sentinel_detail` not defined.

- [ ] **Step 3: Implement against the tests**

Add `doctests = { path = "../tools/doctests" }` to `xtask/Cargo.toml`, alongside
the existing `coverage = { path = "../tools/coverage" }` (`:18`).

In `xtask/src/steps/nix.rs`, mirroring `coverage()` (`:19-45`) and
`sentinel_detail` (`:47-63`):

```rust
/// The Nix doctest check: the producer runs the workspace doctests and reconciles
/// them against the scanned fence population; the consumer fails on a non-ok
/// sentinel. Doctests are the one suite nextest structurally cannot run.
pub fn doctests(result: &mut CommandResult) {
    result.push(build_check("nix-doctests", "doctests"));
    let gate = build_check("nix-doctests-gate", "doctests-gate");
    if !gate.ok {
        let status_path = ".xtask/gcroots/doctests/status.json";
        let detail = std::fs::read_to_string(status_path)
            .ok()
            .and_then(|s| doctests::status::DoctestStatus::from_json(&s).ok())
            .map(|s| doctest_sentinel_detail(&s))
            .unwrap_or_else(|| "doctest gate failed (no status.json)".to_string());
        result.push(StepResult::fail("doctests").detail(detail));
        return;
    }
    result.push(gate);
}

/// Render the in-sandbox doctest sentinel into a human detail. Pure + tested.
///
/// Each violation renders as `file:line [kind] detail`, with `kind` serde-rendered
/// (kebab-case) rather than `Debug`-printed, so the host message and the gate
/// derivation's jq output read identically.
fn doctest_sentinel_detail(status: &doctests::status::DoctestStatus) -> String;
```

- [ ] **Step 4: Wire the call sites**

`xtask/src/lib.rs`, immediately after each `steps::nix::coverage(&mut result);`:

- `Check` arm (`:414-416`): inside the same `if !no_test { … }` block.
- `Validate` arm (`:454`): unconditionally.

Update the `--no-test` doc at `:91-99` to say it skips the Nix coverage **and
doctest** checks, and that the host-side fence step (Task 15) still runs — the
asymmetry AC5 requires be stated.

- [ ] **Step 5: Run and verify**

Run:
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-763-doctest-gate -- cargo test --manifest-path xtask/Cargo.toml`
Expected: PASS.

Run:
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-763-doctest-gate -- cargo xtask check`
Expected: PASS, with `nix-doctests` and `nix-doctests-gate` present. Confirm
with `jq -r '.steps[].name' .xtask/last-result.json`.

Run:
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-763-doctest-gate -- cargo xtask check --no-test`
Expected: PASS, with **neither** step present.

- [ ] **Step 6: Commit**

```bash
git add xtask/src/steps/nix.rs xtask/src/lib.rs xtask/Cargo.toml xtask/Cargo.lock
git commit -m "build(xtask): run the doctest gate in check and validate (#763)"
```

---

### Task 15: Host-side `xtask/` + `tools/` step, and the scan-root test

**Files:**

- Create: `xtask/src/steps/doctest_fences.rs`
- Modify: `xtask/src/lib.rs:23-45` (module list), `Check` and `Validate` arms
- Modify: `xtask/src/git.rs` (add `tracked_files`)

**Interfaces:**

- Consumes: `doctests::{roots::{HOST, ALL}, check::{ScannedFile, problems}}`,
  `crate::files::with_extension`, `crate::git::tracked_files`.
- Produces: a `doctest-fences` step covering the two roots no Nix check can see.

- [ ] **Step 1: Write the failing tests** in `xtask/src/steps/doctest_fences.rs`

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_rs_file_in_the_repo_falls_under_exactly_one_scan_root() {
        // AC9. A file under no root is invisible to the gate; a file under two
        // would be reconciled against the wrong run. Either is a population bug.
        // `git ls-files` rather than a walk, so `target/` and `.claude/worktrees/`
        // can never be recursed into.
        let tracked = crate::git::tracked_files(Path::new("."), "*.rs").expect("git ls-files");
        for path in tracked {
            let n = doctests::roots::ALL
                .iter()
                .filter(|r| path.starts_with(&format!("{r}/")))
                .count();
            assert_eq!(n, 1, "{path} falls under {n} scan roots, want exactly 1");
        }
    }

    #[test]
    fn run_paths_are_relative_to_the_invoked_manifest() {
        // `cargo test --manifest-path xtask/Cargo.toml --doc` prints `src/…`.
        assert_eq!(run_path("xtask", "xtask/src/steps/nix.rs"), "src/steps/nix.rs");
        assert_eq!(run_path("tools", "tools/devtool/src/main.rs"), "devtool/src/main.rs");
    }
}
```

- [ ] **Step 2: Run the tests, verify they fail**

Run:
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-763-doctest-gate -- cargo test --manifest-path xtask/Cargo.toml`
Expected: FAIL — `run_path`, `git::tracked_files` not defined.

- [ ] **Step 3: Implement against the tests**

Add to `xtask/src/git.rs`, matching the module's `(dir: &Path, …)` convention
(`git.rs:37-82`):

```rust
/// Tracked files matching `glob`, repo-relative. `git ls-files` rather than a
/// filesystem walk, so `target/` and nested worktrees are never enumerated.
pub fn tracked_files(dir: &Path, glob: &str) -> anyhow::Result<Vec<String>>;
```

`xtask/src/steps/doctest_fences.rs`:

```rust
//! The `doctest-fences` gate: the half of the doctest population that lives
//! outside every Nix check.
//!
//! `xtask/` is excluded from the flake `src` filter (`flake.nix:272`) and `tools/`
//! is a separate virtual workspace, so the `doctests` derivation's
//! `cargo test --workspace --doc` reaches neither. This step runs each one's
//! doctests directly and reconciles them against the same scanner.
//!
//! `host_tests`'s `cargo test --manifest-path …` also *executes* these doctests,
//! but discards the output; this step is the only thing that *reconciles* them.
//! Like `host_tests`, it runs in every mode — `--no-test` skips only the Nix half.
//!
//! The scan roots live in `doctests::roots`, shared with the producer, so the
//! population this step asserts over cannot drift from the one `devtool` scans.

/// A repo-relative path as the runner prints it for a `--manifest-path <root>` run.
fn run_path(root: &str, path: &str) -> String;

/// Scan and reconcile both host roots, pushing one `doctest-fences` step.
pub fn run(result: &mut CommandResult);
```

`run` walks each `doctests::roots::HOST` entry with `files::with_extension`
(which returns `Vec<PathBuf>` — convert with `.display().to_string()`), runs
`cargo test --manifest-path <root>/Cargo.toml --doc` capturing combined output,
and calls `problems`. Unreadable files fail rather than shrink the population,
following `sqlx_newtype_decode_check.rs:569-583`.

- [ ] **Step 4: Wire the call sites**

`xtask/src/lib.rs`: add `pub mod doctest_fences;` to the `steps` module list
(`:23-45`), and call `steps::doctest_fences::run(&mut result);` next to
`steps::sqlx_newtype_decode_check::run(&mut result);` in **both** the `Check`
(`:411`) and `Validate` (`:451`) arms, outside any `no_test` guard.

- [ ] **Step 5: Run and verify**

Run:
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-763-doctest-gate -- cargo test --manifest-path xtask/Cargo.toml`
Expected: PASS.

Run:
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-763-doctest-gate -- cargo xtask check --no-test`
Expected: PASS, with `doctest-fences` present (it runs in every mode).

- [ ] **Step 6: Commit**

```bash
git add xtask/src/steps/doctest_fences.rs xtask/src/lib.rs xtask/src/git.rs
git commit -m "build(xtask): reconcile the xtask and tools fence population (#763)"
```

---

### Task 16: ADR draft and module docs

**Files:**

- Create: `docs/adr/drafts/doctest-gate-enumerates-the-fence-population.md`
- Modify: `tools/doctests/src/lib.rs`, `tools/doctests/src/check.rs` (module
  docs)

**Interfaces:**

- Consumes: everything above.
- Produces: the numberless draft `cargo xtask adr promote` numbers at ship.

- [ ] **Step 1: Write the ADR draft**

Follow the **jaunder-adr** skill's draft flow (numberless, in
`docs/adr/drafts/`). Content, per AC18:

- **Context:** 31 `compile_fail` proofs cited by ADR-0063, ADR-0084 and
  ADR-0085, none evaluated; and the five ways a run's population silently
  shrinks, with the `sanitize` case worked through — including that the issue's
  own "18 passed" evidence _was_ the shrunk number.
- **Decision:** decisions 1–11 from the spec.
- **Conformance to ADR-0085:** population defined structurally (every fence
  `syn` can read under a scan root); deny-by-default vocabulary; exemptions
  written by a human at the site; parses rather than scans; fails on input it
  cannot read.
- **Honesty obligation** — state, per ADR-0085's closing requirement:
  - the classes the gate can _see but never run_, which it forces to `text`
    rather than skipping: fences in crates with no lib target (`tools/devtool`
    today), and fences under `#[cfg]` combinations no scan root's run enables
    (wasm-only modules, e.g. `web/src/reactive/scope.rs`);
  - that fences inside multi-line `#[doc = "…"]` values are rejected, not
    supported, because the reconciliation key cannot address them;
  - that the `text` population is uncounted, so principle 4's multiplicity
    clause is **not** enforced for them — a deliberate trade, since every such
    change is a reviewable diff hunk.
- **Consequences:** every new `compile_fail` now costs a companion; `ignore` is
  no longer available anywhere in the tree.

- [ ] **Step 2: Write the module docs**

`tools/doctests/src/lib.rs` gets the crate-level doc: what the gate governs, the
three accepted fence forms, and the two halves (workspace producer, host step).

`tools/doctests/src/check.rs` gets the rule statement — the closed vocabulary,
the hidden-prelude companion rule, the bidirectional reconciliation, each with
its one-line reason — **and**, per AC19, why doctests do not feed the coverage
gate (`llvm-cov --doctests` is unstable; ADR-0050's stateless gate measures
nextest only; the `--doc` run stays outside instrumentation). AC19 names the
scanner module specifically, so this is the one that must carry it. Follow the
`sqlx_newtype_decode_check.rs:1-80` precedent.

- [ ] **Step 3: Verify**

Run:
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-763-doctest-gate -- cargo xtask check`
Expected: PASS — including `adr-check`, which validates draft structure.

- [ ] **Step 4: Commit**

```bash
git add docs/adr/drafts tools/doctests/src/lib.rs tools/doctests/src/check.rs
git commit -m "docs(adr): draft the doctest gate decision and its ADR-0085 conformance (#763)"
```

---

### Task 17: Full validate and the coverage-unchanged proof

**Files:** none modified — this task is verification.

**Interfaces:**

- Consumes: everything above.
- Produces: the evidence AC20 requires, carried into the PR body at ship.

- [ ] **Step 1: Capture the baseline coverage verdict**

In a scratch worktree at `wt-base-issue-763` (`2ff62da1`), run
`devtool run -- cargo xtask validate --no-e2e`, then record
`jq '.coverage' .xtask/last-result.json`.

- [ ] **Step 2: Run the full local gate on the branch**

Run:
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-763-doctest-gate -- cargo xtask validate`
Expected: exit 0. (Long — use Bash background mode.)

- [ ] **Step 3: Compare the coverage verdict**

Run: `jq '.coverage' .xtask/last-result.json` and diff against Step 1. Expected:
the same verdict and the same executable-line count. A difference means doctest
profraw reached the coverage profile, contradicting decision 11 — stop and
diagnose before shipping.

- [ ] **Step 4: Confirm the full step list**

Run: `jq -r '.steps[] | "\(.name) \(.ok)"' .xtask/last-result.json` Expected:
`nix-doctests`, `nix-doctests-gate`, and `doctest-fences` all present and ok.

- [ ] **Step 5: Record the evidence for the PR body**

Capture, for **jaunder-ship**: the before/after coverage verdict; the fence
census (0 `ignore`, 0 violations); and the before/after doctest counts —

| crate    | before               | after                | why                                                                                            |
| -------- | -------------------- | -------------------- | ---------------------------------------------------------------------------------------------- |
| `common` | 21 passed            | 29 passed            | +8 companions (3 Task 8, 4 Task 9, 1 Task 10)                                                  |
| `macros` | 17 passed, 2 ignored | 23 passed, 0 ignored | `:298`→`text`, `:353`→real (+1), `Borrow` negative (+1), 3 ordering companions, 1 control (+4) |

Confirm the totals against the run rather than trusting this table — a mismatch
means a companion was folded into an existing fence instead of added.
