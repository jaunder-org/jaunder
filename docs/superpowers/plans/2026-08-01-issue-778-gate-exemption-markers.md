# Gate Exemption Markers Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with jaunder-iterate
> (delegating individual tasks to a subagent via jaunder-dispatch when useful).
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the three XSS gates' central fn-keyed allowlists with
in-source per-site marker comments, deleting the multiplicity machinery and the
qualifier-pattern exemption.

**Architecture:** A shared marker primitive (`xtask/src/markers.rs`) answers "is
there a `<token>` marker on this source line, and what reason does it give" for
both the coverage gate and the ident gates. `ident_gate` gains a classifier that
maps each `Mention` to marked / unexempt / orphan; `Gate` loses its `allowlist`
field and derives its census from the scan. Each gate keeps only its population,
roots and prose.

**Tech Stack:** Rust, `syn` / `proc-macro2`, `cargo nextest`, `cargo xtask`.

**Spec:**
[`docs/superpowers/specs/2026-08-01-issue-778-gate-exemption-markers.md`](../specs/2026-08-01-issue-778-gate-exemption-markers.md)
— the "what" and "why". This plan is the "how"; it does not restate the spec's
analysis.

## Review header

**Scope in:** `xtask/src/markers.rs` (new), `xtask/src/coverage/report.rs`,
`xtask/src/steps/ident_gate.rs`, the three gate modules, twelve marked source
sites, five doc corrections, one ADR draft (already written, ships via
`promote`).

**Scope out:** renaming `ContentType::from_trusted` (Task 1 files it);
`sqlx-newtype-decode`; coverage's `cov:ignore` vocabulary.

**Tasks and commits:**

| Task | Work                                                                                | Commit           |
| ---- | ----------------------------------------------------------------------------------- | ---------------- |
| 1    | File the `ContentType::from_trusted` rename issue                                   | —                |
| 2    | Shared marker primitive + raw-string hardening; rewire coverage                     | 1                |
| 3    | Mark the twelve sites; prove the formatters preserve them                           | 2                |
| 4    | `ident_gate`: `scan` + `classify` (additive)                                        | — (folds into 3) |
| 5    | Convert `html-sink` + `raw-html-door`; delete `Allowed` / `unexempt` helper         | 3                |
| 6    | Convert `rendered-html-from-trusted`; delete `mentions` / `top_level` / `expr_path` | 4                |
| 7    | Documentation corrections                                                           | 5                |
| 8    | Full gate                                                                           | —                |

**Why those commit boundaries.** `xtask-clippy` runs
`--all-targets -- -D warnings` (`xtask/src/steps/static_checks.rs:120-127`) and
`mod steps` is private, so any item under it with no non-test caller is
`dead_code` and fails the gate. That forces three pairings:

- Task 4's `scan`/`classify` have no non-test caller until Task 5 → same commit.
- Tasks 5's two gates share the `Gate`/`Report` change, and converting only one
  leaves the other failing to compile → same commit. `Allowed`/`unexempt` die
  with the second one, so they go too.
- Task 6's conversion is what orphans `mentions`, `Mention::top_level` and
  `Population::expr_path` → same commit.

`rendered_html_from_trusted_check.rs` imports only `{self, Population}` and
never touches `Gate`, so it is **unaffected** by Task 5 — which is why Tasks 5
and 6 can be separate commits at all.

**Key risks / decisions:**

1. **Formatter comment survival — RESOLVED in Task 3, and it changed the rule.**
   Landing the markers before any gate read them (the whole point of Task 3's
   position in the order) surfaced this as a formatting diff rather than a gate
   failure. Written **trailing**, as the spec originally required, 7 of the 12
   markers were relocated: `rustfmt` pushes a comment trailing an opening `{`
   down onto the first line of the block, and `leptosfmt` moves one up or down
   depending on where it sits in a `view!` body. Deterministic and idempotent,
   but not predictable by any rule an author could hold in their head.

   Written as standalone comment lines **directly above** each site, all twelve
   stay put. That is now the canonical position; trailing is deliberately
   **not** accepted, because it is exactly the form that silently moves. Spec
   and ADR amended. Remaining residual risk: none observed — but any new site
   should still be added markers-first and re-checked.

2. **Order is load-bearing.** Markers (Task 3) precede every gate conversion.
   Deleting `EXEMPT_QUALIFIERS` before the three `ContentType` markers exist
   turns the tree red.
3. **Coverage inherits the raw-string hardening** (Task 2). It is fail-closed; a
   `cov:ignore` that stops suppressing is a newly-failing line and must be
   investigated, not re-marked.
4. **The ADR draft is gitignored** (`docs/adr/drafts/`) and is numbered by
   `cargo xtask adr promote` during `jaunder-ship`. No task here commits it.

## Global Constraints

- Every **commit** is green under
  `devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-778-allowlist-multiplicity -- cargo xtask check`.
  Tasks marked "no commit" are verified by `cargo nextest` only, because the
  tree carries dead code or does not compile until their partner task lands.
- No `Co-Authored-By` trailer on any commit.
- Stage explicitly, then commit — never `git commit -- <paths>` (see
  **jaunder-commit**).
- `xtask` unit tests are in-file `#[cfg(test)]`, matching the crate's
  convention. **`xtask` is a separate manifest, not a workspace member**, so its
  tests run as `cargo nextest run --manifest-path xtask/Cargo.toml [filter]` —
  `-p xtask` fails with "package ID specification `xtask` did not match any
  packages".
- ADR-0085's honesty obligation: any behavior a gate cannot see is stated in
  that gate's module doc.
- The marker token is always derived from the gate's own `Gate::step` — never a
  separate const.
- **No identifier in the four in-scope gate files may be spelled `unjustified`**
  (AC16 greps for it). The classifier's field is `unexempt`; its record type is
  `Unexempt`; its reason enum is `Why`.

---

### Task 1: File the separable concern

**Files:** none in this repo.

**Interfaces:** Produces nothing later tasks consume.

- [x] **Step 1: File the rename issue** — filed as
      [#790](https://github.com/jaunder-org/jaunder/issues/790). (Note: the
      installed `gh` predates issue-type support; type set via the
      `updateIssueIssueType` GraphQL mutation.)

Use **jaunder-issues**. Title:
`Rename ContentType::from_trusted to remove the from_trusted ident collision`.
Type `Task`, label `tooling`, milestone `Web: canonical Leptos CSR convergence`,
add to Jaunder Backlog (#1).

Body must state: `common/src/media.rs`'s `ContentType::from_trusted` shares a
leaf ident with `RenderedHtml::from_trusted`, so it sits inside
`rendered-html-from-trusted`'s population and costs four markers (one
definition, three call sites) that say only "this is a different door". Renaming
it (e.g. `known`, `trusted_literal`) removes the collision at the source and
makes the gate's population exact. Note that #778 deliberately left this out of
scope because it changes a `common` API for a gate's benefit.

