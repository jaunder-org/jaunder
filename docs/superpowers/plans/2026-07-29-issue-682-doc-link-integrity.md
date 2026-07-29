# Markdown Link Integrity Implementation Plan (#682)

> **For agentic workers:** Execute this plan task-by-task with `jaunder-iterate`
> (delegating individual tasks to a subagent via `jaunder-dispatch` when
> useful). Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `cargo xtask adr promote` produce working relative links,
backfill the 19 dead ones already in the tree, and add a `doc-links` gate so the
class cannot recur.

**Architecture:** One new `xtask` module, `doc_links`, owns a single Markdown
link scanner that skips code spans and fenced blocks. Two callers share it:
`adr::run_promote` (the file it just wrote) and the new `doc-links` gate step
(the whole tracked corpus). No new CLI subcommand.

**Tech Stack:** Rust, `xtask` host workspace (separate from the root workspace —
`--manifest-path xtask/Cargo.toml`), `anyhow`, `git` via `xtask::git`.

**Spec:**
[2026-07-29-issue-682-doc-link-integrity.md](../specs/2026-07-29-issue-682-doc-link-integrity.md)
— the "what" and "why". This plan is the "how"; decisions are cited as
**D1**–**D10** and criteria as **AC1**–**AC14** rather than restated.

## Global Constraints

- **No `Co-Authored-By` trailer** on any commit.
- **Every commit must be green.** `.githooks/pre-commit` runs the full
  `cargo xtask check`. Run it yourself first (**`jaunder-commit`**).
- **No unused symbols in any commit.** `xtask-clippy` runs
  `cargo clippy --manifest-path xtask/Cargo.toml --all-targets -- -D warnings`,
  and `--all-targets` builds the plain `--lib` target _without_ `cfg(test)`.
  Every module here is declared privately in `lib.rs`, so `dead_code` applies to
  `pub` items and **struct fields** too: a field written but never read outside
  tests fails the commit. Task boundaries below are drawn so each commit's
  symbols all have a non-test reader.
- **No editing while a gated commit is in flight.** Nix builds the working tree
  mid-commit; serialize edit → gate → commit.
- **xtask is a separate workspace.** Test with
  `cargo test --manifest-path xtask/Cargo.toml`, never a bare workspace
  `nextest`.
- **xtask is not coverage-measured** (**D8**). Tests gate via the `xtask-tests`
  step; no coverage threshold applies.
- **`cargo xtask check` auto-fixes formatting.** Re-check
  `git status --porcelain` after a green run and stage anything prettier or
  rustfmt rewrote.

---

## Review header

**Scope — in:**

- Promote strips one `../` level from its own link targets (**D1**, **D2**,
  **D9**).
- Promote warns (never fails) on links still dead after Pass C (**D4**, **D9**).
- 19 dead links backfilled across 4 files (spec Scope §2).
- A `doc-links` gate over tracked `*.md` minus `docs/archive/` and
  `docs/superpowers/` (spec Scope §3).
- Four documentation updates (spec Scope §4, plus the cross-draft rule — see
  "Key risks").

**Scope — out:** `docs/archive/`'s 177 dead links; `docs/superpowers/`'s 2;
anchor validation; rewriting link _text_ during promotion; general path
resolution in promote; non-inline link syntaxes. See the spec's "Out of scope".

**No separable concerns to file.** The `ARCHITECTURE.md`/`DESIGN.md` rot that
investigation surfaced was folded into this issue by explicit decision, so this
plan has no issue-filing first task.

**Tasks:**

1. Backfill the 19 dead relative links (docs only — must precede Task 3).
2. Add the `doc_links` scanner and make promote strip one path level (Pass B).
3. Add the gate logic and wire the `doc-links` step.
4. Promote warns on links still dead after Pass C.
5. Document the gate, the sibling-link rule, the cross-draft rule, and the
   amended invariant.

**Key risks / decisions:**

- **Task ordering is load-bearing twice over.** Task 1 must precede Task 3, or
  the gate is red on arrival and the pre-commit hook refuses the commit. And the
  scanner ships with promote's strip (Task 2) rather than with the gate (Task
  3), because `Link`'s fields are read by `strip_one_level`; shipping the struct
  a commit earlier would fail `dead_code`.
- **`docs/ARCHITECTURE.md` conflicts with parked work** in the
  `adr-materialized-view` worktree (8 of the 19 links). Mechanical to reapply;
  flagged, not avoided.
- **A live workflow landmine, fixed in Task 5.** `drafts/README.md` rule 3 tells
  authors to reference a draft by its repo-root path
  (`docs/adr/drafts/<slug>.md`). For a draft referencing _another draft_, Pass C
  turns that into `docs/adr/NNNN-slug.md`, which is dead from inside `docs/adr/`
  — promote warns (**D4**) and then `doc-links` **hard-fails the ship commit**
  with no mechanical fix. The form that survives promotion is
  `../drafts/<slug>.md`: Pass B strips it to `drafts/<slug>.md`, Pass C rewrites
  it to `NNNN-slug.md`, which resolves. Task 5 documents this; Task 4's ordering
  test uses exactly this shape.

---

## Task 1: Backfill the 19 dead relative links

