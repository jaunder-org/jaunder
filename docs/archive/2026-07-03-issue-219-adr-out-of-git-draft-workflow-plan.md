# Plan: ADRs drafted out of git, numbered at ship (issue #219)

**Spec:**
`docs/superpowers/specs/2026-07-03-issue-219-adr-out-of-git-draft-workflow.md`
(user-approved) **Issue:**
[#219](https://github.com/jaunder-org/jaunder/issues/219) **For agentic
workers:** drive with `jaunder-iterate`; delegate a task to a subagent via
`jaunder-dispatch` when useful. Tick checkboxes in real time.

## Overview

Move ADR number assignment from _authoring_ (committed early, churns after
rebase) to _ship_ (post-rebase, correct on first write). New ADRs are authored
numberless in a gitignored `docs/adr/drafts/`; a new `cargo xtask adr promote`
numbers them at ship. The three ADR gates need no change — their shared
enumeration rule (`is_file → .md → leading number`, non-recursive over
`docs/adr/`) already excludes a numberless draft in a subdirectory. `renumber`
is retained for the already-committed-ADR residual-race case; the amend
discipline for that case is documented, not coded.

## Global constraints

- **Gate before every commit:** `cargo xtask check` clean (fmt + clippy + Nix
  coverage/tests). Run via `devtool run -- cargo xtask check` from the worktree
  (worktree-aware, honest exit) — context-mode's cwd is the MAIN repo.
- **No `Co-Authored-By` trailer.** Commit only with user approval (per project
  CLAUDE.md) — `jaunder-commit` governs.
- **Coverage policy:** new xtask code needs coverage; every `run_promote` branch
  is exercised by a test in this plan.
- **xtask test convention:** in-file `#[cfg(test)]`, throwaway-git-repo harness
  — mirror the existing `renumber` tests in `xtask/src/adr.rs` (`git`/`write`
  helpers, `std::env::temp_dir()` + `std::process::id()` for a unique dir).
- **No storage/dual-backend work here** — this is xtask + docs only.

---

## Task 1 — Drafts holding pen + gate-invisibility lock ✅ done (ed160cd8)

Establish the untracked draft directory and prove the gates ignore it. No
production Rust logic changes; the test locks pre-existing enumeration behavior
against future refactors.

**Files**

- `.gitignore` — append:
  ```gitignore
  # ADR drafts live out of git until `cargo xtask adr promote` numbers them at
  # ship (see docs/adr/drafts/README.md). Keep the explainer tracked.
  docs/adr/drafts/*
  !docs/adr/drafts/README.md
  ```
- `docs/adr/drafts/README.md` (tracked) — explains: why drafts live here
  (numbered at ship, not authoring), the `# ADR-DRAFT:` heading,
  reference-by-path rule, and `cargo xtask adr promote` graduation. Cross-link
  `../template.md`, `CONTRIBUTING.md`, and the `jaunder-adr` skill.
- `docs/adr/template.md` — change line 1 from
  `# ADR-0000: Title of the decision` to `# ADR-DRAFT: Title of the decision`.
  (No leading number → already gate-invisible; safe.)

**Test** (`xtask/src/adr_readme.rs`, in the existing `#[cfg(test)] mod tests`)

- `drafts_subdir_is_invisible_to_gates`: in a temp repo with a real
  `docs/adr/0001-a.md` (canonical heading+status) **and** a numberless
  `docs/adr/drafts/some-decision.md` (`# ADR-DRAFT: Some Decision`, no status
  line), assert `format_problems(repo)` is empty and `parity_report`/`adr_files`
  do not surface the draft. Locks that a malformed-by-ADR-rules draft in the
  subdir never trips `adr-format`.

**Run**

- `devtool run -- cargo nextest run -p xtask drafts_subdir_is_invisible_to_gates`
  → PASS (behavior pre-exists; this is a characterization test).
- `devtool run -- cargo xtask check` → PASS (gates green with the drafts dir
  present).

**Commit:** `feat(adr): add gitignored docs/adr/drafts holding pen (#219)`

---

## Task 2 — `cargo xtask adr promote` ✅ done (9882c59)

Add the subcommand that numbers drafts at ship. TDD: write the promote tests
RED, then implement GREEN.

**Files — CLI wiring (`xtask/src/lib.rs`)**

- Add to `enum AdrCommand` (after `SyncReadme`):
  ```rust
  /// Assign the next free number to each ADR draft in `docs/adr/drafts/`,
  /// move it to `docs/adr/NNNN-<slug>.md`, rewrite its path-form references,
  /// sync the README table, and stage the result. Run at ship, post-rebase.
  #[command(after_help = "EXAMPLES:\n  cargo xtask adr promote")]
  Promote,
  ```
- `command_name()`: add `Command::Adr(AdrCommand::Promote) => "adr-promote",`.
- Dispatch (near the `Renumber`/`SyncReadme` arms):
  ```rust
  Command::Adr(AdrCommand::Promote) => {
      let mut result = CommandResult::new("adr-promote");
      result.push(adr::promote());
      result
  }
  ```
- Parse test (mirror `adr_renumber_parses`): `adr_promote_parses` asserts
  `["xtask","adr","promote"]` → `command_name() == "adr-promote"`.

**Files — implementation (`xtask/src/adr.rs`)**

- `pub fn promote() -> StepResult` wrapping `run_promote(Path::new("."))`
  (mirror `renumber()`).
- `fn run_promote(repo: &Path) -> Result<String>`:
  1. Enumerate `docs/adr/drafts/*.md` (via `read_dir`), excluding `README.md`;
     collect `(slug, filename)`. Sort by slug for determinism. Empty →
     `Ok("no ADR drafts to promote".into())`.
  2. `let mut all = adr_filenames(repo);` (existing helper — numbered ADRs in
     `docs/adr/`).
  3. **Pass A (assign):** for each draft in order,
     `let n = ids::next_number(&all); let new_name = format!("{}-{slug}.md", pad(n)); all.push(new_name.clone());`
     record `(slug, n, new_name)`.
  4. **Pass B (apply)** per assignment:
     - Read `docs/adr/drafts/<slug>.md`; rewrite heading token `ADR-DRAFT` →
       `ADR-{pad(n)}` (reuse `rewrite_bare`-style `str::replace` on
       `"ADR-DRAFT"`); write to `docs/adr/<new_name>`; remove the draft file.
     - Path-form refs repo-wide: `git grep -l --fixed-strings drafts/<slug>`
       (tolerate no-match), then
       `rewrite_stem(content, "drafts/<slug>", &format!("{}-{slug}", pad(n)))`
       per hit. (Same slug-uniqueness assumption `rewrite_stem` documents.)
     - `git add docs/adr/<new_name> docs/adr/drafts/<slug>.md` (stages the add
       and the deletion).
  5. README sync:
     `if adr_readme::readme_has_markers(repo)? { sync_readme_at(repo)? } else { note skip }`
     — mirror `renumber`.
  6. Return summary:
     `drafts/<slug>.md -> <new_name>; …; README table synced (…); staged`.
- Add helper `fn draft_filenames(repo: &Path) -> Vec<(String, String)>` (slug,
  filename) for `docs/adr/drafts/`, excluding `README.md`.

**Test** (`xtask/src/adr.rs` test module — extend the throwaway-repo harness so
`write` can create `docs/adr/drafts/…`)

- `promote_numbers_single_draft`: one draft → moved to `0001-…` (or next free),
  heading rewritten to `# ADR-0001:`, draft file gone, README row added+seeded,
  git index shows the new file staged.
- `promote_assigns_distinct_numbers_to_multiple_drafts`: two drafts →
  consecutive numbers, deterministic by slug.
- `promote_rewrites_path_form_references`: a sibling file referencing
  `docs/adr/drafts/<slug>.md` is rewritten to `docs/adr/NNNN-<slug>.md`.
- `promote_resolves_draft_referencing_another_draft`: draft B references
  `drafts/<A-slug>` → rewritten to A's assigned number.
- `promote_is_noop_without_drafts`: empty drafts dir →
  `"no ADR drafts to promote"`, no tree change.
- `promote_picks_next_after_committed_adr`: repo already has `0005-x.md` → draft
  becomes `0006-…`.

**Run**

- Write tests first: `devtool run -- cargo nextest run -p xtask promote_` → FAIL
  (compile/red).
- Implement; `devtool run -- cargo nextest run -p xtask promote_` → PASS;
  `adr_promote_parses` → PASS.
- `devtool run -- cargo xtask check` → PASS.

**Commit:** `feat(adr): add \`cargo xtask adr promote\` to number drafts at ship
(#219)`

---

## Task 3 — CONTRIBUTING.md rewrite (tracked docs only) ✅ done (972f02d0)

Point the repo's contributor docs at the draft → promote →
(amend-on-late-collision) flow. Markdown only; no gate impact.

**Scope note (why skills are NOT here).** The `.claude/skills/*` files are
_untracked_ local scaffolding that lives in the **main** repo and is shared by
every session and worktree — not part of this branch or PR. Rewriting
`jaunder-adr`/`jaunder-start`/`jaunder-ship` now would advertise
`cargo xtask adr promote` + the drafts pen to sessions running against `main`,
where none of that exists yet. So the skill edits are **deferred to post-merge**
(see the Post-merge section). `CONTRIBUTING.md` is tracked and lands in the same
PR as the code, so it becomes accurate exactly when the code merges — it stays.

**Files**

- `CONTRIBUTING.md` "Adding an ADR" — replace the always-0000 paragraph with the
  draft/promote/amend flow; keep the gate/`sync-readme`/collision paragraphs;
  note drafts are gate-invisible by construction. **(done)**

**Run**

- `devtool run -- cargo xtask check --no-test` → PASS (markdown/format checks,
  no logic).

**Commit:**
`docs(adr): document the draft-out-of-git → promote-at-ship flow (#219)`

---

## Task 4 — Dogfood: author the decision ADR as a draft ✅ done (untracked draft; promoted at ship)

Record the decision using the new flow. The ADR lives as an **untracked** draft
on this branch; `jaunder-ship` promotes it (assigning the real number) as the
first real exercise of `promote`.

**Files**

- `docs/adr/drafts/adr-out-of-git-draft-workflow.md` — the drop-in ADR body from
  the spec's "Proposed ADR" section, heading
  `# ADR-DRAFT: ADRs drafted out of git, numbered at ship`, Issue field set to
  `#219`. Annotate/supersede ADR-0036's authoring half per `jaunder-adr` status
  rules (record that in the ADR body; the actual status edit to 0036 lands as a
  tracked change at promote/ship time).

**Run / verify**

- Nothing to commit here — the draft is gitignored by design. Verification that
  `promote` graduates it correctly is covered by Task 2's tests; the live
  promotion is a `jaunder-ship` step (assigns the next free number post-rebase,
  then amend if a late collision).

**Commit:** none (draft is untracked; promoted at ship).

---

## Ship (via `jaunder-ship`, after plan approval + execution)

- Final rebase onto origin/main.
- `cargo xtask adr promote` → the draft ADR gets its real number (it landed as
  `0048-adr-out-of-git-draft-workflow.md` — promote first assigned `0047`, but
  issue #162's ADR merged during CI and took `0047`, so `cargo xtask adr
  renumber` bumped this one to `0048` and the amend folded it into the
  introducing commit — the residual-race path, exercised for real), README
  synced, staged.
- Apply the deferred ADR-0036 status annotation as a tracked edit.
- `cargo xtask validate --no-e2e` green; archive spec + plan; push; open PR
  referencing #219; release issue Status → Done.

## Post-merge (local skills — only after #219 lands on `main`)

The `.claude/skills/*` files are untracked, main-repo-local, and shared across
every session/worktree, so they must not describe `promote`/the drafts pen until
that code is on `main`. **Once the PR merges**, update them (in the main
checkout, not a worktree) to the draft → promote → amend flow:

- `jaunder-adr/SKILL.md` — replace the always-0000 authoring flow: template →
  `docs/adr/drafts/<slug>.md`; reference by draft path;
  `cargo xtask adr promote` at ship; `renumber` **and amend** on a later
  collision. Single source of truth.
- `jaunder-start/SKILL.md` — step 3/decisions: a task's ADR is authored as a
  numberless draft in `docs/adr/drafts/`, numbered at ship.
- `jaunder-ship/SKILL.md` — step 6: after the final rebase, if
  `docs/adr/drafts/*.md` exist, run `cargo xtask adr promote` and commit the
  staged result; a committed ADR that collides post-rebase → `renumber` + amend.

## Self-review

- Green at every commit: Task 1/2 keep gates passing (drafts invisible; promoted
  files are canonical). Task 3/4 are docs/untracked. ✓
- No placeholders; CLI snippets are exact against `xtask/src/lib.rs`. ✓
- Every `run_promote` branch (single, multi, path-ref, draft-ref-draft, no-op,
  after-committed) has a test → coverage. ✓
- Scope is xtask + docs only; no separable concern to split out. ✓