- [x] **Step 2: Verify it landed** — #790, type `Task`, label `tooling`,
      milestone `Web: canonical Leptos CSR convergence`, added to Jaunder
      Backlog (#1).

No commit — this task touches no files.

---

### Task 2: Shared marker primitive — commit 1

**Files:**

- Create: `xtask/src/markers.rs`
- Modify: `xtask/src/lib.rs` (add `pub mod markers;` in the alphabetical block
  at `:5-22`, between `mod ids;` and `mod nix_build;`)
- Modify: `xtask/src/coverage/report.rs`
- Test: in-file `#[cfg(test)] mod tests` in `xtask/src/markers.rs`

**Interfaces:**

- Produces:
  - `pub fn line_comment(src: &str) -> Option<&str>`
  - `pub fn comment_marker_is(comment: &str, marker: &str) -> bool`
  - `pub fn marker_on_line<'a>(line: &'a str, token: &str) -> Option<&'a str>` —
    `None` when the line carries no `token` marker; `Some(reason)` with the
    trimmed remainder otherwise, which is `""` for a bare marker.

**`pub mod`, not `mod`.** `marker_on_line` has no non-test caller until Task 5.
Under private `mod markers;` that is `dead_code` and `xtask-clippy` fails this
task's own commit. `pub mod` makes it public API and immune, matching
`pub mod coverage;` / `pub mod git;` already in `lib.rs`.

- [x] **Step 1: Move the helpers and write the failing raw-string tests**

Move **three** private fns from `report.rs` into `xtask/src/markers.rs`, made
`pub` except the last: `comment_marker_is` (`:113`), `line_comment` (`:127`),
and **`char_literal_len` (`:170`)** — `line_comment` calls it at `:154` and
nothing else does, so leaving it behind breaks the new module and orphans a
private fn in the old one. Keep their doc comments. Move their existing tests
with them unchanged (they must keep passing): the `line_comment_*`,
`comment_marker_is_*` and string/char-literal tests at `report.rs:317-435`.

Add `marker_on_line` and the new tests:

```rust
#[test]
fn a_marker_inside_a_raw_string_is_not_a_comment() {
    assert_eq!(line_comment(r##"let s = r#"// html-sink:allow x"#;"##), None);
}

#[test]
fn a_marker_after_a_raw_string_is_still_found() {
    assert_eq!(
        line_comment(r##"let s = r#"a // b"#; // html-sink:allow real"##),
        Some(" html-sink:allow real")
    );
}

#[test]
fn a_hash_less_raw_string_is_honored() {
    assert_eq!(line_comment(r#"let s = r"// x"; // real"#), Some(" real"));
}

#[test]
fn an_unterminated_raw_string_swallows_the_rest_of_the_line() {
    // Fail-closed: if we cannot tell where the literal ends, we do not invent a
    // marker inside it.
    assert_eq!(line_comment(r##"let s = r#"// html-sink:allow x"##), None);
}

#[test]
fn marker_on_line_returns_the_reason() {
    assert_eq!(
        marker_on_line("code() // html-sink:allow because reasons", "html-sink:allow"),
        Some("because reasons")
    );
}

#[test]
fn marker_on_line_returns_empty_for_a_bare_marker() {
    assert_eq!(marker_on_line("code() // html-sink:allow", "html-sink:allow"), Some(""));
}

#[test]
fn marker_on_line_ignores_another_gates_token() {
    assert_eq!(marker_on_line("code() // raw-html-door:allow r", "html-sink:allow"), None);
}

#[test]
fn marker_on_line_ignores_a_prose_mention() {
    assert_eq!(
        marker_on_line("code() // see the html-sink:allow docs", "html-sink:allow"),
        None
    );
}

#[test]
fn marker_on_line_ignores_a_doc_comment() {
    assert_eq!(marker_on_line("/// html-sink:allow x", "html-sink:allow"), None);
    assert_eq!(marker_on_line("//! html-sink:allow x", "html-sink:allow"), None);
}
```

- [x] **Step 2: Run, verify it does not build**

Run:
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-778-allowlist-multiplicity -- cargo nextest run --manifest-path xtask/Cargo.toml markers`
Expected: **build failure** — `marker_on_line` not defined. (Not a test failure;
the tests reference an undefined fn.)

- [x] **Step 3: Implement**

To these signatures:

```rust
pub fn line_comment(src: &str) -> Option<&str>;
pub fn comment_marker_is(comment: &str, marker: &str) -> bool;
pub fn marker_on_line<'a>(line: &'a str, token: &str) -> Option<&'a str>;
```

`line_comment`'s existing loop tracks `in_str` for `"`-strings with escapes and
char literals via `char_literal_len`. Add raw-string state: on `r` followed by
zero or more `#` then `"`, enter raw mode recording the hash count; leave on the
first `"` followed by that many `#`; escapes do not apply inside. An
unterminated raw string consumes to end of line, yielding no comment — the
fail-closed branch. Every branch is pinned by a Step 1 test.

`marker_on_line` composes the two: take `line_comment`, require
`comment_marker_is`, return the trimmed remainder after the token.

- [x] **Step 4: Rewire coverage, run the xtask suite** — 715 tests pass,
      including 15 new marker tests.

In `report.rs`, delete the three moved fns and add
`use crate::markers::{comment_marker_is, line_comment};`. No call-site changes —
signatures are unchanged.

Run:
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-778-allowlist-multiplicity -- cargo nextest run --manifest-path xtask/Cargo.toml`
Expected: PASS, including every pre-existing `report.rs` test.

- [x] **Step 5: Prove the coverage gate still passes on the unchanged tree
      (AC30)**

Run:
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-778-allowlist-multiplicity -- cargo xtask check`
Expected: PASS. A newly-failing coverage line means a live `cov:ignore` depended
on the old raw-string reading — **investigate and report it; do not re-mark the
line.**

- [x] **Step 6: Commit**

```bash
git add xtask/src/markers.rs xtask/src/lib.rs xtask/src/coverage/report.rs docs/superpowers/specs/2026-08-01-issue-778-gate-exemption-markers.md docs/superpowers/plans/2026-08-01-issue-778-gate-exemption-markers.md
git commit -m "refactor(xtask): share the trailing-comment marker primitive (#778)

Promotes line_comment/comment_marker_is/char_literal_len out of the coverage
gate into xtask::markers and adds marker_on_line. Raw strings are now tracked,
so a '//' opening inside r\"...\" is no longer read as a comment — fail-closed,
and inherited by cov:ignore."
```

---

### Task 3: Mark the twelve sites — commit 2

**Files:**

- Modify: `common/src/render.rs`, `common/src/media.rs`,
  `common/src/feed/feed_path.rs`
- Modify: `web/src/home/component.rs`, `web/src/sidebar/component.rs`,
  `web/src/posts/component.rs`, `web/src/html.rs`
- Test: none — the markers are inert until Task 5.

**Interfaces:** Produces twelve marker comments that Tasks 5–6 rely on being
present and on their sites' lines.

