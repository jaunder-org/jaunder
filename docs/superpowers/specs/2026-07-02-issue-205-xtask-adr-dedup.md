# Spec — issue #205: xtask ADR-table dedup & naming polish

## Summary

Four pure refactors in `xtask/src/adr_readme.rs` (one touching `xtask/src/adr.rs`
only by association). Deferred polish from the #196 code review; **no behavior
change**. `xtask/` is outside the coverage gate, so the guardrails are the host
unit suite (`xtask/src/adr_readme.rs` `#[cfg(test)]`) plus clippy — every existing
test must stay green, and no new behavior is introduced that would need a new test.

Single small PR. No ADR (no architectural decision; a pure internal refactor).
No `CONTEXT.md`/glossary impact.

## Motivation

The #196 review found three same-shaped directory walks, a duplicated marker
lookup, a primitive-obsession tuple, and a pair of near-identical heading arms.
None affects correctness; all raise the odds of a future edit drifting one copy
out of sync with another. Consolidating gives each rule one home.

## Resolved decisions

### R1 — Shared ADR directory walk (traversal rule, content-free)

Two walkers duplicate the rule
`read_dir → is_file → ends_with(".md") → ids::leading_number`:

- `parse_adr_dir` (propagates IO errors via `?`, returns `Result<Vec<AdrEntry>>`)
- `format_problems` (converts IO errors into problem strings, returns `Vec<String>`)

Their **error handling deliberately differs**, so the shared primitive centralizes
only the *traversal rule*, not the content read:

```rust
/// A directory entry that qualifies as an ADR: a regular `*.md` file whose name
/// carries a leading number. Content is intentionally not read here — callers
/// read it themselves so each keeps its own IO-error policy.
struct AdrFile {
    num: u32,
    filename: String,
    path: PathBuf,
}

/// The qualifying ADR files under `repo/docs/adr`, unsorted. The single home of
/// the "what counts as an ADR file" rule. Propagates the `read_dir` error.
fn adr_files(repo: &Path) -> Result<Vec<AdrFile>> { … }
```

- `parse_adr_dir` becomes: `for f in adr_files(repo)? { let content =
  read_to_string(&f.path).with_context(…)?; entries.push(AdrEntry { … }) }`, then
  the existing `sort_by_key`.
- `format_problems` becomes: `match adr_files(repo) { Ok(files) => for f in files {
  match read_to_string(&f.path) { Ok(c) => problems.extend(file_format_problems(&f.filename, f.num, &c)), Err(e) => problems.push(…) } }, Err(e) => return vec![format!("cannot read …")] }`,
  then the existing `problems.sort()`.

Both callers' **observable behavior is unchanged**: same files selected, same sort,
same per-file and directory-level error strings.

**`adr_filenames` in `adr.rs` is left as-is.** It is a genuinely *looser* walk
(all regular files, no `.md`/number filter — `run_renumber` applies
`ids::leading_number` at the call site) and does not share the `AdrFile` rule.
Routing it through `adr_files` would require weakening the primitive or adding a
filter flag, which is not worth it.

### R2 — Shared marker lookup

`splice_block` and `extract_block` both locate the `BEGIN`/`END` markers with the
same `find` + `with_context` + order check. Extract:

```rust
/// The byte range strictly between the ADR-table markers: `(start, end)` where
/// `start` is just past `BEGIN` and `end` is at `END`. Errors when either marker
/// is missing or they are out of order.
fn marker_bounds(readme: &str) -> Result<(usize, usize)> {
    let begin = readme.find(BEGIN).with_context(|| format!("{README} is missing the `{BEGIN}` marker"))?;
    let end = readme.find(END).with_context(|| format!("{README} is missing the `{END}` marker"))?;
    anyhow::ensure!(begin < end, "{README} adr-table markers are out of order");
    Ok((begin + BEGIN.len(), end))
}
```

- `splice_block` uses `(after_begin, end)` from `marker_bounds`: unchanged output
  (`format!("{}\n\n{}\n\n{}", &readme[..after_begin], new_block, &readme[end..])`).
- `extract_block` returns `readme[start..end].to_string()`.

