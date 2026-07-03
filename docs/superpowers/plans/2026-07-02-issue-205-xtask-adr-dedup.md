# xtask ADR-table dedup & naming polish — Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with `jaunder-iterate`
> (delegating individual tasks to a subagent via `jaunder-dispatch` when
> useful). Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Four pure refactors in `xtask/src/adr_readme.rs` that give each
duplicated rule one home, with no behavior change.

**Architecture:** All four are internal to `xtask/src/adr_readme.rs`. R1
extracts one directory-walk primitive; R2 extracts one marker-lookup helper; R3
replaces a tuple alias with the existing `TableRow` struct; R4 folds two
heading-prefix branches into a table-driven loop. `xtask/src/adr.rs` is read but
not modified (`adr_filenames` is deliberately left alone — it is a looser walk).

**Tech Stack:** Rust, `anyhow`, `cargo xtask` (host-only dev driver).

## Global Constraints

- **No behavior change.** Every public fn of `adr_readme.rs` must produce
  identical outputs/errors for identical inputs. The `#[cfg(test)]` suite in
  `adr_readme.rs` is the safety net — it must stay green with only the one RHS
  edit R3 forces (Task 3, Step 3).
- **`xtask/` is outside the coverage gate** — guardrails are the host unit
  suite + clippy. No new tests are added (no new behavior).
- **Per-task gate:** full `cargo xtask check` (fmt + clippy + host tests;
  auto-heals formatting). Run it before each commit so the pre-commit hook
  passes clean (`jaunder-commit`).
- **One clean commit per task. No `Co-Authored-By` trailer.**
- **Do not modify `xtask/src/adr.rs`** — `adr_filenames`/`run_renumber` stay
  as-is.

---

### Task 1: R1 — one ADR directory-walk primitive (`adr_files`)

**Files:**

- Modify: `xtask/src/adr_readme.rs` (imports line 12; `parse_adr_dir` 109-135;
  `format_problems` 314-339; add `AdrFile` + `adr_files` near the top of the
  file, e.g. just above `heading_title` at line 51)

**Interfaces:**

- Consumes: `crate::ids::leading_number`, `ADR_DIR`.
- Produces: `struct AdrFile { num: u32, filename: String, path: PathBuf }`
  (private) and `fn adr_files(repo: &Path) -> Result<Vec<AdrFile>>` (private).
  No later task depends on these.

**Behavior-preservation note (record, not a blocker):** the `read_dir` error is
returned _unwrapped_ so `parse_adr_dir` and `format_problems` each phrase their
own existing context string. The realistic path (directory missing) is
byte-identical for both. The _only_ non-identical path is a per-`DirEntry`
metadata error raised _mid-iteration_ (an already-opened directory whose entry
can't be stat'd): today `parse_adr_dir` propagates it bare and `format_problems`
silently skips it via `.flatten()`; after this change both surface it through
`adr_files`. This path is effectively unreachable in the repo and is not under
test; it is noted here so a reviewer isn't surprised.

- [x] **Step 1: Extend the `std::path` import**

Change line 12 from:

```rust
use std::path::Path;
```

to:

```rust
use std::path::{Path, PathBuf};
```

- [x] **Step 2: Add the `AdrFile` struct and `adr_files` primitive**

Insert immediately before `fn heading_title` (currently line 51):

```rust
/// A directory entry that qualifies as an ADR: a regular `*.md` file whose name
/// carries a leading number. Content is intentionally not read here so each
/// caller keeps its own IO-error policy.
struct AdrFile {
    num: u32,
    filename: String,
    path: PathBuf,
}

/// The qualifying ADR files under `repo/docs/adr`, unsorted — the single home of
/// the "what counts as an ADR file" rule (`is_file` → `.md` → leading number).
/// The `read_dir` error is returned unwrapped so callers phrase their own context
/// (`parse_adr_dir` wants "reading <dir>", `format_problems` wants
/// "cannot read <dir>").
fn adr_files(repo: &Path) -> Result<Vec<AdrFile>> {
    let mut files = Vec::new();
    for ent in std::fs::read_dir(repo.join(ADR_DIR))? {
        let ent = ent?;
        if !ent.file_type().map(|t| t.is_file()).unwrap_or(false) {
            continue;
        }
        let filename = ent.file_name().to_string_lossy().into_owned();
        if !filename.ends_with(".md") {
            continue;
        }
        let Some(num) = ids::leading_number(&filename) else {
            continue;
        };
        files.push(AdrFile {
            num,
            filename,
            path: ent.path(),
        });
    }
    Ok(files)
}
```