- [ ] **Step 1: Add the twelve markers**

Each goes as a **standalone comment line directly above** the line carrying the
matched ident — never trailing it (measured: trailing relocates for 7 of the 12;
standalone-above is stable for all 12). Re-derive line numbers; the text is
fixed — it carries the reasons the allowlists held (AC19). `PostDisplay`'s
single `×2` entry deliberately splits into two distinct reasons, which is the
point of the change:

| File                           | Site                                                    | Marker                                                                                                             |
| ------------------------------ | ------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------ |
| `common/src/render.rs`         | `pub fn from_trusted`                                   | `// rendered-html-from-trusted:allow the door's own definition; the gate pins its uses`                            |
| `common/src/render.rs`         | `.map(RenderedHtml::from_trusted)`                      | `// rendered-html-from-trusted:allow rebuilds RenderedHtml from a wire DTO field our own server serialized (#445)` |
| `common/src/media.rs`          | `pub(crate) fn from_trusted`                            | `// rendered-html-from-trusted:allow ContentType's own door definition — mints a media type, never HTML (#584)`    |
| `common/src/media.rs`          | `ContentType::from_trusted(content_type)`               | `// rendered-html-from-trusted:allow ContentType from a detected, test-pinned media type — never HTML (#584)`      |
| `common/src/media.rs`          | `ContentType::from_trusted("application/octet-stream")` | `// rendered-html-from-trusted:allow ContentType from the octet-stream literal — never HTML (#584)`                |
| `common/src/feed/feed_path.rs` | `ContentType::from_trusted(literal)`                    | `// rendered-html-from-trusted:allow ContentType from a fixed &'static feed media type — never HTML (#584)`        |
| `web/src/home/component.rs`    | masthead `inner_html`                                   | `// html-sink:allow home::render::render_masthead output — the shared pure fn (ADR-0041 §2)`                       |
| `web/src/sidebar/component.rs` | anonymous `inner_html`                                  | `// html-sink:allow sidebar::markup::render_sidebar output — the anonymous paint the projector emits`              |
| `web/src/posts/component.rs`   | `PostDisplay` anonymous                                 | `// html-sink:allow posts::render::render_post_inner output — the same pure render the projector paints (#179)`    |
| `web/src/posts/component.rs`   | `PostDisplay` authored                                  | `// html-sink:allow posts::render::render_post_content output — the projector's own paint (#181)`                  |
| `web/src/posts/component.rs`   | `permalink_first_paint`                                 | `// html-sink:allow posts::render::permalink_article output — the projector's own permalink paint`                 |
| `web/src/html.rs`              | `PreEscaped` in `from_rendered_html`                    | `// raw-html-door:allow re-wraps a RenderedHtml whose safety sanitization established (ADR-0079)`                  |

- [x] **Step 2: Prove the formatters preserve them (Key risk 1)** — **the risk
      fired, and was resolved by measurement.** Written _trailing_ (the spec's
      original rule), 7 of 12 markers relocated: `rustfmt` pushed the two
      `fn from_trusted` signature markers **down** onto the first body line, and
      `leptosfmt` moved five `view!`-related markers **up or down** depending on
      context. All relocations were stable and idempotent; none were dropped.
      Rewritten as standalone comment lines **directly above** each site, all
      twelve stay put across repeated runs. The spec and ADR were amended to
      make above-the-site the canonical position and to refuse trailing (it is
      the form that silently moves). See the spec's "Why above the site rather
      than trailing it".

Run:
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-778-allowlist-multiplicity -- cargo xtask check`
Expected: PASS, and a re-run moves no marker.

Run:
`rg -n -A1 'html-sink:allow|raw-html-door:allow|rendered-html-from-trusted:allow' common/src web/src`
Expected: twelve hits, each on a comment-only line whose **next** line carries
the matched ident (`from_trusted`, `inner_html`, `PreEscaped`).

- [ ] **Step 3: Commit**

```bash
git add common/src/render.rs common/src/media.rs common/src/feed/feed_path.rs web/src/home/component.rs web/src/sidebar/component.rs web/src/posts/component.rs web/src/html.rs
git commit -m "docs(xss-gates): mark the twelve exempt sites in source (#778)

Inert until the gates learn to read them, so the tree stays green and the
formatters' treatment of the markers is proven before anything depends on it."
```

---

### Task 4: `ident_gate` scan and classifier — no commit

Folds into commit 3 (Task 5). Standalone it is dead code, which `xtask-clippy`
denies.

**Files:**

- Modify: `xtask/src/steps/ident_gate.rs`
- Test: in-file `#[cfg(test)] mod marker_tests`

**Interfaces:**

- Consumes: `crate::markers::marker_on_line` (Task 2).
- Produces:

```rust
pub struct Scan {
    /// Non-test mentions, in line order.
    pub mentions: Vec<Mention>,
    /// 1-based inclusive line ranges of test items, so a marker anywhere in test
    /// code is never reported as an orphan.
    pub test_ranges: Vec<(usize, usize)>,
}
pub fn scan<P: Population>(source: &str, population: &P) -> Result<Scan, String>;

pub enum Why {
    Unmarked,
    NoReason,
    Shared(usize),
}
pub struct Unexempt {
    pub line: usize,
    pub function: String,
    pub why: Why,
}
pub struct Marked {
    pub line: usize,
    pub reason: String,
}
pub struct Classified {
    pub unexempt: Vec<Unexempt>,
    pub marked: Vec<Marked>,
    pub orphans: Vec<usize>,
}
pub fn classify(source: &str, found: &Scan, token: &str) -> Classified;
```

