# Plan — issue #930: comment audit across the codebase

Spec: `docs/archive/2026-08-11-issue-930-comment-audit-spec.md` (defect
classes, scope, protected patterns, deliverables). For agentic workers: drive
with `jaunder-iterate`; delegate the read-only audit passes via
`jaunder-dispatch`/Explore subagents, keep edits and gates in the main loop.

## Review header

**Goal.** Audit every in-scope comment against the spec's four defect classes;
edit directly; compile the findings report; draft ADRs for promoted decisions;
deliver a verdict on the CONTRIBUTING.md standard.

**Scope.** In: the spec's crate/dir/root-file list, tests included. Out:
migrations SQL, markdown docs, generated code, protected gate-marker comments
(spec "Protected comment patterns").

**Tasks (one line each).**

1. Report skeleton + candidate survey (parallel read-only subagent audits).
2. Edit pass: `common` + `macros`.
3. Edit pass: `server`.
4. Edit pass: `storage` (Rust only; migrations untouched).
5. Edit pass: `web` + `csr` + `client` + `host`.
6. Edit pass: `xtask` (largest comment mass) + `tools`.
7. Edit pass: `end2end` + `test-support` + `elisp` + `scripts` + `.githooks` +
   `.github` + root files.
8. ADR drafts: write/finalize `docs/adr/drafts/` entries for promotions.
9. Report: complete catalogue + judgment calls + verdict on the CONTRIBUTING.md
   standard (proposed wording, not applied).

**Key risks/decisions.** Gate-marker adjacency (ADR-0094) — never touch lines
adjacent to marker comments; every commit runs the full gate (pre-commit), which
fails closed on marker damage. No behavior changes: the diff of every commit
must be comment-only (plus the report/ADR files). Commits are `docs(<area>): …`
Conventional Commits, no `Co-Authored-By`, stage-then-commit.

## Global constraints

- Worktree:
  `/home/mdorman/src/jaunder/.claude/worktrees/issue-930-comment-audit` (branch
  `worktree-issue-930-comment-audit`). Pin every gate run:
  `devtool run --cwd <worktree> -- cargo xtask check`.
- Before each commit: `git add -p`-style deliberate staging of the area's files
  only; verify with `git diff --cached --stat`; commit message
  `docs(<area>): comment audit (#930)` with a body naming the defect classes
  addressed. Pre-commit runs the full gate.
- Review each candidate **in context** (Read the surrounding code) before
  editing — a grep hit is a candidate, not a verdict. Pure why-comments stay.
  When in doubt, keep and record in the report.
- Every non-obvious call (kept-despite-length, rewritten-not-deleted,
  ADR-promoted) is appended to the report file as it happens, not reconstructed
  later.
- Elisp comment conventions (`;;` density, `;;;` headers) are idiomatic — don't
  "fix" idiom; audit content only.

## Tasks

### Task 1 — Report skeleton + survey

- [x] Create
      `docs/archive/2026-08-11-issue-930-comment-audit-report.md` with
      sections: per defect class (findings/actions), Judgment calls, ADR
      promotions, Left alone deliberately, Verdict on the standard.
- [x] Seed-discovery read: before any grepping, directly read a sample of files
      across the areas — the comment-densest file in each area (per-file
      comment-line counts) plus 2–3 arbitrary others — and note the actual
      phrasings of any defects found. Harvest new grep seeds from them;
      defective comments cluster in an author's habitual vocabulary, so real
      examples beat guessed words.
- [x] Launch parallel read-only Explore subagents, one per edit-pass area (tasks
      2–7), each briefed with: the spec path (they read the spec — defect
      classes + protected patterns), their file list, and instructions to return
      file:line candidates classified by defect class, with a short quote and a
      keep/delete/rewrite/promote recommendation. **Reading is primary, grep is
      a supplement**: each subagent reads its area's most comment-dense files
      directly (candidates surface from reading, not pattern-matching), then
      greps the full area with the seed list — the initial seeds below plus
      whatever the seed-discovery read harvested — and finally scans for long
      comment runs (e.g. ≥6 consecutive `//` lines outside `//!`/`///`). Initial
      seeds (not exhaustive): `no longer`, `previously`, `used to`, `formerly`,
      `historically`, `was removed`, `moved from`, `renamed`, `legacy`, `old `,
      `originally`, `now that`, `we now`, `instead of the old`.