Docs-only. Lands first so Task 3's gate is green on arrival.

**Files:**

- Modify: `docs/ARCHITECTURE.md` (lines 5, 6, 7, 58, 61, 70, 90, 103)
- Modify: `docs/DESIGN.md` (lines 6, 34, 44, 63, 69, 75, 80)
- Modify: `docs/adr/0057-e2e-capture-dir-contract.md:13`
- Modify: `docs/adr/0069-client-crate-wasm-only-home.md` (lines 12, 29, 35)
- Test: none — verified by Step 4's commands, and permanently by Task 3.

**Interfaces:**

- Consumes: nothing.
- Produces: a tree with zero dead relative links in the gated set, which Task
  3's `doc-links` step depends on to pass (**AC10**).

- [x] **Step 1: Apply the 14 `decisions/` → `adr/` renames**

Link target only; the visible text does not name the directory in any of these.

`docs/ARCHITECTURE.md`:

| Line | From                                      | To                                  |
| ---- | ----------------------------------------- | ----------------------------------- |
| 5    | `decisions/0002-frontend-framework.md`    | `adr/0002-frontend-framework.md`    |
| 6    | `decisions/0008-deployment-model.md`      | `adr/0008-deployment-model.md`      |
| 7    | `decisions/0001-storage-backends.md`      | `adr/0001-storage-backends.md`      |
| 58   | `decisions/0001-storage-backends.md`      | `adr/0001-storage-backends.md`      |
| 61   | `decisions/0006-storage-isolation.md`     | `adr/0006-storage-isolation.md`     |
| 90   | `decisions/0007-auth-mechanisms.md`       | `adr/0007-auth-mechanisms.md`       |
| 103  | `decisions/0011-unified-observability.md` | `adr/0011-unified-observability.md` |

`docs/DESIGN.md`:

| Line | From                                      | To                                  |
| ---- | ----------------------------------------- | ----------------------------------- |
| 6    | `decisions/0008-deployment-model.md`      | `adr/0008-deployment-model.md`      |
| 34   | `decisions/0004-pagination-strategy.md`   | `adr/0004-pagination-strategy.md`   |
| 44   | `decisions/0006-storage-isolation.md`     | `adr/0006-storage-isolation.md`     |
| 63   | `decisions/0005-unified-content-model.md` | `adr/0005-unified-content-model.md` |
| 69   | `decisions/0010-protocol-integration.md`  | `adr/0010-protocol-integration.md`  |
| 75   | `decisions/0009-edit-delete-policy.md`    | `adr/0009-edit-delete-policy.md`    |
| 80   | `decisions/0003-asset-management.md`      | `adr/0003-asset-management.md`      |

- [x] **Step 2: Fix the stale crate-layout link (text and target)**

`docs/ARCHITECTURE.md:70` currently reads:

```markdown
The [**`common/src/storage/`**](../common/src/storage/) directory is the single
```

`common/src/storage/` no longer exists; those traits live in the `storage`
crate. Replace with:

```markdown
The [**`storage/src/`**](../storage/src/) directory is the single
```

