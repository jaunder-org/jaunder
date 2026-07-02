# Plan — issue #207: canonical `docs/adr/template.md`

Spec: `docs/superpowers/specs/2026-07-02-issue-207-adr-template.md` (approved,
A+B in).

## Refinement flagged for plan approval — render-safe placeholders

The spec's decision (3) showed the template with `<angle-bracket>` placeholders
(`# ADR-0000: <Title>`, `<the forces…>`). **GitHub markdown strips unknown
HTML-like tags, so raw `<Title>` / `<the forces…>` render as _empty_ when the
file is viewed on GitHub** — and since choice A links the template from
`CONTRIBUTING.md` for humans to view, that matters. So the template will use
**render-safe placeholders**: a literal placeholder title and _italicised_ body
guidance, no raw `<…>`:

```markdown
# ADR-0000: Title of the decision

- Status: proposed
- Date: YYYY-MM-DD
- Issue: [#0](https://github.com/jaunder-org/jaunder/issues/0)

## Context

_The forces and constraints that make this a decision._

## Decision

_What we're doing, stated so a future reader can't reverse it by accident._

## Consequences

_What this commits us to; the follow-ups it creates; what it rules out._
```

Still `adr-format`-valid after a `cp` to `0000-<slug>.md` (heading number =
filename `0000`, non-empty title, `- Status: proposed`), still
`renumber`-friendly (`ADR-0000` → assigned number). **This supersedes spec
acceptance criterion 2** (byte-identity with the skill's inline copy): that copy
is _deleted_ by the post-merge repoint (Task 3), so identity is moot — the repo
template becomes the sole SSOT.

## Tasks

- [x] **1. Add `docs/adr/template.md` + `CONTRIBUTING.md` pointer.**
  - Write `docs/adr/template.md` with the render-safe content above.
  - Add a one-line pointer in `CONTRIBUTING.md`'s ADR section (e.g. "Start a new
    ADR by copying `docs/adr/template.md` to `docs/adr/0000-<slug>.md`;
    `cargo xtask adr renumber` assigns the number").
  - **Verify:** `cargo xtask check` green — `adr-format`, `adr-readme-parity`,
    `identifier-collisions` all pass with `template.md` present (not flagged, no
    README row added); prettier-clean.

- [x] **2. Add an xtask guard test locking template-invisibility.**
      (`xtask-tests` gate step runs it; teeth confirmed via the `0099-`
      inversion — `adr-format` fires.)
  - In `xtask/src/adr_readme.rs` `#[cfg(test)]`, add a test that builds a tmp
    `docs/adr/` with one numbered ADR (`0001-x.md`, valid heading+status)
    **and** a `template.md`, then asserts `format_problems(tmp)` is empty and
    the parity row-set excludes `template.md` (i.e. `template.md` is treated as
    neither an ADR nor a table row).
  - **Confirm the test has teeth:** locally invert it once (temporarily rename
    the fixture to `0099-template.md`) to see it fail, then restore — don't
    commit the inversion.
  - **Risk to check first:** confirm the gate actually _executes_ xtask's own
    unit tests (the Nix coverage pass is jaunder-crate-only and the flake
    excludes `xtask/`). If `cargo xtask check` doesn't run `-p xtask` tests, the
    guard compiles but never runs — find where PR#204's existing `adr_readme.rs`
    tests run and match that; if nothing runs them, note it and raise whether a
    `cargo test -p xtask` step belongs in the gate (out of scope to _add_ here —
    flag, don't fold).
  - **Verify:** `cargo xtask check` green.

- [ ] **3. (Post-merge, outside this repo — not a commit here.)** Repoint the
      `jaunder-adr` skill's step 2 from its inline template to
      `cp docs/adr/template.md docs/adr/0000-<slug>.md`, and update
      `project_adr_authoring_always_0000.md` / `MEMORY.md` to mark the
      deliverable done. Tracked here so it isn't lost; executed after the PR
      merges.

## Ship

- Final gate: `cargo xtask validate --no-e2e` (no e2e-affecting surface —
  decision in spec "Out of scope"). CI runs full `validate` regardless.
- One PR, closes #207. No new ADR (implementation of ADR-0036 / #196).

## Separable concerns

None. xtask-internal refactors are already tracked by #205.
