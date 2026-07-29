# Issue #682 — Markdown link integrity: fix `adr promote`, backfill, and gate it

- Issue: [#682](https://github.com/jaunder-org/jaunder/issues/682)
- Date: 2026-07-29
- Status: awaiting approval

## Problem

`cargo xtask adr promote` graduates a draft from `docs/adr/drafts/<slug>.md` to
`docs/adr/NNNN-<slug>.md` — **up one directory** — but never adjusts the draft's
own outbound relative links. A sibling ADR reference written correctly while
drafting (`../0061-web-keyed-list-reactive-store.md`) resolves, after promotion,
to `docs/0061-…` — outside `docs/adr/`, pointing at nothing.

Nothing catches it. `adr-format` and `adr-readme-parity` never look at link
targets, and the repo has no Markdown link checker. ADR-0078 was authored this
way and hand-corrected at ship (#587), which is what surfaced the bug.

The `drafts/README.md` authoring rules cover only the _inbound_ direction (rule
3, "reference the draft by path"). Note that the file's own
`[`../template.md`](../template.md)` at line 19 is **correct where it sits** —
`docs/adr/template.md` exists, and that README is never promoted. It is not
itself a defect; it is the habit a _draft_ copies, at which point promotion
breaks it.

Measured over the gated corpus (definition in Scope §3): **19 dead relative
links across 4 files** — the 4 the issue names, plus 15 in the two most-read
live design documents.

## Decisions

Resolved during the design interview. Each binds the implementation.

**D1 — promote strips exactly one `../` level.** The issue proposed a
sibling-specific rewrite (`](../NNNN-` → `](NNNN-`). Rejected for the general
rule, on two grounds that are _not_ about rarity: the general rule is a direct
statement of the actual invariant (the file moves up exactly one directory, so
every relative link is off by exactly one level), and it is no more complex to
implement or explain than the narrow one. It additionally covers
`](../template.md)`, the shape `drafts/README.md:19` models for authors.

**D2 — the rewrite touches only inline link targets, outside code.** It applies
to the target inside `](…)` and nothing else, and skips fenced code blocks and
inline code spans. A draft may legitimately discuss `../` in prose or show it in
a shell snippet; a blanket string replace would silently corrupt those.

**D3 — promote does not fix every dead shape, by design.** Two shapes survive
and are left to the gate: `](docs/adr/NNNN-slug.md)` (repo-root form, which Pass
C actively produces, and which is _correct_ from `CONTRIBUTING.md` but dead from
inside another ADR), and `](sibling-draft.md)` (no `../` to strip). The reason
is **not** that they are rare — it is that fixing them requires real path
resolution: deciding what a root-relative target should become depends on the
referencing file's location, which is exactly the general link-rewriting
problem. D1's rule needs no resolution at all. The gate covers these precisely,
and D4 names them at ship.

**D4 — promote warns, it does not fail, and it checks last.** By the time
promote can check, it has written the ADR, deleted the draft, synced the README
table and staged all of it. Failing over a completed, partly-successful mutation
leaves a tree that cannot be cleanly re-run (the draft is gone). Promote appends
a warning naming the file and each unresolved target to its returned summary;
`doc-links` turns that into a hard failure moments later on a stable,
re-runnable tree.

**D5 — one pure function, two callers; no new command.** A single per-file
function is called by (a) `promote`, on the file it just wrote, and (b) the
`doc-links` gate step. `doc-links` is one more `StepResult` pushed by the
existing steps wiring that `cargo xtask check` / `validate` already run — **no
new CLI subcommand, no new invocation, no parallel mechanism.**

**D6 — the gate lives in `xtask`, because D5 is unimplementable in `devtool`.**
`xtask` and `devtool` are separate workspaces that communicate **only over a CLI
boundary** — `tools/devtool/src/check.rs:14-16` states the host list "can't
import this list, being a separate host-only workspace that reaches devtool only
over the CLI." `promote` lives in `xtask`. If the link logic lived in `devtool`,
promote could only shell out to a whole-tree check (not the one file it wrote)
or reimplement the logic — reintroducing the duplication ADR-0052 exists to
kill.

Secondary support: every one of devtool's 8 checks wraps an **external binary**
(`cargo`, `prettier`, `tsc`, `emacs`, `leptosfmt`); `doc-links` is pure Rust
logic over the source tree, like `adr-format`, `adr-readme-parity`, and
`identifier-collisions`, all of which live in `xtask`. And adding a 9th devtool
check means hand-editing two lists with no compiler enforcement
(`check.rs:14-16`), a small divergence surface of exactly the kind ADR-0052
targets.

**On ADR-0052's reach — stated accurately.** Its decision sentence is a class
rule ("devtool is the single implementation of the non-compiling static checks",
line 41) and `doc-links` does not compile, so the _letter_ points at devtool.
But its mechanism is "owns each check's **tool + arguments**" (line 42) for
checks that had drifted between a host `StepSpec` and a hand-written nix
sibling. Note its context names **five** such siblings (line 12) while the
decision covers **seven** non-compiling checks (line 35) — `tsc` and `tools-fmt`
were never duplicated — so "specified twice" does not define the set.
`doc-links` wraps no tool and has no nix sibling; the CLI-boundary argument
above, not this one, is what decides placement.

**D7 — no ADR is written.** Placement resolves to ADR-0028's litmus and the
workspace boundary above; the frozen-archive exclusion to
`docs/README.md:112-117` and `.prettierignore:1`; the promote fix is an
implementation repair to ADR-0048's flow, not a change to it.

**D8 — the issue's coverage claim is wrong and is not carried forward.** The
issue asks for a test "in `xtask` (the crate is coverage-measured)". It is not:
`xtask/src/steps/host_tests.rs:6-10` states `xtask` and `tools/` "are each
excluded from every Nix check" with "**No coverage here**", and
`flake.nix:1183-1184` excludes both from the coverage derivation's source. Tests
there do run and gate, in every mode, via the `xtask-tests` step. Criteria below
reference that step.

**D9 — pass ordering inside `promote` is fixed, not incidental.** `adr.rs` runs
Pass B (write graduated file, delete draft, stage) then Pass C (rewrite
`drafts/<slug>` → `NNNN-<slug>` repo-wide, including inside the graduated file).

- The **level-strip runs in Pass B**, at the moment the file is written to its
  new location — that is where the "moved up one directory" fact lives. A draft
  link `](../drafts/aaa.md)` becomes `](drafts/aaa.md)`, which Pass C then
  resolves to `](0002-aaa.md)`.
- The **warning check runs last**, after Pass C _and_ the README sync, when the
  file is in final form. Running it earlier would false-warn on every
  multi-draft ship, because Pass B has already deleted the draft that a
  not-yet-rewritten `drafts/…` target names.

**D10 — the gate skips code spans and fenced blocks.** Required prospectively,
not for existing noise: Scope §4 adds prose to `CONTRIBUTING.md` and
`docs/adr/drafts/README.md` — both **gated** files — where the natural phrasing
of the sibling-link rule is a backticked `` `](NNNN-slug.md)` `` example.
Without the carve-out the gate fails its own documentation. Today it suppresses
0 findings in the gated set (and 33 in `docs/archive/`), so it changes no
current number.

## Scope

Four pieces, ordered so every commit is green — the pre-commit gate runs the
verify ladder, so the gate cannot land before its backfill.

### 1. Fix `promote` (the cause)

Strip one leading `../` from every inline link target in the draft being
promoted, in Pass B, per D1/D2/D9. Warn on any target that still fails to
resolve, after Pass C, per D4/D9.

Edge cases the rule must pin down (D1 is otherwise silent): a target of exactly
`..` or `../` is left unchanged; `../` occurring non-initially (`a/../b.md`) is
not stripped — only one **leading** `../` is removed; a target with no leading
`../` is unchanged.

### 2. Backfill the 19 dead links (the debt)

| File                                           | Count | Nature                                                           |
| ---------------------------------------------- | ----- | ---------------------------------------------------------------- |
| `docs/ARCHITECTURE.md`                         | 7     | `decisions/NNNN-*.md` → `adr/NNNN-*.md`                          |
| `docs/ARCHITECTURE.md:70`                      | 1     | `../common/src/storage/` → `../storage/src/` (link **text** too) |
| `docs/DESIGN.md`                               | 7     | `decisions/NNNN-*.md` → `adr/NNNN-*.md`                          |
| `docs/adr/0057-e2e-capture-dir-contract.md:13` | 1     | drop `../`                                                       |
| `docs/adr/0069-client-crate-wasm-only-home.md` | 3     | drop `../` (lines 12, 29, 35)                                    |

The 14 `decisions/` links are mechanical: `decisions/` is the pre-ADR name of
`docs/adr/`, and every target exists at an identical filename there.
`common/src/storage/` no longer exists — those traits live in the `storage/`
crate (`storage/src/{users,posts,sessions,atomic}.rs`), matching the surrounding
prose.

### 3. Add the `doc-links` gate (the guard)

**Shape (D5).** Logic in `xtask/src/doc_links.rs`, step in
`xtask/src/steps/doc_links.rs`, mirroring the `adr_readme.rs` / `adr_check.rs`
split the ADR gates already use:

- `pub struct DeadLink { pub line: usize, pub target: String }`
- `pub fn dead_links_in(repo: &Path, rel: &str) -> Result<Vec<DeadLink>>` — the
  shared per-file unit. **Both** callers use this one.
- `pub fn gated_files(repo: &Path) -> Result<Vec<String>>` — enumeration, used
  by the step only. Promote never enumerates; it passes the single file it
  wrote.

Enumeration needs a new `git::ls_files(repo) -> Result<Vec<String>>` helper —
`xtask/src/git.rs` has `merge_base`, `diff_names`, `diff_added`, `grep_files`,
`mv`, `add`, but no `ls-files`.

**File set.** Tracked `*.md` via `git ls-files`, minus `docs/archive/` and
`docs/superpowers/`. Today: **95 files, 149 relative links**.

**Why tracked-only:** it excludes gitignored `docs/adr/drafts/` **by
construction**, preserving the "Gate invisibility" invariant with no special
case; it excludes untracked local files (`AGENTS.md`, `docs/agents/`,
`.claude/`); and it picks up clean files outside `docs/` for free
(`CONTRIBUTING.md`, root `README.md`, `elisp/README.md`).

**Exclusions — two lists, and they are not the same list.** `.prettierignore`
excludes `docs/archive/` only; `doc-links` excludes `docs/archive/` **and**
`docs/superpowers/`. These are deliberately separate — superpowers docs _should_
be prettier-formatted but _should not_ be link-checked — so neither can be
derived from the other. Both must therefore be documented explicitly (Scope §4),
or they become the silent-divergence class ADR-0052 warns about.

- `docs/archive/` — a frozen historical record (`docs/README.md:112-117`). Its
  links are dead _because_ the docs moved on; rewriting it would falsify the
  record. Currently 177 dead / 188 links.
- `docs/superpowers/` — transient by design; nothing is meant to persist there,
  and a plan may legitimately link a file it will create. At ship these move
  into `docs/archive/`, also excluded, so specs and plans are never link-checked
  at any point in their life. That is intended.

**Link syntax.** Inline `](target)` only, outside code spans and fenced blocks
(D10). Skip `http:`/`https:`/`mailto:` and bare `#anchor`. Strip any `#fragment`
before resolving. Resolve relative to the containing file's directory. A target
may be a **file or a directory** — `docs/README.md` links `adr/` and `archive/`.
No anchor validation. (Measured: the gated corpus has zero reference-style
definitions, zero `<a href>`, zero `%`-encoding, zero angle-bracket links, zero
link titles.)

**Failure output.** `StepResult::fail("doc-links")` with one line per finding —
`<file>:<line> -> <target>`. **No `recovery:` line**: unlike `adr-readme-parity`
(`adr_check.rs:34`, which can point at `cargo xtask adr sync-readme`), a dead
link has no mechanical fix — the intended target is unknowable. This matches
`adr-format`, whose resolution is "a guided manual fix."

**Tracked-but-absent files.** `git ls-files` lists a file staged for deletion
but gone from the worktree. Such files are skipped, not reported as errors.

### 4. Documentation

- `CONTRIBUTING.md` — a `doc-links` bullet in the verify-ladder list, naming
  **both** exclusions and why they differ from `.prettierignore`'s single one.
- `docs/adr/drafts/README.md` authoring rules — a line stating that sibling
  links in a draft are written **as if the file already lived in `docs/adr/`**,
  because promotion moves it up one directory.
- `docs/adr/drafts/README.md` authoring rules — a second line covering
  **draft-to-draft** links, which must be written `](../drafts/<slug>.md)`. This
  closes a workflow landmine the gate would otherwise create: rule 3 tells
  authors to reference a draft by its repo-root path, and for a _cross-draft_
  link Pass C turns that into `docs/adr/NNNN-<slug>.md` — dead from inside
  `docs/adr/` (**D3**), so promote warns and then `doc-links` hard-fails the
  ship commit with no mechanical fix. The `../drafts/` form survives: Pass B
  strips one level to `drafts/<slug>.md`, which Pass C rewrites to the assigned
  number. Rule 3 remains correct for references from outside `docs/adr/`.
- `docs/adr/drafts/README.md` "Gate invisibility" (lines 44-49) — amend: it
  currently claims all ADR gates share one `read_dir` enumeration rule, which
  `doc-links` breaks by enumerating tracked files instead.

## Acceptance criteria

Each is observable — a reviewer can run it and see pass or fail.

1. **AC1 — promote fixes sibling links.** Given a draft containing
   `](../0001-foo.md)`, after `run_promote` the graduated file contains
   `](0001-foo.md)` and no `](../0001-foo.md)`.
2. **AC2 — the strip is general, not ADR-specific.** Given `](../template.md)`,
   the graduated file contains `](template.md)`. Given
   `](../../CONTRIBUTING.md)`, it contains `](../CONTRIBUTING.md)`.
3. **AC3 — the rewrite spares prose and code.** Given a draft whose body
   contains `../foo` inside a fenced code block, inside an inline code span, and
   in plain prose, all three are byte-identical in the graduated file.
4. **AC4 — strip edge cases.** Targets `..`, `../`, and `a/../b.md` are
   unchanged; a target with no leading `../` is unchanged.
5. **AC5 — promote warns on a surviving dead link.** Given a draft containing
   `](nonexistent.md)`, `run_promote` returns `Ok`, the graduated file exists,
   and the returned summary names both the file and the unresolved target. The
   step does **not** fail.
6. **AC6 — promote's check runs after Pass C (D9).** Given two drafts where
   `bbb` references `aaa` by the path form `drafts/README.md` rule 3 teaches,
   promoting both yields a summary with **no** warning for that reference, and
   `bbb`'s graduated file links `aaa`'s assigned number. (Fails if the check
   runs before Pass C.)
7. **AC7 — the gate reports a dead link.** A test constructing a repo whose
   tracked Markdown contains a dead relative link asserts `doc-links` reports
   it, naming file, line, and target.
8. **AC8 — the gate ignores what it must.** Separate assertions that **no**
   finding is produced for: a dead link in `docs/archive/`; one in
   `docs/superpowers/`; one in an untracked file; one inside a fenced code
   block; one inside an inline code span; an `https://` target; a bare
   `#anchor`; and a tracked-but-deleted file.
9. **AC9 — directory targets resolve.** A tracked link to an existing directory
   (`adr/`) produces no finding.
10. **AC10 — the gate passes on the real repo.** After the backfill,
    `cargo xtask check --no-test` reports `doc-links` ok. (Subsumes "all 19
    links resolve": if the gate is ok, no gated link is dead.)
11. **AC11 — no new CLI surface.** `cargo xtask --help` gains no subcommand;
    `doc-links` appears only as a step in `check` / `validate` output.
12. **AC12 — one implementation, named.** `xtask::doc_links::dead_links_in`
    exists and is the **only** function resolving link targets; both
    `xtask::adr::run_promote` and `xtask::steps::doc_links` call it. Observable
    by grep: no second resolution implementation.
13. **AC13 — tests gate.** Every test above runs under the `xtask-tests` step,
    i.e. `cargo test --manifest-path xtask/Cargo.toml` exercises them.
14. **AC14 — docs updated, with required content.** `CONTRIBUTING.md`'s
    verify-ladder list contains a `doc-links` bullet naming both `docs/archive/`
    and `docs/superpowers/`. `docs/adr/drafts/README.md` states the sibling-link
    rule **and** the draft-to-draft `../drafts/<slug>.md` rule, and its "Gate
    invisibility" paragraph names `doc-links` and its tracked-file enumeration
    as an exception to the shared `read_dir` rule.

**Implementer note (not a criterion):** AC7's test must be seen to fail before
the backfill lands — write it, watch it go red against a deliberately dead link,
then make it green. A gate that has never been observed failing is not known to
bite.

## Out of scope

- **`docs/archive/`'s 177 dead links** — frozen record, deliberately never
  fixed.
- **`docs/superpowers/`'s 2 dead links** — transient docs, deliberately ungated.
- **Anchor (`#fragment`) validation** — existence of the target file only.
- **Rewriting link _text_**. D2 rewrites the target, not the visible text, so a
  draft written ``[`../template.md`](../template.md)`` emerges with a correct
  target and stale text. Stated rather than silent; the one instance in the
  backfill (`docs/ARCHITECTURE.md:70`) is fixed by hand because it is a live
  doc, not a promotion artifact.
- **Teaching promote general path resolution** — per D3.
- **Non-Markdown link forms** — no reference-style, `<a href>`, or angle-bracket
  handling; measured to be zero in the gated corpus.

## Risks

- **`docs/ARCHITECTURE.md` has uncommitted parked work** — a full rewrite sits
  in the `adr-materialized-view` worktree. Eight of the 19 backfilled links are
  in that file, so this cycle will conflict with that parked branch when it
  resumes. The backfill here is small and mechanical; flagged so whoever resumes
  the rewrite expects it rather than discovers it.
- **The `ALL`-style sync trap does not apply here, but a similar one does.** The
  two exclusion lists (`.prettierignore` and `doc-links`') are independent by
  design. Scope §4 documents both; if a future exclusion is added to one without
  the other, nothing detects it.