This is the one backfill where the **link text** changes too (the spec's "Out of
scope" notes why promotion never does this automatically).

- [x] **Step 3: Drop the `../` from the four ADR sibling links**

`docs/adr/0057-e2e-capture-dir-contract.md:13`:
`../0049-app-driven-scoped-server-diagnostics.md` →
`0049-app-driven-scoped-server-diagnostics.md`

`docs/adr/0069-client-crate-wasm-only-home.md`:

| Line | From                                             | To                                            |
| ---- | ------------------------------------------------ | --------------------------------------------- |
| 12   | `../0058-host-crate-layering.md`                 | `0058-host-crate-layering.md`                 |
| 29   | `../0056-web-canonical-colocated-leptos.md`      | `0056-web-canonical-colocated-leptos.md`      |
| 35   | `../0055-web-host-wasm-boundary-module-level.md` | `0055-web-host-wasm-boundary-module-level.md` |

- [x] **Step 4: Verify no dead links remain**

Run: `rg -n 'decisions/00' docs/ARCHITECTURE.md docs/DESIGN.md` Expected: no
matches (exit 1).

Run: `rg -n '\]\(\.\./00' docs/adr/` Expected: no matches (exit 1).

Run:
`ls docs/adr/0049-app-driven-scoped-server-diagnostics.md docs/adr/0058-host-crate-layering.md docs/adr/0056-web-canonical-colocated-leptos.md docs/adr/0055-web-host-wasm-boundary-module-level.md docs/adr/0002-frontend-framework.md storage/src`
Expected: all listed, no "No such file".

- [x] **Step 5: Commit**

Run `cargo xtask check` first (**`jaunder-commit`**); prettier may reflow the
Markdown, so re-check `git status --porcelain` and stage any rewrite.

```bash
git add docs/ARCHITECTURE.md docs/DESIGN.md docs/adr/0057-e2e-capture-dir-contract.md docs/adr/0069-client-crate-wasm-only-home.md
git commit -m "docs: repoint 19 dead relative links at their real targets"
```

---

## Task 2: The link scanner, and promote strips one path level

Scanner **and** promote's strip in one commit: `Link`'s fields are read by
`strip_one_level`, so shipping the struct alone would fail `dead_code` (Global
Constraints).

**Files:**

- Create: `xtask/src/doc_links.rs` (scanner + its `#[cfg(test)]` tests)
- Modify: `xtask/src/lib.rs:5-14` (add `mod doc_links;`)
- Modify: `xtask/src/adr.rs` — add `strip_one_level` near `rewrite_stem`
  (:47-49); call it in Pass B (:245-256); tests in the existing
  `#[cfg(test)] mod tests` (:286)

**Interfaces:**

- Consumes: nothing new.
- Produces:

```rust
// xtask/src/doc_links.rs

/// An inline Markdown link found outside code spans and fenced blocks. Carries a
/// byte range rather than a line number so the scanner computes only what its
/// callers read; `line_at` (Task 3) derives a line from `span.start` on demand.
pub struct Link {
    /// Byte range of the *target* within the source — the text between `](` and `)`.
    pub span: std::ops::Range<usize>,
    /// The target text.
    pub target: String,
}

/// Every inline `](target)` link in `body`, skipping fenced code blocks and inline
/// code spans (D10).
pub fn links_in(body: &str) -> Vec<Link>;

/// True when `target` is a relative path worth resolving — i.e. not a
/// `http:`/`https:`/`mailto:` URL and not a bare `#anchor`.
pub fn is_relative_target(target: &str) -> bool;
```

```rust
// xtask/src/adr.rs
/// Rewrite every inline link target in `body`, removing one leading `../`. A draft
/// moves up exactly one directory at promotion, so each relative target is off by
/// exactly one level (D1). Targets inside code spans and fenced blocks are left
/// alone (D2); so are `..`, `../`, and any target without a leading `../`.
pub fn strip_one_level(body: &str) -> String;
```

- [x] **Step 1: Write the failing tests**

In `xtask/src/doc_links.rs`:

````rust
// --- links_in: what counts as a link ---
#[test] fn finds_a_plain_inline_link() {
    let ls = links_in("see [x](a.md) here");
    assert_eq!(ls.len(), 1);
    assert_eq!(ls[0].target, "a.md");
}
#[test] fn finds_multiple_links_on_one_line() {
    let ls = links_in("[a](x.md) and [b](y.md)");
    assert_eq!(ls.len(), 2);
    assert_eq!(ls[1].target, "y.md");
}
#[test] fn span_covers_exactly_the_target() {
    let body = "see [x](a.md)";
    let l = &links_in(body)[0];
    assert_eq!(&body[l.span.clone()], "a.md");
}
#[test] fn skips_links_inside_a_fenced_block() {
    assert!(links_in("before\n```\n[x](a.md)\n```\nafter").is_empty());
}
#[test] fn skips_links_inside_a_tilde_fenced_block() {
    assert!(links_in("~~~\n[x](a.md)\n~~~\n").is_empty());
}
#[test] fn skips_links_inside_an_indented_fenced_block() {
    // CONTRIBUTING.md fences code inside list items; an unindented-only check
    // would leave those blocks live.
    assert!(links_in("- item:\n\n  ```\n  [x](a.md)\n  ```\n").is_empty());
}
#[test] fn skips_links_inside_an_inline_code_span() {
    assert!(links_in("write `[x](a.md)` like so").is_empty());
}
#[test] fn finds_a_link_after_a_fenced_block_closes() {
    let ls = links_in("```\n[a](x.md)\n```\n[b](y.md)");
    assert_eq!(ls.len(), 1);
    assert_eq!(ls[0].target, "y.md");
}

// --- is_relative_target ---
#[test] fn urls_and_anchors_are_not_relative_targets() {
    for t in ["https://e.com", "http://e.com", "mailto:a@b.c", "#section"] {
        assert!(!is_relative_target(t), "{t}");
    }
}
#[test] fn paths_are_relative_targets() {
    for t in ["a.md", "../a.md", "adr/", "a.md#frag"] {
        assert!(is_relative_target(t), "{t}");
    }
}
````

In `xtask/src/adr.rs`'s test module:

````rust
#[test] fn strip_one_level_drops_a_single_leading_parent() {
    assert_eq!(strip_one_level("[x](../0001-foo.md)"), "[x](0001-foo.md)");
}
#[test] fn strip_one_level_drops_only_one_of_two() {
    assert_eq!(
        strip_one_level("[x](../../CONTRIBUTING.md)"),
        "[x](../CONTRIBUTING.md)"
    );
}
#[test] fn strip_one_level_leaves_bare_targets_alone() {
    assert_eq!(strip_one_level("[x](template.md)"), "[x](template.md)");
}
#[test] fn strip_one_level_leaves_dot_dot_edge_cases_alone() {   // AC4
    assert_eq!(strip_one_level("[x](..)"), "[x](..)");
    assert_eq!(strip_one_level("[x](../)"), "[x](../)");
    assert_eq!(strip_one_level("[x](a/../b.md)"), "[x](a/../b.md)");
}
#[test] fn strip_one_level_ignores_urls_and_anchors() {
    let body = "[x](https://e.com/../a) [y](#s)";
    assert_eq!(strip_one_level(body), body);
}
#[test] fn strip_one_level_spares_links_inside_code() {          // AC3
    // Real `](...)` links, so this fails against any implementation that
    // rewrites targets without honouring the code carve-out.
    let body = "prose ../foo\n\n```\n[a](../x.md)\n```\n\n`[b](../y.md)`\n";
    assert_eq!(strip_one_level(body), body);
}
#[test] fn strip_one_level_rewrites_every_link_in_one_pass() {
    assert_eq!(
        strip_one_level("[a](../x.md) and [b](../y.md)"),
        "[a](x.md) and [b](y.md)"
    );
}

// End-to-end through promote:
#[test] fn promote_strips_one_level_from_sibling_links() {       // AC1
    let tmp = promote_repo("strip-sibling");
    write(&tmp, "docs/adr/drafts/d.md",
          "# ADR-DRAFT: D\n\nSee [foo](../0001-foo.md).\n");
    run_promote(&tmp).unwrap();
    let body = std::fs::read_to_string(tmp.join("docs/adr/0002-d.md")).unwrap();
    assert!(body.contains("](0001-foo.md)"), "body: {body}");
    assert!(!body.contains("](../0001-foo.md)"), "body: {body}");
    let _ = std::fs::remove_dir_all(&tmp);
}
#[test] fn promote_strips_one_level_from_non_adr_links() {       // AC2
    let tmp = promote_repo("strip-general");
    write(&tmp, "docs/adr/drafts/d.md",
          "# ADR-DRAFT: D\n\n[t](../template.md) [c](../../CONTRIBUTING.md)\n");
    run_promote(&tmp).unwrap();
    let body = std::fs::read_to_string(tmp.join("docs/adr/0002-d.md")).unwrap();
    assert!(body.contains("](template.md)"), "body: {body}");
    assert!(body.contains("](../CONTRIBUTING.md)"), "body: {body}");
    let _ = std::fs::remove_dir_all(&tmp);
}
````

- [x] **Step 2: Run the tests, verify they fail**

Run: `cargo test --manifest-path xtask/Cargo.toml` Expected: FAIL — `doc_links`
/ `links_in` / `strip_one_level` not defined.

- [x] **Step 3: Implement the scanner**

`links_in` scans `mask_code(body)` for `](`, takes bytes to the matching `)`,
and reports the target's byte span. `is_relative_target` rejects the three URL
schemes and a leading `#`. Both are pinned by Step 1's tests.

