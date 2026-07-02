# Generate the `docs/README.md` ADR table from `docs/adr/` (#196) — Plan

**Spec:** `docs/superpowers/specs/2026-07-02-issue-196-adr-readme-sync.md`
**Issue:** jaunder-org/jaunder#196
**ADR:** addendum to **ADR-0036** (no new number).

**Goal:** Make the README ADR table a generated projection of `docs/adr/` for its
mechanical cells (number, link, status), keep curated titles hand-owned, fold
regeneration into `adr renumber`, and gate both ADR-file style conformance and
table/directory parity.

## Global constraints

- **No Co-Authored-By trailers.**
- **Per-commit gate is git-enforced** (pre-commit runs a single full
  `cargo xtask check`). Run it from **inside the worktree**
  (`.claude/worktrees/issue-196-adr-readme-sync`), not the main repo.
- **xtask is outside the coverage gate** — the flake excludes `xtask/`. So the
  Rust here incurs **no coverage-baseline churn**; its safety net is the host
  xtask unit suite (`host_tests` step) + clippy. Write unit tests for every new
  pure function.
- **Dead-code boundary** (project lesson): a new `pub` item in a private xtask
  module is `-D dead_code` until consumed. Each commit must wire in what it adds —
  never a standalone "add module" commit.
- **Prettier owns the table's column padding.** The parity gate compares
  **semantically** (trimmed cells), and `check` runs prettier (Fix) *before* the
  ADR checks in the pipeline, so `sync-readme`'s raw single-space output is healed
  in the same run. `sync-readme` need not reproduce prettier's spacing.
- **Scope:** touch only `xtask/src/{adr.rs, lib.rs, steps/…}`, `docs/README.md`,
  the 14 ADR files being normalized, and `docs/adr/0036-…md`. Do not restyle ADR
  bodies beyond heading line 1 and the status line.

## Shared core (design the module once, both consumers reuse it)

The `sync-readme` **writer** and the `adr-readme-parity` **checker** must agree, so
they share one pure core. Put it in `xtask/src/adr.rs` (already the ADR home) or a
new `xtask/src/adr_readme.rs` module — implementer's choice, but ONE source of the
parse/render logic. Core shapes:

- `struct AdrEntry { num: u32, filename: String, title: String, status: String }`
  — parsed from a `docs/adr/NNNN-*.md` file: `num`/`filename` from the name,
  `title` from the `# ADR-NNNN: <title>` heading (prefix stripped), `status` from
  the `- Status: <token>` line.
- `struct TableRow { num: u32, target: String, title: String, status: String }` —
  parsed from a committed markdown row `| [NNNN](adr/slug.md) | Title | status |`.
- `fn parse_adr_dir(dir) -> Result<Vec<AdrEntry>>` — sorted by num.
- `fn parse_table_block(block: &str) -> Vec<TableRow>` — trims cells (padding-proof).
- `fn render_block(entries: &[AdrEntry], existing: &[TableRow]) -> String` — the
  writer's merge: for each entry (ascending), reuse the existing row's **title**
  when a row with that num exists, else seed the title from `entry.title`
  (heading). Emits header + separator + one row per entry, single-space padded.
  Orphan rows (num with no entry) are dropped by construction.
