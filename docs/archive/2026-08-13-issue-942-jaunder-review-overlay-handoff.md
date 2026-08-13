# Handoff — add a `jaunder-review` overlay

**Status:** ready to implement. Self-contained; the implementing agent needs no
other context.

**Target repository:** `~/src/agent-configuration` — **not** the jaunder repo.
Start the implementing session in that checkout.

**Origin:** jaunder issue #942 ("Only use `mod.rs` for assembling the module").
That issue needs a documented rule policed at review time rather than by a new
xtask gate. The rule itself lands in the jaunder repo (an ADR plus a
`CONTRIBUTING.md` entry). The reviewer-side half is this document, and it lands
here instead because the jaunder repo does not own its own review skill.

---

## 1. Why an overlay, and why not edit `code-review`

`code-review` is a vendored skill in `~/.config/claude/skills/code-review/`. It
has no source under `~/src/agent-configuration/global/.rulesync/`, so it is not
ours to edit — a local edit would be lost on its next update.

The repo already has an established pattern for exactly this: every
`jaunder-<x>` skill is a thin **overlay** of a generic `<x>` skill. It opens by
deferring to the generic skill for the discipline, then supplies only the
project's concretes. `jaunder-iterate` over `dev-cycle-iterate` is the model to
copy. `jaunder-review` becomes the first overlay of a skill we don't own, which
changes nothing structurally.

## 2. Repository mechanics — read before touching anything

Source of truth for jaunder's project skills:

```
~/src/agent-configuration/projects/jaunder/.rulesync/skills/<skill-name>/SKILL.md
```

These are fanned out into each long-lived worktree under `~/src/jaunder/` by
rulesync. **The copies in a worktree's `.claude/skills/` are generated
artifacts** — editing one there is silently reverted on the next generate, and
it is invisible to git besides (`.claude/` is excluded via
`~/src/jaunder/jaunder/.git/info/exclude`, which every linked worktree honors).

Generate, run from inside a jaunder worktree:

```
rulesync generate --input-root ~/src/agent-configuration/projects/jaunder
```

Drift check (exit 1 = stale, 0 = clean):

```
rulesync generate --input-root ~/src/agent-configuration/projects/jaunder --check --silent
```

**Never pass `--delete`.** This is called out in
`projects/jaunder/rulesync.jsonc` and it is not advisory — the generated tree
shares directories with content rulesync does not own.

## 3. Work item 1 — create the overlay

Create
`~/src/agent-configuration/projects/jaunder/.rulesync/skills/jaunder-review/SKILL.md`
with the content below. It follows the house overlay style: frontmatter with
`name` + `description`, an
`# <Title> — the jaunder overlay of \`<generic>\``heading, a paragraph deferring to the generic skill, then`##
Jaunder specifics`.

Four of the five sections below exist because the generic skill's defaults are
wrong for this repo, not merely incomplete — see §3.1 for the justification of
each. Do not trim them as boilerplate.