`mask_code` is written out because no assertion determines _how_ offsets survive
masking. Line-oriented, so an indented fence is recognized:

````rust
/// Blank out fenced blocks and inline code spans, replacing every non-newline byte
/// with a space. Length- and line-preserving, so byte offsets computed on the mask
/// are valid in the original.
///
/// UTF-8 safety: the only bytes inspected are `` ` ``, `~`, space, tab and `\n`,
/// none of which can occur inside a multi-byte character (continuation and lead
/// bytes are all >= 0x80). So byte-wise scanning never lands mid-character and the
/// `from_utf8` below cannot fail.
///
/// An unclosed fence masks to end of file. That yields false negatives (links after
/// it go unchecked), never false positives — the safe direction for a gate.
fn mask_code(body: &str) -> String {
    fn blank(out: &mut [u8], at: usize, line: &str) {
        for (k, b) in line.bytes().enumerate() {
            if b != b'\n' {
                out[at + k] = b' ';
            }
        }
    }
    /// Blank paired `` ` `` spans within a single line.
    fn blank_spans(out: &mut [u8], at: usize, line: &str) {
        let b = line.as_bytes();
        let mut i = 0;
        while i < b.len() {
            if b[i] == b'`' {
                let mut j = i + 1;
                while j < b.len() && b[j] != b'`' {
                    j += 1;
                }
                if j < b.len() {
                    for k in i..=j {
                        out[at + k] = b' ';
                    }
                    i = j + 1;
                    continue;
                }
            }
            i += 1;
        }
    }

    let mut out: Vec<u8> = body.bytes().collect();
    let mut fence: Option<&[u8]> = None;
    let mut at = 0usize;
    for line in body.split_inclusive('\n') {
        let trimmed = line.trim_start();
        let marker = [&b"```"[..], &b"~~~"[..]]
            .into_iter()
            .find(|m| trimmed.as_bytes().starts_with(m));
        match (fence, marker) {
            (None, Some(m)) => {
                fence = Some(m);
                blank(&mut out, at, line);
            }
            (Some(open), Some(m)) if open == m => {
                fence = None;
                blank(&mut out, at, line);
            }
            (Some(_), _) => blank(&mut out, at, line),
            (None, None) => blank_spans(&mut out, at, line),
        }
        at += line.len();
    }
    String::from_utf8(out).expect("masking preserves UTF-8 boundaries")
}
````

- [x] **Step 4: Implement `strip_one_level` and call it in Pass B**

Add to `adr.rs`, signature above. Every branch is pinned by Step 1's tests.
Build the output by walking `doc_links::links_in`'s spans left to right, copying
through the bytes between them — the spans are byte ranges into the original, so
one pass suffices. Strip only when the target starts with `../` **and** is
longer than `../`.