- `fn parity_problems(entries: &[AdrEntry], existing: &[TableRow]) -> Vec<String>`
  — the checker: compares **mechanical** cells only. For each entry: row must exist
  (else "missing row NNNN"); `target` must equal `entry.filename`; `status` must
  equal `entry.status`. Each committed row must have a matching entry (else "orphan
  row NNNN"). Rows must be ascending (else "rows out of order"). **Titles are not
  compared.** Robust to a transient duplicate num (does not panic; the duplicate is
  `identifier-collisions`' concern).
- Marker constants `BEGIN = "<!-- adr-table:begin -->"`,
  `END = "<!-- adr-table:end -->"`; a `fn splice_block(readme, new_block)` that
  replaces the text strictly between the marker lines (error if a marker is
  missing).

Format-conformance (Task 4) is a separate pure fn over the raw ADR files, since it
must report malformed files the parser would otherwise choke on:

- `fn format_problems(dir) -> Vec<String>` — per `docs/adr/NNNN-*.md`: heading line
  must match `# ADR-NNNN: <nonempty>` with `NNNN` == filename number; a
  `- Status: <token>` line must exist with `<token>` in
  `{proposed, accepted, superseded, deprecated, rejected}` and nothing trailing.

---

### Task 1: Normalize the 14 outlier ADR files + add README table markers

**Files:**

- Modify (headings, 12 files): `docs/adr/0019-…`, `0021-…`, `0022-…`, `0026-…`,
  `0030-…`, `0032-…`, `0033-…`, `0034-…`, `0037-…`, `0039-…`, `0040-…`, `0041-…`
- Modify (status, 3 files): `docs/adr/0015-atompub-serialization-surfaces.md`,
  `0029-git-enforced-verify-gate.md`, `0030-coverage-reanchor-text-identity.md`
  (0030 is in both sets)
- Modify: `docs/README.md` (wrap the ADR table in markers)

**Interfaces:** none (documentation). No gate depends on conformance yet (the
`adr-format` gate lands in Task 4), so this is a safe standalone docs commit that
leaves the corpus uniform *before* the gate is introduced.

- [ ] **Step 1: Rewrite the 12 non-canonical headings.**
  Uniform rule on line 1 of each listed file: `# NNNN. <title>` → `# ADR-NNNN:
  <title>` (same number, title text verbatim). E.g. `# 0030. Coverage re-anchor by
  text identity` → `# ADR-0030: Coverage re-anchor by text identity`.

- [ ] **Step 2: Fix the 3 status lines.**
  - `0029` line 3: `Status: accepted` → `- Status: accepted`.
  - `0030` line 3: `Status: accepted` → `- Status: accepted` (heading already done
    in Step 1).
  - `0015` lines 3–5: replace
    ```
    - Status: accepted (content-type token scheme superseded by
      [ADR-0023](0023-atompub-jaunder-wire-extensions.md); the separate-serializers
      principle stands)
    ```
    with
    ```
    - Status: accepted
    - Note: the content-type token scheme is superseded by
      [ADR-0023](0023-atompub-jaunder-wire-extensions.md); the separate-serializers
      principle stands.
    ```

- [ ] **Step 3: Add the table markers in `docs/README.md`.**
  Insert `<!-- adr-table:begin -->` on its own line immediately before the table
  header (current line 26, `| # | Title | Status |`) and `<!-- adr-table:end -->`
  immediately after the last row (current line 69, `0041`). Leave the table
  contents exactly as they are — they already match `docs/adr/` for number/link/
  status, so no row changes here.

- [ ] **Step 4: Gate + commit.**
  ```
  cargo xtask check --no-test
  ```
  (prettier will reflow markers/table; accept its formatting). Expect green.
  ```
  git add docs/adr docs/README.md
  git commit -m "docs(adr): normalize ADR heading/status style; mark the README ADR table (#196)"
  ```

---

### Task 2: `cargo xtask adr sync-readme` — the generator + shared core

**Files:**

- Modify: `xtask/src/adr.rs` (add the shared core above + a `sync_readme() ->
  StepResult` entry point) — or a new `xtask/src/adr_readme.rs` declared in
  `lib.rs`.
- Modify: `xtask/src/lib.rs` (`AdrCommand::SyncReadme` variant, `command_name`
  arm `"adr-sync-readme"`, dispatch arm calling `adr::sync_readme()`).

**Interfaces:**

- Produces: `adr::sync_readme()` (writes `docs/README.md` in place; `StepResult`
  `adr-sync-readme` ok with a summary, or fail on missing markers / unparseable
  ADR). Consumes the shared core (`parse_adr_dir`, `parse_table_block`,
  `render_block`, `splice_block`).

- [ ] **Step 1: Implement the shared core** (`AdrEntry`, `TableRow`,
  `parse_adr_dir`, `parse_table_block`, `render_block`, marker consts,
  `splice_block`) as described in **Shared core** above. Heading parse must accept
  the canonical `# ADR-NNNN: <title>` form (post-Task-1 the corpus is uniform);
  strip the `ADR-NNNN:` prefix for the seeded title.

- [ ] **Step 2: Implement `sync_readme()`** — read `docs/adr/` and
  `docs/README.md`, `render_block(parse_adr_dir, parse_table_block(current))`,
  `splice_block`, write back only if changed. Return a summary
  (`rows: N, added: [..], removed: [..]`).

- [ ] **Step 3: Wire the CLI** — add `SyncReadme` to `AdrCommand` (with
  `after_help` example `cargo xtask adr sync-readme`), the `command_name` arm, and
  the dispatch arm (mirror the `Renumber` arm at `lib.rs:259-265`). This consumes
  the new code — no dead-code gap.

- [ ] **Step 4: Unit tests** (in the core module) — `parse_adr_dir` extracts
  num/title/status from a canonical file; `parse_table_block` trims padded cells;
  `render_block` **preserves an existing title** and **seeds a new row's title from
  the heading**; `render_block` **drops an orphan** and **sorts ascending**;
  `splice_block` replaces only between markers and **errors on a missing marker**.
  Add a `sync-readme parses ADR command` CLI test mirroring
  `adr_renumber_parses` (`lib.rs:478`).

- [ ] **Step 5: Gate + prove idempotence + commit.**
  ```
  cargo xtask check
  ```
  Then verify the generator is a no-op on the already-consistent tree:
  ```
  cargo xtask adr sync-readme
  ```
  Expect it to report no change (or a change that prettier+diff shows is only
  whitespace, which `check` heals) and leave `docs/README.md` clean under
  `git status`. Commit:
  ```
  git add xtask docs/README.md
  git commit -m "feat(xtask): adr sync-readme regenerates the README ADR table from docs/adr/ (#196)"
  ```

---

### Task 3: Fold `sync-readme` into `adr renumber`

**Files:**

- Modify: `xtask/src/adr.rs` (`run_renumber`: after the collision loop, regenerate
  the README table via the shared core, operating on the same `repo` root).

**Interfaces:**

- Consumes the shared core. `run_renumber` already rewrites the moved ADR's own
  `# ADR-NNNN:` heading via `rewrite_bare` (verified: test at `adr.rs:308`
  asserts `# ADR-0035: Bar`), so after a bump the table regen sees the new number
  and slug and updates the row's number + link; a brand-new number appends a row
  seeded from the (rewritten) heading.

- [ ] **Step 1: Regenerate the table at the end of `run_renumber`.** After the
  `for added_name in &added` loop, run the same `render_block`/`splice_block`
  against `repo.join("docs/README.md")` and `repo.join("docs/adr")`, writing it
  back (unstaged, like the reference rewrites). Mention in the returned summary
  that the table was refreshed. Make it tolerant when the README has no markers
  (skip with a note rather than erroring — a test repo may omit them).

- [ ] **Step 2: Test.** Extend the temp-repo renumber tests: seed a
  `docs/README.md` with markers and a row for the colliding ADR, run
  `run_renumber`, assert the row's number cell and link target moved to the bumped
  number. (Reuse the existing `write`/`git` helpers at `adr.rs:245-254`.)

- [ ] **Step 3: Gate + commit.**
  ```
  cargo xtask check
  ```
  ```
  git add xtask
  git commit -m "feat(xtask): fold README table regen into adr renumber (#196)"
  ```

---

### Task 4: The gate steps — `adr-format` then `adr-readme-parity`

**Files:**

- Add: `xtask/src/steps/adr_check.rs` (declared under the `steps` module in
  `lib.rs:10-16`) with `run(result: &mut CommandResult)` pushing two steps:
  `adr-format` (from `format_problems`) then `adr-readme-parity` (from
  `parity_problems`). Both read-only.
- Modify: `xtask/src/lib.rs` (call `steps::adr_check::run(&mut result)` in both the
  `Check` pipeline after line 182 and the `Validate` pipeline after line 208 —
  beside `sequence_check::run`).

**Interfaces:**

- `adr-format` fail detail lists each offending file + reason; recovery is a
  guided manual fix (there is no auto-fixer). `adr-readme-parity` fail detail lists
  drift lines + `  recovery: cargo xtask adr sync-readme` (2-space indent, matching
  `sequence_check.rs:41`). Both consume the shared core / `format_problems`.

- [ ] **Step 1: Implement `format_problems`** (in the core module) per **Shared
  core**, with the fixed status vocabulary. Return sorted, human-readable lines.

- [ ] **Step 2: Implement `steps::adr_check::run`** — push `adr-format`
  (ok/fail from `format_problems`); push `adr-readme-parity` (ok/fail from
  `parity_problems(parse_adr_dir, parse_table_block(readme-block))`, with the
  `recovery:` line appended on failure). If the README markers are missing, fail
  `adr-readme-parity` with a clear "add the adr-table markers" message.

- [ ] **Step 3: Wire into `check` and `validate`.** Add the call after
  `sequence_check::run` in both pipelines. Register the module in the `steps { … }`
  block at `lib.rs:10-16`.

- [ ] **Step 4: Unit tests** — `format_problems` flags a bad heading form, a
  filename/heading number mismatch, a missing/malformed status, an
  out-of-vocabulary token, and is empty on a clean set. `parity_problems` flags a
  missing row, a wrong target, a wrong status, an orphan row, and mis-ordering;
  **ignores a title difference**; is empty when mechanical cells agree; does not
  panic on a duplicate num.

- [ ] **Step 5: Gate + commit.** By now the tree is uniform (Task 1) and the table
  matches (Task 2), so both new steps pass.
  ```
  cargo xtask check
  ```
  ```
  git add xtask
  git commit -m "feat(xtask): gate ADR file style + README-table parity (#196)"
  ```

---

### Task 5: ADR-0036 addendum + CONTRIBUTING pointer

**Files:**

- Modify: `docs/adr/0036-identifier-collision-policy.md` (append an addendum
  section).
- Modify: `CONTRIBUTING.md` **iff** it documents ADR-table maintenance — repoint it
  at `cargo xtask adr sync-readme` / the always-`0000` flow. (Grep first; only edit
  if there's a live instruction.)

**Interfaces:** none (documentation).

- [ ] **Step 1: Write the ADR-0036 addendum** — a `## Addendum (#196): the README
  ADR table is a generated projection` section recording: the table's number/link/
  status cells are generated from `docs/adr/` by `cargo xtask adr sync-readme`
  (folded into `adr renumber`); titles stay hand-curated (seeded from the heading
  on row creation); canonical ADR style is `# ADR-NNNN: <title>` + `- Status:
  <token>` from the fixed vocabulary; two read-only gates (`adr-format`,
  `adr-readme-parity`) enforce it. Keep `Status: accepted` unchanged.

- [ ] **Step 2: Check CONTRIBUTING.** `rg -n 'ADR' CONTRIBUTING.md` — if it tells
  authors to hand-edit the README ADR row, replace that with the generated-table
  flow. If it says nothing about the table, make no change.

- [ ] **Step 3: Gate + commit.**
  ```
  cargo xtask check --no-test
  ```
  ```
  git add docs/adr/0036-identifier-collision-policy.md CONTRIBUTING.md
  git commit -m "docs(adr): record generated README ADR table as an ADR-0036 addendum (#196)"
  ```

---

### Task 6: Final validation

**Files:** none (verification gate).

- [ ] **Step 1: Full local gate.**
  ```
  cargo xtask validate --no-e2e
  ```
  (This issue touches no runtime/e2e surface — xtask + docs only — so `--no-e2e`
  is the meaningful gate; a full `validate` is optional and only exercises unrelated
  VMs.) Expect green: static+prettier, `identifier-collisions`, the new
  `adr-format` + `adr-readme-parity`, host xtask unit suite, coverage unchanged.

- [ ] **Step 2: End-to-end behavior spot-check** (throwaway, not committed) — in a
  scratch area, create a `docs/adr/0000-scratch.md` with a canonical heading +
  status, confirm `adr-format` passes it but `identifier-collisions` flags the
  `0000` dup, run `cargo xtask adr renumber`, and confirm it bumps the file,
  rewrites the heading number, and appends a README row seeded from the heading.
  Revert the scratch file. Record the observed behavior for the ship hand-off.

- [ ] **Step 3: Record evidence** — summarize the gate result + the spot-check for
  the pre-merge halt.

---

## Deferred / follow-ups (not filed — raise with the user at ship if wanted)

- **`cargo xtask adr fmt` auto-fixer** — mechanically normalize a non-canonical
  heading prefix / status list-marker (can't auto-resolve a typo'd status token).
  Explicitly out of scope (spec Non-goals); a plausible later DX issue.
- **The prose line "All are currently `accepted`."** (`README.md:24`) is still
  hand-maintained and outside the markers; it stays true today (0015 remains
  `accepted`). Left untouched; a future non-`accepted` ADR would need it updated or
  removed.
- **`jaunder-adr` skill** — held pending this issue (per project memory); write it
  against the shipped behavior afterward, then collapse the ADR mechanics currently
  duplicated in `jaunder-start`/`jaunder-ship` to pointers.

## Self-review

- Spec D1 (generator owns number/link/status; titles preserved; new row seeded;
  orphan removed; sorted; idempotent; marked block) → Task 2 (`render_block`
  + tests). ✓
- Spec D2 (HTML-comment markers) → Task 1 Step 3 + `splice_block`. ✓
- Spec D3 (fold into renumber) → Task 3. ✓
- Spec D4 (canonical heading + status; fixed vocab; fix 14 files) → Task 1 +
  `format_problems`. ✓ (heading auto-rewrite under renumber verified via
  `adr.rs:308`.)
- Spec D5 (two read-only steps; semantic parity; recovery hints; robust to dup) →
  Task 4. ✓
- Spec ADR (0036 addendum, no new number) → Task 5. ✓
- Dead-code boundary respected: every task that adds Rust also wires/consumes it in
  the same commit. ✓
- Coverage: xtask excluded from the flake → no baseline churn; host unit tests are
  the safety net. ✓
- Ordering: normalize corpus (Task 1) **before** introducing the gates (Task 4) so
  the gates are green when they land. ✓