Purely additive — `mentions`, `Allowed` and the old allowlist path stay until
Tasks 5–6, so every existing test keeps passing.

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod marker_tests {
    use super::{classify, scan, AnyOf, Classified, Why};

    const TOKEN: &str = "guard:allow";

    fn classified(src: &str) -> Classified {
        let s = scan(src, &AnyOf(&["GUARDED"])).unwrap();
        classify(src, &s, TOKEN)
    }

    #[test]
    fn a_marked_site_is_exempt_and_enters_the_census() {
        let c = classified("// guard:allow because reasons\nfn a() { GUARDED; }\n");
        assert!(c.unexempt.is_empty());
        assert_eq!(c.marked.len(), 1);
        assert_eq!(c.marked[0].line, 2, "the census names the SITE line");
        assert_eq!(c.marked[0].reason, "because reasons");
        assert!(c.orphans.is_empty());
    }

    #[test]
    fn an_unmarked_site_is_unexempt() {
        let c = classified("fn a() { GUARDED; }\n");
        assert_eq!(c.unexempt.len(), 1);
        assert!(matches!(c.unexempt[0].why, Why::Unmarked));
        assert_eq!(c.unexempt[0].function, "a");
        assert!(c.marked.is_empty());
    }

    #[test]
    fn a_bare_marker_is_unexempt() {
        let c = classified("// guard:allow\nfn a() { GUARDED; }\n");
        assert_eq!(c.unexempt.len(), 1);
        assert!(matches!(c.unexempt[0].why, Why::NoReason));
        assert!(c.marked.is_empty());
    }

    /// AC4. Trailing is the position the formatters relocate, so honoring it
    /// would let someone write a marker that stops working on the next format.
    /// It fails twice over: the site sees no marker above it, and the trailing
    /// marker points at a line with no site.
    #[test]
    fn a_trailing_marker_does_not_exempt() {
        let c = classified("fn a() { GUARDED; } // guard:allow trailing\n");
        assert_eq!(c.unexempt.len(), 1);
        assert!(matches!(c.unexempt[0].why, Why::Unmarked));
        assert_eq!(c.orphans, vec![1]);
    }

    #[test]
    fn a_marker_two_lines_above_does_not_exempt() {
        let c = classified("// guard:allow far\n\nfn a() { GUARDED; }\n");
        assert_eq!(c.unexempt.len(), 1);
        assert_eq!(c.orphans, vec![1]);
    }

    #[test]
    fn a_marker_below_the_site_does_not_exempt() {
        let c = classified("fn a() { GUARDED; }\n// guard:allow below\n");
        assert_eq!(c.unexempt.len(), 1);
        assert_eq!(c.orphans, vec![2]);
    }

    #[test]
    fn two_sites_on_the_marked_line_are_both_unexempt() {
        let c = classified("// guard:allow reason\nfn a() { GUARDED; GUARDED; }\n");
        assert_eq!(c.unexempt.len(), 2);
        assert!(c.unexempt.iter().all(|u| matches!(u.why, Why::Shared(2))));
        assert!(c.marked.is_empty());
    }

    #[test]
    fn two_sites_on_one_unmarked_line_are_unmarked_not_shared() {
        let c = classified("fn a() { GUARDED; GUARDED; }\n");
        assert_eq!(c.unexempt.len(), 2);
        assert!(c.unexempt.iter().all(|u| matches!(u.why, Why::Unmarked)));
    }

    #[test]
    fn a_marker_with_no_site_below_is_an_orphan() {
        let c = classified("// guard:allow reason\nfn a() { harmless(); }\n");
        assert_eq!(c.orphans, vec![1]);
        assert!(c.unexempt.is_empty());
    }

    #[test]
    fn a_marker_on_a_test_code_site_is_not_an_orphan() {
        let src = "#[cfg(test)]\nmod t {\n  // guard:allow fixture\n  fn f() { GUARDED; }\n}\n";
        let c = classified(src);
        assert!(c.orphans.is_empty());
        assert!(c.unexempt.is_empty());
        assert!(c.marked.is_empty(), "test code is not part of the census");
    }

    /// AC10's harder half: a marker in test code whose site is GONE. Test regions
    /// are exempt wholesale, so it is not an orphan either.
    #[test]
    fn a_stale_marker_inside_test_code_is_not_an_orphan() {
        let src = "#[cfg(test)]\nmod t {\n  // guard:allow stale\n  fn f() { harmless(); }\n}\n";
        assert!(classified(src).orphans.is_empty());
    }

    #[test]
    fn a_marker_inside_a_string_literal_exempts_nothing() {
        let c = classified("fn b() { let s = \"// guard:allow x\"; }\nfn a() { GUARDED; }\n");
        assert_eq!(c.unexempt.len(), 1);
        assert!(c.orphans.is_empty());
    }

    #[test]
    fn a_doc_comment_marker_exempts_nothing() {
        let c = classified("/// guard:allow x\nfn a() { GUARDED; }\n");
        assert_eq!(c.unexempt.len(), 1);
        assert!(c.orphans.is_empty(), "a doc comment carries no marker");
    }

    #[test]
    fn another_gates_marker_does_not_exempt() {
        let c = classified("// other:allow reason\nfn a() { GUARDED; }\n");
        assert_eq!(c.unexempt.len(), 1);
        assert!(c.orphans.is_empty(), "a foreign token is not this gate's orphan");
    }

    #[test]
    fn a_site_inside_a_macro_body_is_exempted_from_the_line_above() {
        let src = "fn a() -> V {\n    // guard:allow reason\n    m! { GUARDED }\n}\n";
        let c = classified(src);
        assert!(c.unexempt.is_empty());
        assert_eq!(c.marked.len(), 1);
        assert_eq!(c.marked[0].line, 3);
    }

    #[test]
    fn a_multi_line_statement_is_marked_above_the_ident_line() {
        let src = "fn a() {\n    take(\n        // guard:allow reason\n        GUARDED,\n    );\n}\n";
        let c = classified(src);
        assert!(c.unexempt.is_empty());
        assert_eq!(c.marked[0].line, 4);
    }

    /// AC9: above the IDENT's line, not above the statement that contains it.
    #[test]
    fn a_marker_above_the_statements_first_line_does_not_exempt() {
        let src = "fn a() {\n    // guard:allow reason\n    take(\n        GUARDED,\n    );\n}\n";
        let c = classified(src);
        assert_eq!(c.unexempt.len(), 1);
        assert_eq!(c.orphans, vec![2]);
    }

    #[test]
    fn the_census_comes_back_in_line_order() {
        let src = "// guard:allow first\nfn a() { GUARDED; }\n// guard:allow second\nfn b() { GUARDED; }\n";
        let c = classified(src);
        assert_eq!(c.marked.iter().map(|m| m.line).collect::<Vec<_>>(), vec![2, 4]);
    }
}
```

- [ ] **Step 2: Run, verify it does not build**

Run:
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-778-allowlist-multiplicity -- cargo nextest run --manifest-path xtask/Cargo.toml marker_tests`
Expected: **build failure** — `scan`, `classify`, `Why`, `Classified` not
defined.

- [ ] **Step 3: Implement**

Add `test_ranges: Vec<(usize, usize)>` to `Scanner`. In `visit_item_mod` /
`visit_item_impl` / `visit_item_fn` / `visit_impl_item_fn`, when the item is
test code (`is_test_cfg` or `has_test_attr`), record its 1-based inclusive line
range via `syn::spanned::Spanned::span(i).start().line` / `.end().line`. Add
`scan` alongside `mentions`, returning mentions sorted by line plus the ranges.

`classify` walks `found.mentions`, grouping by line for each line's site count,
and consults `marker_on_line` on the line **immediately above** each mention
(none when the mention is on line 1). Four outcomes — no marker above, empty
reason, count > 1 on the marked line, otherwise marked — each pinned by a Step 1
test. Orphans are every 1-based source line where `marker_on_line` returns
`Some`, whose **next** line carries no mention, and that falls inside no
`test_ranges` entry.

Note the census records the **site's** line, not the marker's — that is the line
a reader needs and the line the failure messages already print.

- [ ] **Step 4: Run, verify pass**