- [x] **Step 3: Rewrite `parse_adr_dir` to consume `adr_files`**

Replace the whole `parse_adr_dir` body (lines 109-135) with:

```rust
/// Parse the ADR files under `repo/docs/adr`, sorted ascending by number.
pub fn parse_adr_dir(repo: &Path) -> Result<Vec<AdrEntry>> {
    let dir = repo.join(ADR_DIR);
    let mut entries = Vec::new();
    for f in adr_files(repo).with_context(|| format!("reading {}", dir.display()))? {
        let content = std::fs::read_to_string(&f.path)
            .with_context(|| format!("reading {}", f.path.display()))?;
        entries.push(AdrEntry {
            num: f.num,
            filename: f.filename,
            title: heading_title(&content),
            status: status_token(&content),
        });
    }
    entries.sort_by_key(|e| e.num);
    Ok(entries)
}
```

- [x] **Step 4: Rewrite `format_problems` to consume `adr_files`**

Replace the whole `format_problems` body (lines 314-339) with:

```rust
/// Every ADR file's `adr-format` problems, sorted for stable output. A directory
/// read error is surfaced as a single problem rather than a panic.
pub fn format_problems(repo: &Path) -> Vec<String> {
    let files = match adr_files(repo) {
        Ok(f) => f,
        Err(e) => return vec![format!("cannot read {}: {e}", repo.join(ADR_DIR).display())],
    };
    let mut problems = Vec::new();
    for f in files {
        match std::fs::read_to_string(&f.path) {
            Ok(content) => problems.extend(file_format_problems(&f.filename, f.num, &content)),
            Err(e) => problems.push(format!("{}: cannot read ({e})", f.filename)),
        }
    }
    problems.sort();
    problems
}
```

- [x] **Step 5: Run the gate, verify green**

Run: `cargo xtask check` Expected: PASS — clippy clean, all `xtask` unit tests
pass (esp. `parse_adr_dir_reads_sorts_and_skips_non_adrs`,
`file_format_problems_flags_each_violation`,
`gates_ignore_docs_adr_template_md`).

- [x] **Step 6: Commit**

```bash
git add xtask/src/adr_readme.rs
git commit -m "refactor(xtask): one ADR directory-walk primitive (adr_files)"
```

---

### Task 2: R2 — one marker-lookup helper (`marker_bounds`)

**Files:**

- Modify: `xtask/src/adr_readme.rs` (`splice_block` 155-170; `extract_block`
  173-183; add `marker_bounds` just above `splice_block`)

**Interfaces:**

- Consumes: `BEGIN`, `END`, `README`.
- Produces: `fn marker_bounds(readme: &str) -> Result<(usize, usize)>`
  (private), returning `(start, end)` =
  `(begin + BEGIN.len(), end_marker_offset)`.

- [x] **Step 1: Add `marker_bounds` above `splice_block`**

Insert immediately before `pub fn splice_block` (currently line 155, above its
doc comment):

```rust
/// The byte range strictly between the ADR-table markers: `(start, end)` where
/// `start` is just past `BEGIN` and `end` is at `END`. Errors when either marker
/// is missing or they are out of order.
fn marker_bounds(readme: &str) -> Result<(usize, usize)> {
    let begin = readme
        .find(BEGIN)
        .with_context(|| format!("{README} is missing the `{BEGIN}` marker"))?;
    let end = readme
        .find(END)
        .with_context(|| format!("{README} is missing the `{END}` marker"))?;
    anyhow::ensure!(begin < end, "{README} adr-table markers are out of order");
    Ok((begin + BEGIN.len(), end))
}
```

- [x] **Step 2: Rewrite `splice_block` to use `marker_bounds`**

Replace the whole `splice_block` body (lines 155-170, keeping its existing doc
comment) with:

```rust
/// Replace the text strictly between the markers with `new_block`. Errors when a
/// marker is missing or out of order.
pub fn splice_block(readme: &str, new_block: &str) -> Result<String> {
    let (after_begin, end) = marker_bounds(readme)?;
    Ok(format!(
        "{}\n\n{}\n\n{}",
        &readme[..after_begin],
        new_block,
        &readme[end..]
    ))
}
```