In Pass B (`adr.rs:245-256`), apply it where the body is written to its new
location — **D9** puts the strip here, not after Pass C:

```rust
let numbered = body.replace("ADR-DRAFT", &format!("ADR-{}", pad(*num)));
let relinked = strip_one_level(&numbered);
std::fs::write(repo.join(&new_rel), relinked)
    .with_context(|| format!("writing {new_rel}"))?;
```

Declare the module in `xtask/src/lib.rs` alongside `mod adr;` (:5-14):

```rust
mod doc_links;
```

- [x] **Step 5: Run the tests, verify they pass**

Run: `cargo test --manifest-path xtask/Cargo.toml` Expected: PASS — 19 new
tests, plus the pre-existing promote tests unchanged.

- [x] **Step 6: Commit**

```bash
git add xtask/src/doc_links.rs xtask/src/adr.rs xtask/src/lib.rs
git commit -m "fix(xtask): strip one path level from a promoted draft's links"
```

---

## Task 3: Add the gate logic and wire the `doc-links` step

Logic **and** wiring in one commit — unwired `problems`/`gated_files` in a
private module would fail `dead_code` (Global Constraints).

**Files:**

- Modify: `xtask/src/doc_links.rs` (add resolution + enumeration + tests)
- Create: `xtask/src/steps/doc_links.rs`
- Modify: `xtask/src/lib.rs:15-30` (add `pub mod doc_links;` to the `steps`
  block), `:294` and `:326` (call the step)
- Modify: `xtask/src/git.rs` (add `ls_files_md`, after `grep_files` at :159-169)

**Interfaces:**

- Consumes: `links_in`, `is_relative_target`, `Link` (Task 2);
  `xtask::git::lines` (`git.rs:88`);
  `xtask::result::{CommandResult, StepResult}`.
- Produces:

```rust
// xtask/src/doc_links.rs

/// Trees excluded from the gate. `docs/archive/` is a frozen record; specs and
/// plans under `docs/superpowers/` are transient. See the spec, Scope §3.
pub const EXCLUDED: &[&str] = &["docs/archive/", "docs/superpowers/"];

/// 1-based line containing byte `offset`.
pub fn line_at(body: &str, offset: usize) -> usize;

/// A dead relative link in one file.
pub struct DeadLink {
    pub line: usize,
    pub target: String,
}

/// Relative links in `repo`/`rel` whose target does not exist on disk. The shared
/// per-file unit — promote and the gate both reach link resolution through this
/// one function (D5, AC12).
pub fn dead_links_in(repo: &Path, rel: &str) -> Result<Vec<DeadLink>>;

/// Tracked `*.md` under `repo`, minus [`EXCLUDED`] and minus tracked-but-absent
/// paths (a staged deletion).
pub fn gated_files(repo: &Path) -> Result<Vec<String>>;

/// Every dead link across [`gated_files`], formatted `<file>:<line> -> <target>`.
pub fn problems(repo: &Path) -> Result<Vec<String>>;
```

```rust
// xtask/src/git.rs
/// `git ls-files -- *.md` — tracked Markdown, repo-relative. The pathspec glob
/// matches at any depth. (Narrower than the spec's `ls_files`: the gate only ever
/// wants Markdown, and filtering in git beats filtering in Rust.)
pub(crate) fn ls_files_md(dir: &Path) -> Result<Vec<String>>;
```

- [x] **Step 1: Write the failing tests**

In `xtask/src/doc_links.rs`'s test module. Three local helpers, defined once and
reused by every test below — do not copy-paste them per test:

- `repo(tag) -> PathBuf` — a temp git repo, following the `git.rs:204-216` idiom
  (`git init -q -b main`, then `user.email`/`user.name` config).
- `write(dir, rel, body)` — `create_dir_all` the parent, then write.
- `commit(dir, rel, body)` — `write`, then `git add <rel>`, then
  `git commit -qm c`.

````rust
#[test] fn line_at_is_one_based() {
    assert_eq!(line_at("a\nb\nc", 0), 1);
    assert_eq!(line_at("a\nb\nc", 2), 2);
    assert_eq!(line_at("a\nb\nc", 4), 3);
}

// --- dead_links_in ---
#[test] fn existing_target_is_not_dead() {
    let d = repo("alive");
    write(&d, "docs/a.md", "[x](b.md)\n");
    write(&d, "docs/b.md", "hi\n");
    assert!(dead_links_in(&d, "docs/a.md").unwrap().is_empty());
}
#[test] fn missing_target_is_dead_with_line_and_target() {
    let d = repo("dead");
    write(&d, "docs/a.md", "one\n[x](gone.md)\n");
    let found = dead_links_in(&d, "docs/a.md").unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].line, 2);
    assert_eq!(found[0].target, "gone.md");
}
#[test] fn directory_target_resolves() {          // AC9
    let d = repo("dir");
    std::fs::create_dir_all(d.join("docs/adr")).unwrap();
    write(&d, "docs/a.md", "[x](adr/)\n");
    assert!(dead_links_in(&d, "docs/a.md").unwrap().is_empty());
}
#[test] fn fragment_is_stripped_before_resolving() {
    let d = repo("frag");
    write(&d, "docs/a.md", "[x](b.md#sec)\n");
    write(&d, "docs/b.md", "hi\n");
    assert!(dead_links_in(&d, "docs/a.md").unwrap().is_empty());
}
#[test] fn urls_and_anchors_are_ignored() {       // AC8
    let d = repo("urls");
    write(&d, "docs/a.md", "[x](https://e.com) [y](#s) [z](mailto:a@b.c)\n");
    assert!(dead_links_in(&d, "docs/a.md").unwrap().is_empty());
}
#[test] fn dead_link_inside_code_is_ignored() {   // AC8
    let d = repo("code");
    write(&d, "docs/a.md", "`[x](gone.md)`\n\n```\n[y](gone.md)\n```\n");
    assert!(dead_links_in(&d, "docs/a.md").unwrap().is_empty());
}