Run:
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-778-allowlist-multiplicity -- cargo nextest run --manifest-path xtask/Cargo.toml`
Expected: PASS — the new tests plus every pre-existing `ident_gate` and gate
test.

**No commit** — `scan`/`classify` have no non-test caller until Task 5.

---

### Task 5: Convert `html-sink` and `raw-html-door` — commit 3

Both in one task: they share the `Gate`/`Report` change, so converting one
leaves the other failing to compile and neither is independently verifiable.

**Files:**

- Modify: `xtask/src/steps/ident_gate.rs` (`Gate` loses `allowlist`; `Report`
  loses `noun`/`vanished`; `problems`/`violations` use `classify`; delete
  `Allowed` and the old allowlist helper)
- Modify: `xtask/src/steps/html_sink_check.rs`,
  `xtask/src/steps/raw_html_door_check.rs`
- Test: in-file, all three modules

**Interfaces:**

- Consumes: `scan`, `classify` (Task 4); the markers from Task 3.
- Produces:

```rust
pub struct Gate<P: Population> {
    pub step: &'static str,
    pub roots: &'static [&'static str],
    pub population: P,
    pub report: Report,
}
impl<P: Population> Gate<P> {
    /// The marker token this gate honors: `"<step>:allow"`.
    pub fn marker_token(&self) -> String;
    pub fn problems(&self, scanned: &[(String, String)]) -> Option<String>;
    #[cfg(test)]
    pub fn violations(&self, source: &str) -> Result<Vec<(usize, String)>, String>;
}
pub struct Report {
    pub subject: &'static str,
    pub verdict: &'static str,
    pub recovery: &'static str,
}
```

- [ ] **Step 1: Rewrite the `ident_gate` allowlist tests**

Delete: `an_entry_covers_its_declared_count_and_no_more`,
`a_zero_count_entry_exempts_nothing`,
`a_non_top_level_fn_cannot_borrow_the_entry`, `an_unlisted_fn_is_never_covered`
(all test the deleted key). Keep `mentions_come_back_in_line_order`, retargeted
at `scan`.

- [ ] **Step 2: Rewrite `html_sink_check`'s tests**

> **Marker placement in every fixture below:** the code blocks in this task and
> Task 6 were written against the original trailing-marker rule. Transpose each
> one — the marker is a standalone comment line **directly above** its site.
> `a_marked_sink_passes` and friends therefore read `// html-sink:allow …` on
> its own line, then the `view!` line. Any fixture that keeps a trailing marker
> is now testing AC4 (it must fail).

**Keep unchanged:** `an_unlisted_sink_is_a_violation`,
`set_inner_html_outside_a_macro_is_in_the_population`,
`a_comment_mentioning_inner_html_does_not_trip`,
`unparseable_source_is_a_hard_error`, `a_sink_in_a_cfg_test_module_is_exempt`,
`a_cfg_not_test_production_fn_is_scanned`,
`an_import_alone_is_outside_the_population`,
`a_sink_at_module_scope_is_flagged`,
`problems_surfaces_a_parse_failure_with_the_file`.

**Delete:** `an_allowlisted_fn_at_its_declared_multiplicity_passes`,
`exceeding_a_declared_multiplicity_is_a_violation`,
`a_nested_fn_shadowing_an_allowed_name_is_still_flagged`,
`a_same_named_fn_in_another_file_breaks_the_declared_multiplicity`,
`a_vanished_sink_leaves_a_stale_entry_that_fails`.

**Rewrite (they assert against the deleted allowlist output):**

- `the_real_tree()` (`:278-314`) — add the five markers from Task 3's table to
  the fixture sources, so it models the marked tree.
- `problems_is_none_for_the_four_allowlisted_fns_at_their_multiplicities` →
  rename `problems_is_none_for_the_fully_marked_tree`, same `the_real_tree()`
  input.
- `problems_reports_file_line_and_recovery` — drop the `contains("ALLOWLIST")`
  and `contains("fn \`PostDisplay\` ×2")` assertions; assert the marker
  instruction and a census entry instead.

**Add** (AC12: every mechanism AC also exercised through `Gate::violations`, not
only through `classify`):

```rust
#[test]
fn a_marked_sink_passes() {
    let src = r#"
        fn anything(html: Markup) -> AnyView {
            view! { <div inner_html=html></div> }.into_any() // html-sink:allow pure render output
        }
    "#;
    assert_eq!(violations(src).unwrap(), vec![]);
}

/// The old ALLOWLIST exempted `PostDisplay` wholesale at count 2. Now each sink
/// argues for itself and a name buys nothing.
#[test]
fn a_formerly_allowlisted_fn_name_grants_nothing() {
    let src = r#"
        fn PostDisplay(view: PostView) -> AnyView {
            view! { <article inner_html=inner></article> }.into_any()
        }
    "#;
    assert_eq!(violations(src).unwrap().len(), 1);
}

#[test]
fn two_sinks_in_one_fn_each_need_their_own_marker() {
    let src = r#"
        fn PostDisplay(view: PostView) -> AnyView {
            if a {
                view! { <article inner_html=inner></article> }.into_any() // html-sink:allow anonymous layout
            } else {
                view! { <div inner_html=inner_content></div> }.into_any() // html-sink:allow authored layout
            }
        }
    "#;
    assert_eq!(violations(src).unwrap(), vec![]);
}

#[test]
fn a_bare_marker_fails() {
    let src = "fn f(h: Markup) -> AnyView { view! { <div inner_html=h></div> }.into_any() } // html-sink:allow\n";
    assert_eq!(violations(src).unwrap().len(), 1);
}

#[test]
fn an_orphan_marker_fails() {
    let src = "fn f() { harmless(); } // html-sink:allow stale\n";
    assert_eq!(violations(src).unwrap().len(), 1);
}

#[test]
fn a_marker_on_the_adjacent_line_does_not_exempt() {
    // AC4 through the gate: the marker must be on the site's own line.
    let src = "// html-sink:allow above\nfn f() { el.set_inner_html(h); }\n";
    assert_eq!(violations(src).unwrap().len(), 2, "the site AND the orphan");
}

#[test]
fn a_doc_comment_marker_does_not_exempt() {
    let src = "/// html-sink:allow prose\nfn f() { el.set_inner_html(h); }\n";
    assert_eq!(violations(src).unwrap().len(), 1);
}

#[test]
fn two_sinks_on_one_marked_line_fail() {
    let src = "fn f() { el.set_inner_html(a); el.set_inner_html(b); } // html-sink:allow r\n";
    assert_eq!(violations(src).unwrap().len(), 2);
}

#[test]
fn a_multi_line_call_is_marked_on_the_ident_line() {
    let src = "fn f() {\n    el.set_inner_html(\n        h, // html-sink:allow pure render output\n    );\n}\n";
    // The ident is on line 2; the marker is on line 3.
    assert_eq!(violations(src).unwrap().len(), 2, "unmarked site plus an orphan");
}

#[test]
fn a_raw_html_door_marker_does_not_exempt_a_sink() {
    let src = "fn f(h: Markup) -> AnyView { view! { <div inner_html=h></div> }.into_any() } // raw-html-door:allow wrong gate\n";
    assert_eq!(violations(src).unwrap().len(), 1);
}

#[test]
fn problems_reports_the_bare_marker_distinctly() {
    let scanned = vec![(
        "web/src/x.rs".to_string(),
        "fn f() { el.set_inner_html(h); } // html-sink:allow\n".to_string(),
    )];
    assert!(problems(&scanned).expect("a problem").contains("bare"));
}

#[test]
fn problems_reports_a_shared_line_distinctly() {
    let scanned = vec![(
        "web/src/x.rs".to_string(),
        "fn f() { el.set_inner_html(a); el.set_inner_html(b); } // html-sink:allow r\n".to_string(),
    )];
    assert!(problems(&scanned).expect("a problem").contains("split"));
}

#[test]
fn problems_ends_with_the_derived_census() {
    let scanned = vec![
        (
            "web/src/a.rs".to_string(),
            "fn f() { el.set_inner_html(h); } // html-sink:allow the good one\n".to_string(),
        ),
        ("web/src/b.rs".to_string(), "fn g() { el.set_inner_html(h); }\n".to_string()),
    ];
    let detail = problems(&scanned).expect("a problem");
    assert!(detail.contains("web/src/a.rs:1 — the good one"), "{detail}");
}
```