- [x] **Step 3: Rewrite `extract_block` to use `marker_bounds`**

Replace the whole `extract_block` body (lines 173-183) with:

```rust
/// The block text between the markers (for reading existing titles).
fn extract_block(readme: &str) -> Result<String> {
    let (start, end) = marker_bounds(readme)?;
    Ok(readme[start..end].to_string())
}
```

(`extract_block`'s order check was `<=`; `marker_bounds` uses strict `<`. Since
`BEGIN`/`END` are distinct, mutually non-substring markers, both `find`s
succeeding implies distinct offsets, so `begin == end` is impossible and no real
input's behavior changes.)

- [x] **Step 4: Run the gate, verify green**

Run: `cargo xtask check` Expected: PASS — esp.
`splice_block_replaces_only_between_markers` and
`splice_block_errors_on_missing_marker` (error still contains `"marker"`).

- [x] **Step 5: Commit**

```bash
git add xtask/src/adr_readme.rs
git commit -m "refactor(xtask): one ADR-table marker-lookup helper (marker_bounds)"
```

---

### Task 3: R3 — reuse `TableRow`, drop `Cells` and `current_cells`

**Files:**

- Modify: `xtask/src/adr_readme.rs` (`TableRow` 43-49; `render_block` 145-151;
  delete `Cells` 185-187 and `current_cells` 217-222; `resolved_rows` 194-215;
  `sync_readme_at` 234-235; test `desired_matches_current_when_in_sync` ~512)

**Interfaces:**

- Consumes: `AdrEntry`, `BTreeMap`.
- Produces:
  `resolved_rows(entries: &[AdrEntry], existing: &[TableRow]) -> Vec<TableRow>`
  (was `Vec<Cells>`). `TableRow` gains `#[derive(Debug, PartialEq, Eq)]`.

- [x] **Step 1: Derive `Debug, PartialEq, Eq` on `TableRow`**

Change lines 43-44 from:

```rust
/// A parsed committed table row. Cells are trimmed (padding-proof).
pub struct TableRow {
```

to:

```rust
/// A parsed committed table row. Cells are trimmed (padding-proof).
#[derive(Debug, PartialEq, Eq)]
pub struct TableRow {
```

(`Debug` is needed because the test uses `assert_eq!` on `Vec<TableRow>`;
`PartialEq`/`Eq` power the `desired == existing` idempotence check.)

- [x] **Step 2: Change `resolved_rows` to build `TableRow`, delete `Cells` +
      `current_cells`**

First, delete the `Cells` type alias (lines 185-187, including its doc comment):

```rust
/// A row's mechanical cells (number, link target, status) plus its
/// preserved-or-seeded title: `(num, target, title, status)`.
type Cells = (u32, String, String, String);
```

Then replace `resolved_rows` (lines 194-215) with (keeping its doc comment):

```rust
/// The desired table rows, ascending by number, applying the title-preservation
/// rule once: reuse an existing row's title when a row with that number exists,
/// else seed it from the ADR heading. The single source of that rule — both the
/// renderer ([`render_block`]) and the idempotence check ([`sync_readme_at`])
/// consume it, so they can never disagree.
fn resolved_rows(entries: &[AdrEntry], existing: &[TableRow]) -> Vec<TableRow> {
    let title_by_num: BTreeMap<u32, &str> =
        existing.iter().map(|r| (r.num, r.title.as_str())).collect();
    let mut sorted: Vec<&AdrEntry> = entries.iter().collect();
    sorted.sort_by_key(|e| e.num);
    sorted
        .into_iter()
        .map(|e| {
            let title = title_by_num
                .get(&e.num)
                .copied()
                .unwrap_or(e.title.as_str())
                .to_string();
            TableRow {
                num: e.num,
                target: format!("adr/{}", e.filename),
                title,
                status: e.status.clone(),
            }
        })
        .collect()
}
```

Then delete `current_cells` entirely (lines 217-222):

```rust
fn current_cells(existing: &[TableRow]) -> Vec<Cells> {
    existing
        .iter()
        .map(|r| (r.num, r.target.clone(), r.title.clone(), r.status.clone()))
        .collect()
}
```

- [x] **Step 3: Update `render_block` and `sync_readme_at`, and the one test**

Replace `render_block`'s loop (lines 147-149) so it reads `TableRow` fields.
Replace the whole `render_block` body (145-151) with:

```rust
/// The generated table block: header + separator + one row per ADR entry
/// (ascending), reusing an existing row's title when present, else seeding the
/// title from the ADR heading. Single-space padded — prettier owns alignment.
pub fn render_block(entries: &[AdrEntry], existing: &[TableRow]) -> String {
    let mut out = String::from("| #   | Title | Status |\n| --- | ----- | ------ |\n");
    for r in resolved_rows(entries, existing) {
        out.push_str(&format!(
            "| [{:04}]({}) | {} | {} |\n",
            r.num, r.target, r.title, r.status
        ));
    }
    out.trim_end().to_string()
}
```

In `sync_readme_at`, change lines 234-235 from:

```rust
    let desired = resolved_rows(&entries, &existing);
    if desired == current_cells(&existing) {
```

to:

```rust
    let desired = resolved_rows(&entries, &existing);
    if desired == existing {
```

In the test `desired_matches_current_when_in_sync` (~line 512), change:

```rust
        assert_eq!(resolved_rows(&entries, &existing), current_cells(&existing));
```

to:

```rust
        assert_eq!(resolved_rows(&entries, &existing), existing);
```

- [x] **Step 4: Run the gate, verify green**

Run: `cargo xtask check` Expected: PASS — esp.
`desired_matches_current_when_in_sync`,
`render_block_preserves_existing_title_and_seeds_new_from_heading`,
`render_block_drops_orphans_and_sorts_ascending`. Confirm no leftover reference
to `Cells` or `current_cells` (a stale reference is a compile error).

- [x] **Step 5: Commit**

```bash
git add xtask/src/adr_readme.rs
git commit -m "refactor(xtask): reuse TableRow for resolved rows, drop Cells tuple"
```

---

### Task 4: R4 — fold `heading_title`'s prefix branches

**Files:**

- Modify: `xtask/src/adr_readme.rs` (`heading_title` 53-67)

**Interfaces:** none new — `heading_title` keeps its signature.

- [ ] **Step 1: Replace the two branches with a table-driven loop**

Replace the whole `heading_title` body (lines 53-67, keeping its doc comment)
with:

```rust
fn heading_title(content: &str) -> String {
    let line = content.lines().find(|l| l.starts_with("# ")).unwrap_or("");
    let after = line.trim_start_matches("# ").trim();
    for (prefix, sep) in [("ADR-", ": "), ("", ". ")] {
        if let Some((lhs, title)) = after.split_once(sep) {
            if lhs.starts_with(prefix)
                && !lhs.is_empty()
                && lhs[prefix.len()..].chars().all(|c| c.is_ascii_digit())
            {
                return title.trim().to_string();
            }
        }
    }
    after.to_string()
}
```

(Reproduces both arms bit-for-bit: `!lhs.is_empty()` is redundant-but-harmless
for the `"ADR-"` row and load-bearing for the `""` legacy row; the table order
keeps the `": "`-before-`". "` first-match precedence.)

- [ ] **Step 2: Run the gate, verify green**

Run: `cargo xtask check` Expected: PASS — esp.
`heading_title_strips_canonical_and_legacy_prefixes`.

- [ ] **Step 3: Commit**

```bash
git add xtask/src/adr_readme.rs
git commit -m "refactor(xtask): fold heading_title prefix branches into a loop"
```

---

## Self-Review

**Spec coverage:**

- R1 → Task 1 (AC1). R2 → Task 2 (AC2). R3 → Task 3 (AC3). R4 → Task 4 (AC4).
- AC5 (`cargo xtask check` green) → each task's gate step.
- AC6 (no observable behavior change) → the Global Constraints + Task 1's
  behavior-preservation note; the existing unit suite is the running proof.
- Out-of-scope (leave `adr_filenames`/`adr.rs`, no new tests, no format changes)
  → Global Constraints + Task 1 note; no task touches `adr.rs`.

**Placeholder scan:** none — every code step shows full code; no TBD/TODO.

**Type consistency:** `AdrFile { num, filename, path }` defined in Task 1 and
used only there. `resolved_rows -> Vec<TableRow>` (Task 3) is consumed by
`render_block` and `sync_readme_at` in the same task.
`marker_bounds -> (usize, usize)` (Task 2) consumed by
`splice_block`/`extract_block` in the same task. `TableRow` derive (Task 3) is
what the Task 3 test edit relies on. No cross-task name drift.