// --- gated_files + problems ---
#[test] fn gate_reports_a_dead_link_in_tracked_markdown() {   // AC7
    let d = repo("gate");
    commit(&d, "docs/a.md", "[x](gone.md)\n");
    assert_eq!(problems(&d).unwrap(), vec!["docs/a.md:1 -> gone.md".to_string()]);
}
#[test] fn gate_skips_excluded_trees() {          // AC8
    let d = repo("excluded");
    commit(&d, "docs/archive/old.md", "[x](gone.md)\n");
    commit(&d, "docs/superpowers/plan.md", "[x](gone.md)\n");
    assert!(problems(&d).unwrap().is_empty());
}
#[test] fn gate_skips_untracked_files() {         // AC8
    let d = repo("untracked");
    write(&d, "docs/loose.md", "[x](gone.md)\n");   // never `git add`ed
    assert!(problems(&d).unwrap().is_empty());
}
#[test] fn gate_skips_tracked_but_deleted_files() {  // AC8
    let d = repo("deleted");
    commit(&d, "docs/a.md", "[x](gone.md)\n");
    std::fs::remove_file(d.join("docs/a.md")).unwrap();
    assert!(problems(&d).unwrap().is_empty());
}
#[test] fn gate_is_clean_when_every_link_resolves() {
    let d = repo("clean");
    commit(&d, "docs/b.md", "hi\n");
    commit(&d, "docs/a.md", "[x](b.md)\n");
    assert!(problems(&d).unwrap().is_empty());
}
````

- [x] **Step 2: Run the tests, verify they fail**

Run: `cargo test --manifest-path xtask/Cargo.toml doc_links` Expected: FAIL —
`problems` / `dead_links_in` / `line_at` not defined.

- [x] **Step 3: Implement**

Signatures above; the tests pin every branch. `line_at` counts newlines before
the offset (`body[..offset].matches('\n').count() + 1`). `dead_links_in` filters
`links_in` by `is_relative_target`, splits the `#fragment` off, and joins the
remainder to the file's parent directory. `gated_files` calls
`git::ls_files_md`, drops any path with an `EXCLUDED` prefix, and drops paths
absent from the worktree. `problems` maps `gated_files` through `dead_links_in`.

Add to `xtask/src/git.rs` beside `grep_files`:

```rust
/// `git ls-files -- *.md` — tracked Markdown, repo-relative. The pathspec glob
/// matches at any depth.
pub(crate) fn ls_files_md(dir: &Path) -> Result<Vec<String>> {
    lines(dir, &["ls-files", "--", "*.md"])
}
```

- [x] **Step 4: Run the tests, verify they pass**

Run: `cargo test --manifest-path xtask/Cargo.toml doc_links` Expected: PASS (12
new tests).

- [x] **Step 5: Add the step and wire it**

Create `xtask/src/steps/doc_links.rs`, mirroring `adr_check.rs:33-41`'s shape.
No `recovery:` line — a dead link has no mechanical fix (spec Scope §3):

```rust
//! `doc-links` — every relative Markdown link in the gated file set resolves on
//! disk. Read-only; resolution is a manual fix (the intended target is unknowable),
//! so unlike `adr-readme-parity` there is no recovery command.

use std::path::Path;

use crate::doc_links;
use crate::result::{CommandResult, StepResult};

pub fn run(result: &mut CommandResult) {
    result.push(match doc_links::problems(Path::new(".")) {
        Ok(problems) if problems.is_empty() => StepResult::ok("doc-links"),
        Ok(problems) => StepResult::fail("doc-links").detail(problems.join("\n")),
        Err(e) => StepResult::fail("doc-links").detail(format!("{e:#}")),
    });
}
```

In `xtask/src/lib.rs`: add `pub mod doc_links;` to the `steps` block (`:15-30`),
and add `steps::doc_links::run(&mut result);` immediately after
`steps::adr_check::run(&mut result);` in **both** arms — `:294` (Check) and
`:326` (Validate).

- [x] **Step 6: Verify the gate passes, and that it bites**

Run: `cargo xtask check --no-test` Expected: PASS, with a `doc-links` step
reported ok (**AC10**).