**Note on `a_multi_line_call_is_marked_on_the_ident_line`:** the expectation is
deliberately "2" — a site with no marker on its line, plus the marker sitting
alone on line 3 as an orphan. That is the rule working, and pins AC9 from the
failing direction.

- [ ] **Step 3: Rewrite `raw_html_door_check`'s tests**

**Keep:** the population tests, `unparseable_source_is_a_hard_error`, the
`cfg(test)` / `cfg(not(test))` tests,
`problems_surfaces_a_parse_failure_with_the_file`.

**Delete:** the multiplicity and nested-shadow tests.

**Rewrite:** `THE_DOOR` fixture (`:139-148`) — add the Task 3 marker.
`problems_is_none_for_the_one_declared_door` →
`problems_is_none_for_the_marked_door`.
`problems_reports_file_line_and_recovery` — drop `contains("ALLOWLIST")` and
`contains("fn \`from_rendered_html\` ×1")`.

**Add:**

```rust
#[test]
fn a_marked_door_passes() {
    let src = "fn f(h: &RenderedHtml) -> Self { Self(PreEscaped(h.as_ref()).into_string()) } // raw-html-door:allow inherits sanitize's invariant (ADR-0079)\n";
    assert_eq!(violations(src).unwrap(), vec![]);
}

#[test]
fn a_formerly_allowlisted_fn_name_grants_nothing() {
    let src = "fn from_rendered_html(h: &RenderedHtml) -> Self { Self(PreEscaped(h.as_ref()).into_string()) }\n";
    assert_eq!(violations(src).unwrap().len(), 1);
}

#[test]
fn a_bare_marker_fails() {
    let src = "fn f(h: &str) -> Markup { PreEscaped(h.to_string()) } // raw-html-door:allow\n";
    assert_eq!(violations(src).unwrap().len(), 1);
}

#[test]
fn an_orphan_marker_fails() {
    let src = "fn f() { harmless(); } // raw-html-door:allow stale\n";
    assert_eq!(violations(src).unwrap().len(), 1);
}

#[test]
fn an_html_sink_marker_does_not_exempt_a_door() {
    let src = "fn f(h: &str) -> Markup { PreEscaped(h.to_string()) } // html-sink:allow wrong gate\n";
    assert_eq!(violations(src).unwrap().len(), 1);
}
```

- [ ] **Step 4: Run, verify it does not build**

Run:
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-778-allowlist-multiplicity -- cargo nextest run --manifest-path xtask/Cargo.toml`
Expected: **build failure** — `Gate` still has `allowlist`, `Report` still
requires `noun`/`vanished`.

- [ ] **Step 5: Implement**

In `ident_gate`: drop `allowlist` from `Gate` and `noun`/`vanished` from
`Report`; add `marker_token` returning `format!("{}:allow", self.step)`; rewrite
`Gate::problems` to `scan` + `classify` per file, emitting one line per
`Unexempt` and per orphan and appending the derived census after
`report.recovery`. Delete `Allowed` and the old allowlist-application helper.
Message shapes, each pinned above:

- `Why::Unmarked` →
  `{path}:{line}: {subject} {in fn `x` | at module scope} {verdict}`
- `Why::NoReason` →
  `{path}:{line}: {subject} {where} carries a bare `{token}` marker — an exemption with no reason is not an exemption; say why this site is safe`
- `Why::Shared(n)` →
  `{path}:{line}: {n} `{step}` sites share this line, so one marker cannot justify them — split the line so each carries its own`
- orphan →
  `{path}:{line}: `{token}`marker on a line with no`{step}` site — a stale exemption; delete it`
- census → `    - {path}:{line} — {reason}`

`Gate::violations` returns `unexempt` and `orphans` as `(line, function)` pairs
(orphans carry an empty function), sorted by line.

In both gate modules: delete `ALLOWLIST` and the `Allowed` import, drop
`noun`/`vanished`, and rewrite `recovery` to end with the marker instruction and
"Currently marked:" in place of "Currently exempt:".

- [ ] **Step 6: Run the suite and the gate**

Run:
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-778-allowlist-multiplicity -- cargo xtask check`
Expected: PASS. `rendered-html-from-trusted` is untouched and still uses
`mentions`/`top_level`/`expr_path`, so nothing is dead yet.

- [ ] **Step 7: Commit**

```bash
git add xtask/src/steps/ident_gate.rs xtask/src/steps/html_sink_check.rs xtask/src/steps/raw_html_door_check.rs
git commit -m "feat(xtask): html-sink and raw-html-door exempt by in-source marker (#778)

Gate loses its allowlist; each site argues for itself with a
'<step>:allow <reason>' comment on its own line. A bare marker, two sites
sharing a marked line, and an orphan marker all fail. The census is derived
from the scan, so it cannot go stale. Allowed and the multiplicity
reconciliation go with the last caller."
```

---

### Task 6: Convert `rendered-html-from-trusted` — commit 4

**Files:**

- Modify: `xtask/src/steps/rendered_html_from_trusted_check.rs`
- Modify: `xtask/src/steps/ident_gate.rs` (delete `mentions`,
  `Mention::top_level`, `Population::expr_path`, `Scanner::visit_expr_path`,
  `AnyOf::expr_path`)

**Interfaces:** Produces a `Gate<AnyOf>`, making all three gates one shape.
Deleting the `expr_path` hook is safe: `TrustedDoor` is its only real
implementor and dies here; `server_fn_registrar_check.rs:182`'s
`visit_expr_path` is an unrelated `syn::visit::Visit` method.

- [ ] **Step 1: Rewrite the tests**