The order check unifies on strict `<` (was `<` in `splice_block`, `<=` in
`extract_block`). Since `BEGIN != END` and both are found, `begin == end` is
impossible, so strict `<` preserves behavior for every real input while keeping
one rule. The existing `splice_block_errors_on_missing_marker` test still passes;
error messages are byte-identical.

### R3 — Reuse `TableRow`, drop `Cells` and `current_cells`

`type Cells = (u32, String, String, String)` mirrors `TableRow`'s fields exactly
(`num, target, title, status`). Reuse `TableRow`:

- Add `#[derive(PartialEq, Eq)]` (and `Clone` if needed for the returned `Vec`) to
  `TableRow`.
- `resolved_rows` returns `Vec<TableRow>` (build `TableRow { num: e.num, target:
  format!("adr/{}", e.filename), title, status: e.status.clone() }`). **Keep the
  existing internal `sort_by_key(|e| e.num)`** — the function still sorts ascending
  before mapping; only the element type changes from the tuple to `TableRow`.
- `render_block` iterates fields by name:
  `for r in resolved_rows(entries, existing) { out.push_str(&format!("| [{:04}]({}) | {} | {} |\n", r.num, r.target, r.title, r.status)); }`.
- Delete the `Cells` type alias and the `current_cells` helper. `sync_readme_at`'s
  idempotence check becomes `if resolved_rows(&entries, &existing) == existing`
  (comparing `Vec<TableRow> == Vec<TableRow>` via the new derive).
- Update the `desired_matches_current_when_in_sync` test to compare
  `resolved_rows(&entries, &existing)` against `existing` directly (the assertion's
  intent is unchanged; only the RHS spelling changes since `current_cells` is gone).

Semantics preserved: `resolved_rows` still computes `target = adr/{filename}`; the
"in sync" case is exactly when the committed `target`/`title`/`status` already
equal the resolved values — identical to the old tuple comparison.

### R4 — Fold the heading-prefix branches in `heading_title`

Replace the two `split_once` arms with a loop over a small table:

```rust
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
```

This reproduces **both** original arms exactly:

- `"ADR-"`/`": "` arm — `!lhs.is_empty()` is redundant (an `ADR-`-prefixed `lhs`
  is never empty) and harmless; `lhs[4..].all(digit)` matches the original.
- `""`/`". "` arm — `starts_with("")` is always true (no constraint, as before);
  `!lhs.is_empty()` and `lhs.all(digit)` match the original's guard.

The existing `heading_title_strips_canonical_and_legacy_prefixes` test still passes.

## Acceptance criteria

1. **AC1** — `adr_readme.rs` has exactly one directory-traversal rule
   (`adr_files`); `parse_adr_dir` and `format_problems` both consume it and no
   longer inline the `read_dir`/`is_file`/`.md`/`leading_number` sequence.
   `adr_filenames` in `adr.rs` is unchanged.
2. **AC2** — `splice_block` and `extract_block` both obtain their marker positions
   from a single `marker_bounds` helper; the `find`/`with_context`/order-check
   sequence appears once.
3. **AC3** — The `Cells` type alias and `current_cells` helper are gone;
   `resolved_rows` returns `Vec<TableRow>`; `render_block`, `sync_readme_at`, and
   the affected test refer to `TableRow` fields by name.
4. **AC4** — `heading_title` has a single folded prefix-matching loop; no separate
   `": "`/`". "` branches remain.
5. **AC5** — `cargo xtask check` is green: clippy clean, all `xtask` unit tests
   pass **unmodified in intent** (only R3's test RHS spelling changes to follow the
   removed `current_cells`).
6. **AC6** — No public API of `adr_readme.rs` changes its externally-observable
   behavior: `parse_adr_dir`, `format_problems`, `splice_block`, `render_block`,
   `sync_readme_at`, `parity_report`, `readme_has_markers` all produce identical
   outputs/errors for identical inputs.

## Out of scope

- Any change to `adr_filenames` / `run_renumber` in `adr.rs` (R1 leaves it alone).
- Any change to the ADR file format, README table format, or the gates' semantics.
- Adding new tests (no new behavior); existing tests stay green.