Then (**AC7**'s implementer note): break one link in a gated file, re-run,
confirm `doc-links` **fails** naming file, line and target — then restore it and
re-run green before committing. A gate never observed failing is not known to
bite.

- [x] **Step 7: Commit**

```bash
git add xtask/src/doc_links.rs xtask/src/steps/doc_links.rs xtask/src/lib.rs xtask/src/git.rs
git commit -m "feat(xtask): gate relative Markdown links with doc-links"
```

---

## Task 4: Promote warns on links still dead after Pass C

**Files:**

- Modify: `xtask/src/adr.rs` `run_promote` — after the README sync (:275-283)
- Test: `xtask/src/adr.rs` `#[cfg(test)] mod tests`

**Interfaces:**

- Consumes: `crate::doc_links::dead_links_in` (Task 3), `strip_one_level` (Task
  2).
- Produces: no new public API — `run_promote`'s summary gains a `warning:`
  clause of exactly this form, which the tests match on:

```
; warning: unresolved link(s) — docs/adr/0002-d.md: nonexistent.md
```

- [x] **Step 1: Write the failing tests**

> **Assert on the `warning:` clause, never on the whole summary.** Pass C
> already pushes `docs/adr/drafts/<slug>.md -> docs/adr/NNNN-<slug>.md` into the
> summary (`adr.rs:270`), so `summary.contains("drafts/aaa")` and
> `summary.contains("0002-d.md")` are **both true regardless of this task's
> code**. Assertions against them prove nothing in either direction.

```rust
#[test] fn promote_warns_on_a_surviving_dead_link() {            // AC5
    let tmp = promote_repo("warn");
    write(&tmp, "docs/adr/drafts/d.md",
          "# ADR-DRAFT: D\n\nSee [gone](nonexistent.md).\n");
    let summary = run_promote(&tmp).unwrap();                    // Ok, not Err
    assert!(tmp.join("docs/adr/0002-d.md").exists(), "file still written");
    // The full clause — discriminating, unlike a bare `contains("0002-d.md")`.
    assert!(
        summary.contains("warning: unresolved link(s) — docs/adr/0002-d.md: nonexistent.md"),
        "summary: {summary}"
    );
    let _ = std::fs::remove_dir_all(&tmp);
}
#[test] fn promote_is_silent_when_every_link_resolves() {
    let tmp = promote_repo("no-warn");
    write(&tmp, "docs/adr/drafts/d.md",
          "# ADR-DRAFT: D\n\nSee [foo](../0001-foo.md).\n");
    let summary = run_promote(&tmp).unwrap();
    assert!(!summary.contains("warning"), "summary: {summary}");
    let _ = std::fs::remove_dir_all(&tmp);
}
#[test] fn promote_checks_links_after_pass_c() {                 // AC6 — D9
    // `../drafts/aaa.md` is the cross-draft form that survives promotion (Task 5
    // documents it): Pass B strips it to `drafts/aaa.md`, Pass C rewrites that to
    // `0002-aaa.md`, which RESOLVES from docs/adr/. So a correctly-ordered check
    // finds nothing.
    //
    // This is the discriminating shape. Run the check before Pass C and the target
    // is still `drafts/aaa.md` — pointing at the draft Pass B already deleted — so
    // a premature check emits a warning and this test fails.
    let tmp = promote_repo("order");
    write(&tmp, "docs/adr/drafts/aaa.md", "# ADR-DRAFT: Aaa\n");
    write(&tmp, "docs/adr/drafts/bbb.md",
          "# ADR-DRAFT: Bbb\n\nBuilds on [aaa](../drafts/aaa.md).\n");
    let summary = run_promote(&tmp).unwrap();
    assert!(!summary.contains("warning"), "premature check: {summary}");
    let bbb = std::fs::read_to_string(tmp.join("docs/adr/0003-bbb.md")).unwrap();
    assert!(bbb.contains("](0002-aaa.md)"), "bbb: {bbb}");
    let _ = std::fs::remove_dir_all(&tmp);
}
```

- [x] **Step 2: Run the tests, verify they fail**

Run: `cargo test --manifest-path xtask/Cargo.toml adr::` Expected: FAIL — no
`warning:` clause in the summary.

- [x] **Step 3: Implement**

In `run_promote`, after the `table_note` block (`adr.rs:275-281`) and before the
final `Ok(format!(...))`:

```rust
// D4/D9: check the graduated files in their FINAL form — after Pass C's reference
// rewrite and the README sync — so a link Pass C is about to fix is never
// reported. Warn only: the tree is already mutated and staged, and `doc-links`
// turns this into a hard failure on a stable, re-runnable tree.
let mut warnings = Vec::new();
for (_slug, _num, new_name) in &assigned {
    let rel = format!("{ADR_DIR}/{new_name}");
    let dead = crate::doc_links::dead_links_in(repo, &rel)?;
    if !dead.is_empty() {
        let targets: Vec<String> = dead.into_iter().map(|d| d.target).collect();
        warnings.push(format!("{rel}: {}", targets.join(", ")));
    }
}
let warn_note = if warnings.is_empty() {
    String::new()
} else {
    format!("; warning: unresolved link(s) — {}", warnings.join("; "))
};
```

Append `{warn_note}` to the returned summary format string.

- [x] **Step 4: Run the tests, verify they pass**

Run: `cargo test --manifest-path xtask/Cargo.toml adr::` Expected: PASS.

- [x] **Step 5: Commit**

```bash
git add xtask/src/adr.rs
git commit -m "feat(xtask): warn when a promoted ADR still has dead links"
```

---

## Task 5: Document the gate and the draft link rules

**Files:**

- Modify: `CONTRIBUTING.md` (the verify-ladder bullet list, :237-248)
- Modify: `docs/adr/drafts/README.md` (authoring rules :17-24; "Gate
  invisibility" :44-49)
- Test: none — verified by `doc-links` itself. Both files are gated, so a
  backticked example the scanner mishandled would fail the gate (**D10**).

**Interfaces:**

- Consumes: the `doc-links` step name and `EXCLUDED` list (Task 3).
- Produces: nothing consumed by later tasks.

- [ ] **Step 1: Add the `CONTRIBUTING.md` bullet**

After the `prettier` bullet at `:240-242`. Name both exclusions and say why the
list differs from `.prettierignore`'s — neither can be derived from the other
(spec Scope §3):

```markdown
- `doc-links` checks that every relative Markdown link in tracked `*.md`
  resolves on disk. It excludes `docs/archive/` (a frozen record — its links are
  dead because the docs moved on) and `docs/superpowers/` (transient specs and
  plans, which may link files they will create). Note this is a **different**
  list from `.prettierignore`'s, which excludes only `docs/archive/`; the two
  are maintained separately on purpose.
```

- [ ] **Step 2: Add the sibling-ADR and cross-draft link rules**

In `docs/adr/drafts/README.md`, add after the existing rule 3 (:22-24). Rule 5
is the workflow landmine from "Key risks" — without it, following rule 3 for a
cross-draft reference produces an ADR that hard-fails `doc-links` at ship:

```markdown
4. Link sibling ADRs **as if the draft already lived in `docs/adr/`** —
   `[ADR-0061](0061-web-keyed-list-reactive-store.md)`, not
   `../0061-web-keyed-list-reactive-store.md`. `promote` moves the file up one
   directory and strips one `../` level from every link target, so the bare form
   is what survives. (The `../template.md` link in step 1 above is correct
   _here_ — this README is never promoted.)
5. Link **another draft** as `[Aaa](../drafts/aaa.md)`. Promotion strips one
   level to `drafts/aaa.md`, which `promote` then rewrites to the number it
   assigned. Do **not** use the rule-3 repo-root form (`docs/adr/drafts/aaa.md`)
   in a markdown link from one draft to another: it becomes
   `docs/adr/NNNN-aaa.md`, which is dead from inside `docs/adr/` and will fail
   the `doc-links` gate. Rule 3 still applies to references from code and prose
   _outside_ `docs/adr/`.
```

- [ ] **Step 3: Amend the "Gate invisibility" section**

`docs/adr/drafts/README.md:44-49` claims all the ADR gates share one `read_dir`
enumeration. `doc-links` does not. Replace the paragraph with:

```markdown
The `identifier-collisions`, `adr-format`, and `adr-readme-parity` gates share
one enumeration rule — `is_file` → `.md` → leading number, applied by a
non-recursive `read_dir` over `docs/adr/`. A numberless draft in this
subdirectory is excluded twice over, so drafts never trip a gate.

`doc-links` enumerates differently — tracked files, via `git ls-files`.
Everything here except this `README.md` is gitignored, so drafts stay invisible
to it too, by a stronger rule: an uncommitted draft is not a tracked file.
```

- [ ] **Step 4: Verify**

Run: `cargo xtask check --no-test` Expected: PASS, `doc-links` ok — proving the
backticked link examples added above are correctly skipped as code spans
(**D10**).

- [ ] **Step 5: Commit**

Prettier will reflow these Markdown edits during the gate; re-check
`git status --porcelain` and stage the rewrite.

```bash
git add CONTRIBUTING.md docs/adr/drafts/README.md
git commit -m "docs: record the doc-links gate and the draft link rules"
```

---

## Final verification

- [ ] Run `cargo xtask validate --no-e2e` — the full local gate short of e2e. No
      runtime code path is touched, so no e2e combo is affected; the matrix is
      left to CI (ADR-0034). This also satisfies **AC13** — `validate` runs
      `steps::host_tests` → `xtask-tests`, which executes every test above.
- [ ] **AC11 — no new CLI surface.** Run `cargo xtask --help`; confirm the
      subcommand list is unchanged from `main` (no `doc-links` entry), and that
      `doc-links` appears only as a step name in `cargo xtask check` output.
- [ ] **AC12 — one implementation.** Run `rg -n 'dead_links_in' xtask/src`.
      Expect exactly one definition, in `xtask/src/doc_links.rs`; exactly two
      non-test call sites — one in `xtask/src/adr.rs` (promote) and one inside
      `doc_links.rs` itself (from `problems`, which the step calls); plus the
      `#[cfg(test)]` calls. Then run
      `rg -n 'fn links_in|\.exists\(\)' xtask/src` and confirm link parsing and
      path resolution appear **only** in `xtask/src/doc_links.rs` — no second
      resolver.
- [ ] Confirm `git status --porcelain` is clean.