> **Marker placement:** as in Task 5 — every marker in the fixtures below is a
> standalone comment line **directly above** its site, not trailing it.

**Delete:** `allowlisted_fn_is_clean`,
`map_reference_in_allowlisted_fn_is_clean`,
`a_nested_fn_shadowing_an_allowed_name_is_still_flagged`,
`content_type_door_is_exempt_in_a_non_allowlisted_fn`,
`content_type_map_reference_is_exempt`,
`an_exempt_qualifier_inside_a_macro_body_is_still_exempt`,
`the_definition_site_has_no_path_mention`,
`problems_is_none_for_allowlisted_and_test_sites`.

**Keep:** `call_in_a_non_allowlisted_fn_is_flagged`,
`an_inbound_shaped_fn_using_from_trusted_is_flagged`,
`map_reference_in_a_non_allowlisted_fn_is_flagged`,
`a_from_trusted_on_an_unrelated_type_is_still_flagged`,
`call_in_a_cfg_test_module_is_exempt`,
`a_cfg_not_test_production_fn_is_scanned`, `call_in_a_test_fn_is_exempt`,
`module_scope_call_is_flagged`, the four macro-body tests,
`parse_failure_is_an_error`, `problems_surfaces_a_parse_failure_with_the_file`.

**Rewrite:** `problems_reports_file_line_and_recovery` — its
`contains("not an allowlisted trusted-rebuild door")` and
`contains("ALLOWED_FNS")` are both killed by the AC24 prose rewrite. Assert the
new verdict and the marker instruction.

**Add:**

```rust
#[test]
fn a_marked_door_passes() {
    let src = "fn deserialize_rendered_html(s: String) -> RenderedHtml { RenderedHtml::from_trusted(s) } // rendered-html-from-trusted:allow wire DTO our own server serialized (#445)\n";
    assert_eq!(violations(src).unwrap(), vec![]);
}

#[test]
fn a_formerly_allowlisted_fn_name_grants_nothing() {
    let src = "fn deserialize_rendered_html(s: String) -> RenderedHtml { RenderedHtml::from_trusted(s) }\n";
    assert_eq!(violations(src).unwrap().len(), 1);
}

/// #778: the qualifier exemption is gone. `ContentType::from_trusted` is a
/// different door, but it says so in a marker rather than self-exempting from a
/// pattern (ADR-0085 principle 3) — which also closes the qualifier-alias
/// fail-open (`use RenderedHtml as ContentType`).
#[test]
fn a_content_type_door_is_in_the_population_and_needs_a_marker() {
    let src = "fn detect(n: &str) -> ContentType { ContentType::from_trusted(n) }\n";
    assert_eq!(violations(src).unwrap().len(), 1);
}

#[test]
fn a_marked_content_type_door_passes() {
    let src = "fn detect(n: &str) -> ContentType { ContentType::from_trusted(n) } // rendered-html-from-trusted:allow mints a media type, never HTML (#584)\n";
    assert_eq!(violations(src).unwrap(), vec![]);
}

/// Replaces `the_definition_site_has_no_path_mention`. Under `AnyOf` the door's
/// own declaration is in the population — a deliberate behavior change, failing
/// closed. `visit_impl_item_fn` pushes the fn name before the signature's ident
/// is visited, so the mention's enclosing fn is the door itself.
#[test]
fn the_definition_site_is_in_the_population() {
    let src = "impl RenderedHtml {\n    pub fn from_trusted(h: impl Into<String>) -> Self { Self(h.into()) }\n}\n";
    assert_eq!(violations(src).unwrap(), vec![(2, "from_trusted".to_string())]);
}

#[test]
fn a_marked_definition_site_passes() {
    let src = "impl RenderedHtml {\n    pub fn from_trusted(h: impl Into<String>) -> Self { Self(h.into()) } // rendered-html-from-trusted:allow the door's own definition\n}\n";
    assert_eq!(violations(src).unwrap(), vec![]);
}

/// AC24: the VERDICT line must not assert RenderedHtml or unescaped emission at
/// a site that is neither. Asserted against the first line only — the recovery
/// paragraph legitimately discusses `RenderedHtml`.
#[test]
fn the_verdict_does_not_name_one_type() {
    let scanned = vec![(
        "common/src/media.rs".to_string(),
        "fn detect(n: &str) -> ContentType { ContentType::from_trusted(n) }\n".to_string(),
    )];
    let detail = problems(&scanned).expect("a problem");
    let verdict = detail.lines().next().expect("a violation line");
    assert!(!verdict.contains("RenderedHtml"), "{verdict}");
    assert!(!verdict.contains("emitted unescaped"), "{verdict}");
    assert!(verdict.contains("from_trusted"), "{verdict}");
}

#[test]
fn problems_is_none_for_a_fully_marked_tree() {
    let scanned = vec![(
        "common/src/render.rs".to_string(),
        "fn deserialize_rendered_html(s: String) -> RenderedHtml { RenderedHtml::from_trusted(s) } // rendered-html-from-trusted:allow wire DTO (#445)\n".to_string(),
    )];
    assert_eq!(problems(&scanned), None);
}
```

- [ ] **Step 2: Run, verify failure**

Run:
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-778-allowlist-multiplicity -- cargo nextest run --manifest-path xtask/Cargo.toml rendered_html`
Expected: FAIL — `EXEMPT_QUALIFIERS` still exempts `ContentType`; the definition
site is still invisible.

- [ ] **Step 3: Implement the gate**

Delete `ALLOWED_FNS`, `EXEMPT_QUALIFIERS`, `TrustedDoor`,
`macro_qualifier_is_exempt`, and the local `violations`/`problems`/`run` built
on `ident_gate::mentions`. Replace with:

```rust
const DOORS: &[&str] = &["from_trusted"];

const GATE: Gate<AnyOf> = Gate {
    step: "rendered-html-from-trusted",
    roots: POLICED_ROOTS,
    population: AnyOf(DOORS),
    report: Report { /* subject, verdict, recovery */ },
};
```

The `Report` must be accurate at **every** member of the widened population
(AC24), so it names the ident, not one type. Subject:
``"a `from_trusted` door"``. Verdict:
`"is not marked — this gate pins every `from*trusted`in production code, because`RenderedHtml`'s is the door that lets HTML reach the DOM unescaped (XSS) (#398)"`.
Recovery keeps the sanitize-vs-inherit guidance, adds that a `from_trusted` on
another type is \_marked* as such rather than self-exempting, and ends with the
marker instruction and "Currently marked:". Then `violations` (`#[cfg(test)]`),
`problems` and `run` delegate to `GATE`, matching `html_sink_check.rs:146-162`.

- [ ] **Step 4: Delete the now-dead machinery**

Remove `mentions`, `Mention::top_level`, `Population::expr_path`,
`Scanner::visit_expr_path` and `AnyOf::expr_path` from `ident_gate`. Update
`Population`'s trait doc, which explains why all hooks are required.