```markdown
---
name: jaunder-review
description:
  "Use when reviewing a jaunder branch, PR, or work in progress — the two-axis
  Standards/Spec review, with this repo's standards sources, spec locations, and
  tracker wired in."
---

# Jaunder Review — the jaunder overlay of `code-review`

Follow **`code-review`** for the discipline — pin the fixed point and confirm it
resolves before spawning anything; identify the spec source; identify the
standards sources; run the **Standards** and **Spec** axes as parallel
subagents; aggregate them under separate headings without merging or reranking.
This overlay supplies jaunder's concretes. (Mid-branch, `jaunder-iterate`
dispatches this at each deliverable boundary; at the end, `jaunder-ship` runs it
over the whole branch.)

## Jaunder specifics

- **Fixed point.** Prefer the `issue-<N>-<slug>-base` tag `jaunder-start` leaves
  at the fork point. Failing that, three-dot `git diff main...HEAD` — **never**
  two-dot, which folds everyone else's merges in as phantom changes.
- **Spec source.** `docs/superpowers/specs/*issue-<N>*` and the plan alongside
  it at `docs/superpowers/plans/*issue-<N>*`. Don't hunt `specs/` or
  `.scratch/`; they don't exist here.
- **Issue tracker.** GitHub, `jaunder-org/jaunder`, via the GitHub MCP tools —
  see **`jaunder-issues`**. `code-review` looks for
  `docs/agents/issue-tracker.md` and tells you to run
  `/setup-matt-pocock-skills` when it's absent: **ignore that**. The file does
  not exist in this repo and is not supposed to; running that command here is
  wrong.
- **Standards sources.** `CONTRIBUTING.md` is definitive (backend parity,
  coverage policy, testing requirements, DI/ADR-0016, repository layout, the
  verify ladder). `CONTEXT.md` is the domain glossary — flag a diff that names a
  domain concept with a synonym the glossary avoids. `docs/adr/` entries are
  load-bearing invariants, not history; a diff that contradicts one is a hard
  finding, so cite the ADR. `docs/ARCHITECTURE.md` is the materialized view of
  the ADR log.
- **Module assembly (ADR — `mod.rs` is assembly-only).** A `mod.rs` may contain
  only `mod`/`pub mod` declarations, `use`/`pub use` re-exports, `//!` module
  docs, and attributes. Any `fn`, `struct`, `enum`, `trait`, `impl`, `const`,
  `static`, `type`, `macro_rules!`, or inline `#[cfg(test)] mod tests { … }`
  body added to a `mod.rs` is a **hard finding** — it belongs in a sibling file
  that `mod.rs` then re-exports. Workspace-wide, no exemptions; test trees and
  `xtask/` included. Deliberately not machine-enforced: whether a given item
  earns its own file is a judgement a syntactic gate would get wrong in both
  directions, which is why it is your job.
- **Suppressions.** An `#[allow(...)]` or `#[expect(...)]` (clippy or rustc)
  needs explicit user approval per `CONTRIBUTING.md`. An unapproved one in the
  diff is a finding. Prefer the narrowest scope that works: an inner
  `#![expect(...)]` covering a whole subtree stays "fulfilled" as long as any
  descendant trips it, so it can outlive its reason silently.
- **Skip what tooling already enforces** — `code-review`'s standing rule, and it
  matters more here than in most repos, because `cargo xtask check` enforces an
  unusually long list and duplicate findings drown the real ones. Already
  machine-checked, so **do not report**: formatting (rustfmt, leptosfmt,
  prettier), clippy, `target_arch` gate placement (`target-arch-placement`),
  server-fn registration and tracing and coverage, backend-template homing on
  tests (`test-backend-pattern`, ADR-0053), thin Leptos components, intra-doc
  links, doctest fences, ADR format and README/ARCHITECTURE parity, migration
  sequence parity, and the various ident gates (raw-html, html-sink,
  xlang-literal, sqlx-newtype). Review what a human must judge.
```

### 3.1 Why each section is there

Give these to the implementing agent as rationale; don't paste them into the
skill.

| Section                | The generic default it corrects                                                                                                                                                                                                                                             |
| ---------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Fixed point            | `code-review` asks the user for one. `jaunder-start` already leaves a tag; asking is a worse answer than reading it.                                                                                                                                                        |
| Spec source            | `code-review` searches `docs/`, `specs/`, `.scratch/`. Jaunder's specs are at `docs/superpowers/specs/`, keyed by `issue-<N>`.                                                                                                                                              |
| Issue tracker          | **A live bug.** `code-review` line 18 tells the agent to run `/setup-matt-pocock-skills` when `docs/agents/issue-tracker.md` is missing. It is missing in jaunder, permanently and correctly. Without this override the reviewer is instructed to scaffold something wrong. |
| Standards sources      | `code-review` says "anything in the repo that documents how code should be written." Jaunder has five such surfaces with a defined precedence; naming them beats rediscovery, and the ADR corpus is the part an agent would otherwise miss.                                 |
| Module assembly        | The reason this overlay exists (issue #942).                                                                                                                                                                                                                                |
| Skip-what-tooling-does | The generic skill states the rule in one clause. Jaunder's gate ladder is long enough that an unprimed reviewer reliably reports formatting and clippy findings that cannot reach a PR.                                                                                     |

## 4. Work item 2 — re-point the three call sites

Three skills dispatch `code-review` by name. All three must point at the overlay
instead, or the overlay is dead code. All paths relative to
`~/src/agent-configuration/projects/jaunder/.rulesync/skills/`.

**`jaunder-iterate/SKILL.md`, line 28:**

```diff
-- **Review (deliverable boundary):** dispatch `code-review`.
+- **Review (deliverable boundary):** dispatch `jaunder-review`.
```

**`jaunder-ship/SKILL.md`, lines 21–23** — the sentence ends "Whole-branch
review runs `code-review`.":

```diff
 1. **Review / 3. gate — fork point:** three-dot `git diff main...HEAD`, or the
    `issue-<N>-<slug>-base` tag `jaunder-start` left. Whole-branch review runs
-   `code-review`.
+   `jaunder-review`.
```

**`jaunder-dispatch/SKILL.md`, lines 56–57:**

```diff
-- **The deliverable-boundary reviewer** runs via `code-review` (see
+- **The deliverable-boundary reviewer** runs via `jaunder-review` (see
   **`jaunder-iterate`**) — that is where review lives, not per task.
```

After editing, confirm no stale references remain:

```
rg -n 'code-review' ~/src/agent-configuration/projects/jaunder/.rulesync/
```

The only surviving hits should be inside `jaunder-review/SKILL.md` itself, where
naming the generic skill is the point. Note
`global/.rulesync/skills/dev-cycle-ship/SKILL.md` also mentions code-review's
Spec axis — that is the **global** tree, describes the generic skill correctly,
and must be left alone.

## 5. Work item 3 — deploy and verify

1. Run the generate command from §2 inside a jaunder worktree.
2. Confirm `.claude/skills/jaunder-review/SKILL.md` now exists there and matches
   the source.
3. Confirm the three edited skills show the new name in their generated copies.
4. Re-run generate with `--check --silent` and confirm exit 0.
5. Commit in `~/src/agent-configuration` only. Nothing in this work item
   produces a tracked change in the jaunder repo.

## 6. Acceptance criteria

- [ ] `projects/jaunder/.rulesync/skills/jaunder-review/SKILL.md` exists, with
      frontmatter `name: jaunder-review` and a description, and follows the
      overlay style of `jaunder-iterate` (defer to the generic skill, then
      `## Jaunder specifics`).
- [ ] The skill states the `mod.rs` assembly-only rule as a hard finding, scoped
      workspace-wide, and says why it is review-enforced rather than gated.
- [ ] The skill overrides `code-review`'s `docs/agents/issue-tracker.md` /
      `/setup-matt-pocock-skills` instruction, pointing at GitHub via
      `jaunder-issues`.
- [ ] The skill pins the fixed point to the `issue-<N>-<slug>-base` tag or
      three-dot diff, and the spec source to `docs/superpowers/specs/`.
- [ ] `jaunder-iterate`, `jaunder-ship`, and `jaunder-dispatch` dispatch
      `jaunder-review`; no `code-review` reference remains anywhere in
      `projects/jaunder/.rulesync/` outside `jaunder-review/SKILL.md`.
- [ ] `rulesync generate --input-root … --check --silent` exits 0 after a
      generate.
- [ ] The vendored `~/.config/claude/skills/code-review/` is **unmodified**.
- [ ] No file in any `~/src/jaunder/*` worktree is edited by hand.

## 7. Out of scope

- The ADR and the `CONTRIBUTING.md` rule for `mod.rs` — those land in the
  jaunder repo under issue #942, separately.
- Any change to the vendored `code-review` skill.
- Any other `jaunder-*` overlay.