- [x] Fold the returned candidate lists into the report skeleton as the working
      inventory (marked "unverified").

### Tasks 2–7 — Edit passes (one per area; identical procedure)

Areas: (2) `common` + `macros`; (3) `server`; (4) `storage` minus
`storage/migrations/*.sql`; (5) `web`, `csr`, `client`, `host`; (6) `xtask`,
`tools`; (7) `end2end`, `test-support`, `elisp`, `scripts`, `.githooks`,
`.github` workflows, root files (`Cargo.toml`, `clippy.toml`, `deny.toml`,
`.rustfmt.toml`, `rust-toolchain.toml`, `flake.nix`).

Per area:

- [x] (2) For each candidate from Task 1: Read the surrounding code; decide keep
      / delete / rewrite / shorten / promote per the spec's defect classes;
      apply with Edit. Never touch a line adjacent to a protected marker (spec
      list). For "promote": write or extend the ADR draft (Task 8 owns final
      wording) and leave the path-form pointer comment.
- [x] (2) Sweep the area once more yourself for defects the subagent missed
      (spot-check files with the densest comment counts).
- [x] (2) Update the report: move each handled candidate from "unverified" to
      its outcome; record judgment calls.
- [x] (2) Gate: `devtool run --cwd <worktree> -- cargo xtask check` → must be
      `ok: true`. For area 7 (e2e/elisp/CI files) the same gate covers
      formatting and the comment-parsing steps; elisp has no gate — eyeball `;;`
      edits for balance.
- [x] (2) Verify the diff is comment-only: `git diff -- <area paths>` and
      confirm no executable-line changes; stage the area's files; commit
      `docs(<area>): comment audit (#930)`.
- [x] (3) Same procedure for `server` (done first — its inventory arrived while
      `common`'s was still pending).
- [x] (4) Same procedure for `storage` (Rust/TOML only).
- [x] (5) Same procedure for `web` + `csr` + `client` + `host`.
- [x] (6) Same procedure for `xtask` + `tools`.
- [x] (7) Same procedure for `end2end` + `test-support` + `elisp` + `scripts` +
      `.githooks` + `.github` + root files (elisp, .githooks and .github
      workflows audited clean — no edits needed).

### Task 8 — ADR drafts

- [x] For every promotion recorded in the report, ensure a draft exists in
      `docs/adr/drafts/<slug>.md` with heading `# ADR-DRAFT: <Title>`,
      `- Status: proposed`, Context/Decision/Consequences; the code comment
      points at `docs/adr/drafts/<slug>.md` (path form, so `adr promote`
      rewrites it at ship).
- [x] Cross-check: `rg -n 'docs/adr/drafts/' --type rust` (and elisp/ts) lists
      exactly the promoted pointers; each target file exists. (Drafts are
      gitignored — they ride to ship outside the PR diff; note this in the
      report so the reviewer knows where to look.)

### Task 9 — Report completion + verdict

- [x] Fill the "Verdict on the standard" section: judge whether CONTRIBUTING.md
      "Comment for intent, not mechanics" suffices, informed by the tally of
      defects found per class; if not, propose exact replacement/addition
      wording (naming backward-looking and essay defects). Proposed only — no
      CONTRIBUTING.md edit.
- [x] Finish the catalogue: totals per defect class per area; "left alone
      deliberately" list complete.
- [x] `devtool run --cwd <worktree> -- prettier -w <report>`; final gate
      `devtool run --cwd <worktree> -- cargo xtask check`; commit
      `docs(superpowers): issue-930 findings report (#930)`.

## Execution handoff

After plan approval (HALT), execute with `jaunder-iterate`, ticking these
checkboxes in real time. Ship via `jaunder-ship` (archives this plan and the
report to `docs/archive/`, promotes ADR drafts, opens the PR).