- [ ] **Step 5: Verify the deletion (AC16)**

Run:
`rg -n 'Allowed|unjustified|top_level|expr_path|EXEMPT_QUALIFIERS|macro_qualifier_is_exempt|TrustedDoor|ALLOWLIST|ALLOWED_FNS' xtask/src/steps/ident_gate.rs xtask/src/steps/html_sink_check.rs xtask/src/steps/raw_html_door_check.rs xtask/src/steps/rendered_html_from_trusted_check.rs`
Expected: no output. (Other `steps/` modules keep several of these names
legitimately — see AC16's carve-out. `rg` is case-sensitive, and the classifier
deliberately spells its field `unexempt`.)

- [ ] **Step 6: Run the suite and the gate**

Run:
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-778-allowlist-multiplicity -- cargo xtask check`
Expected: PASS — all three gates green against the twelve markers from Task 3,
no dead-code warnings. A failure naming a site means its marker is missing or on
the wrong line.

- [ ] **Step 7: Commit**

```bash
git add xtask/src/steps/ident_gate.rs xtask/src/steps/rendered_html_from_trusted_check.rs
git commit -m "feat(xtask): rendered-html-from-trusted exempts by marker (#778)

Drops EXEMPT_QUALIFIERS — a pattern-decided exemption (ADR-0085 principle 3)
that failed open on a qualifier alias. The population is now
AnyOf(from_trusted), so ContentType doors and the definition sites are in it
and carry their own markers, and the verdict names the ident rather than one
type.

mentions, Mention::top_level and Population::expr_path existed to make a
name-keyed exemption work, and go with the last caller."
```

---

### Task 7: Documentation corrections — commit 5

**Files:**

- Modify: `xtask/src/steps/ident_gate.rs`, `html_sink_check.rs`,
  `raw_html_door_check.rs`, `rendered_html_from_trusted_check.rs` (module docs)
- Modify: `docs/adr/0093-web-render-html-macro.md`,
  `0085-static-type-safety-gates-enumerate.md`,
  `0080-media-path-naming-correspondence.md`,
  `0079-rendered-html-sanitization.md`, `0050-stateless-coverage-gate.md`
- Modify: `common/src/media.rs`, `common/src/render.rs`

- [ ] **Step 1: Module docs (AC20, AC21, AC28)**

- `ident_gate`: delete the two-layer allowlist description, the `#778` reference
  at `:23`, and unreadable class 4 (the fn-name key — there is no name key now).
  Describe the marker contract in its place. (The `#778` reference at `:288`
  lived in `Allowed`'s doc and is already gone with Task 5.)
- All three gates: state the unreadable classes, adding **"a marker is trusted,
  not verified — the gate checks that a reason exists and that its site still
  exists, never that the reason is true"** and **"a marked site is exempt
  regardless of what value flows into it; there is no call graph"**.
- `html_sink_check`: rehome the deleted `ALLOWLIST` doc's collective observation
  — that every sink's reason has the same shape (the injected value is the pure
  render layer's output, the same fn the projector paints) and that the
  uniformity is the point.

- [ ] **Step 2: ADR corrections (AC26–AC29)**

- **ADR-0093** "What it creates": replace the `ALLOWED_FNS` follow-up paragraph
  with the landed decision, citing the new ADR by its
  `docs/adr/drafts/gate-exemptions-in-source-markers.md` path (`promote`
  rewrites it to the numbered path at ship).
- **ADR-0085**: add the three ident gates to **Conformance**, recording that
  they conform via in-source markers. Amend the Consequences sentence claiming
  co-location _with the gate_ discharges the "record why these sites are fine"
  requirement — the reason now lives at the site; name the new ADR as
  superseding it. **Do not touch the six principles.**
- **ADR-0080:** correct the claim that `EXEMPT_QUALIFIERS` is untouched — the
  const no longer exists.
- **ADR-0079** (§88-89) **and the mirroring comment in `common/src/render.rs`**:
  both say the gate matches `from_trusted` **in expression position**. It now
  matches the ident anywhere, definitions included. Correct the mechanism; the
  residual-risk conclusion is unaffected and stays.
- **ADR-0050:** add the cross-reference — `cov:ignore` is the same
  written-exemption mechanism priced for a larger, lower-stakes population, and
  the two now share a marker primitive.
- **`common/src/media.rs`:** the parenthetical claiming the `ContentType::`
  qualifier is exempt is false; the site is marked like any other.

- [ ] **Step 3: Verify the ADR draft covers AC25**

Read `docs/adr/drafts/gate-exemptions-in-source-markers.md` and confirm it
records everything AC25 lists: the marker decision and its rules; that a machine
can re-check an _inferred_ exemption but never a _written_ one (so `cov:ignore`
is permanent too and re-checkability separates neither population); that keying
and a derivable census decide marker vs. central list, with review weight the
accepted loss; that stakes set strictness rather than mechanism; the
`EXEMPT_QUALIFIERS` deletion as ADR-0085 principle 3; and the two accepted
costs. Add anything missing — the draft is gitignored and ungated, so nothing
else will catch a gap.

- [ ] **Step 4: Verify no stale claim survives**

Run:
`rg -n 'ALLOWED_FNS|allowlisted trusted-rebuild|in expression position' docs/adr common/src xtask/src/steps`
Expected: no output.

`EXEMPT_QUALIFIERS` is deliberately **not** in that pattern: Step 2 requires
ADR-0080 to state that the const no longer exists, and AC25's ADR records its
deletion, so live prose mentions it by name on purpose.

- [ ] **Step 5: Run the gate (doc-links and ADR format are gated)**

Run:
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-778-allowlist-multiplicity -- cargo xtask check`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add xtask/src/steps docs/adr common/src/media.rs common/src/render.rs
git commit -m "docs(xss-gates): correct every description of the old exemption mechanism (#778)

ADR-0085 gains the three ident gates in Conformance and loses the sentence
claiming co-location with the gate discharges the justification requirement.
ADR-0093, -0080, -0079 and ADR-0050 are corrected or cross-referenced, and
each gate's module doc states that a marker is trusted, not verified."
```

---

### Task 8: Full gate

**Files:** none.

- [ ] **Step 1: Run the full local gate**

Run (Bash background mode — long and cold):
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-778-allowlist-multiplicity -- cargo xtask validate --no-e2e`
Expected: PASS (AC31).

- [ ] **Step 2: Confirm the marker census matches the spec**

Run:
`rg -c 'html-sink:allow|raw-html-door:allow|rendered-html-from-trusted:allow' common/src web/src`
Expected: twelve total. A different count is a finding to investigate, not a
number to adjust.

- [ ] **Step 3: Hand off to jaunder-ship**

The ADR draft at `docs/adr/drafts/gate-exemptions-in-source-markers.md` is
gitignored and is numbered, moved, status-rewritten and staged by
`cargo xtask adr promote` during **jaunder-ship**, after the final rebase onto
`main`. No task here commits it.
