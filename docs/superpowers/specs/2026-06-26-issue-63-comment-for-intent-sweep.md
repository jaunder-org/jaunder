# Spec: comment existing code for intent (issue #63)

## Goal

Bring the codebase in line with the comment-for-intent convention added in #62
(`CONTRIBUTING.md` → `## Code conventions`): every non-obvious decision carries a
*why*, redundant *what*-narration is removed, and every module states what it is
for — **without** burying the code in comments. Restraint is the point: a reader
can already follow mechanics; comments must earn their place by conveying intent,
or the rationale for code that is not, at first glance, written the obvious way.
A why-comment's job is to return surprising-looking code to a state of being
obviously correct — so the reader stops, understands why the non-obvious path was
necessary, and sees that it is right.

## Deliverable & boundaries

- **Direct edits, committed per area** in the worktree. **Behavior-frozen**:
  comments and doc-comments only — zero code changes.
- **Halt before push and the PR — not before commit.** Each area is committed
  (`docs(<area>): …`) as the controller reviews it, so the user reviews committed
  local history (easier than one large uncommitted blob). The only build before
  that review is a fast `cargo check` (compile sanity for a malformed
  doc-comment); the heavy `cargo xtask validate --no-e2e` runs only when the user
  approves the push. (This deliberately relaxes `jaunder-commit`'s validate-gate
  for a comment-only change, on the user's instruction, to avoid the wait.)

## Scope

All Rust across `common`, `storage`, `server`, `web`, `hydrate` — production
**and** `#[cfg(test)]` — plus the dev-tooling crates `xtask/` and `tools/`, plus
all of `end2end/` TypeScript. (Generated files, migration SQL, and build scripts
are out of scope.)

## The restraint bar

- **Add** an intent/why comment *only* where a competent reader cannot readily
  infer why the code exists, or where the code takes a path that is not, at first
  glance, the obvious one and the comment is what makes it obviously correct.
  Prime targets: boundary
  parsing/validation, backend-parity divergences, transaction ordering /
  `SQLITE_BUSY` mitigations, deliberate trade-offs, *which* errors are swallowed
  and why, security-sensitive steps (auth, token hashing, username lowercasing),
  non-obvious workarounds (e2e timeouts/warmup/hydration).
- **Never** comment self-evident code — getters, plain mappings, obvious control
  flow, well-named functions with plain bodies. **When in doubt, leave it out.**
- **Prune**: remove or rewrite a comment that only restates adjacent code.
  **Keep** anything carrying rationale, an ADR/issue reference, or a
  non-obvious-constraint warning — even if terse.
- **Tests**: comment only a non-obvious case (what invariant/edge it guards),
  never Arrange/Act/Assert mechanics.
- **Module `//!`**: add a purpose header only where absent; do not pad existing
  ones.
- **Match** surrounding comment style and density. One comment per non-obvious
  decision; no narration of mechanics.

## Targeting signal: CRAP / complexity

`crap-manifest.json` (repo root) lists every function with `{crate, file,
function, line, crap, cyclomatic, coverage}`. High CRAP/complexity functions are,
almost by definition, where intent is most obscured. Each area's subagent is
handed its slice of the manifest (priority: `crap >= 10` or `cyclomatic >= 8`) as
**"look here first"** targets, on top of the general bar. This does not bound the
sweep — it prioritizes within it.

## Execution

- **Partition by area** with the large crates split: `common`, `storage`
  (split sqlite/postgres/core if needed), `server` (split into sub-areas — it is
  ~28.5k lines / 62 files), `web`, `hydrate`, `end2end`.
- **Parallel subagents, at most two concurrent.** Opus. Each receives: the
  restraint bar verbatim, two worked ❌/✅ examples, and its CRAP slice. Each edits
  existing files in place at absolute worktree paths and returns a concise change
  report (file → what was added/pruned + the why).
- **Controller verification.** On each return, the controller reviews the diff
  against the bar — catching over-commenting and over-pruning (a removed comment
  that carried rationale) — and spot-fixes inline.

## Risks & mitigations

- *Over-commenting / inconsistency* → explicit bar ("when in doubt, leave it
  out") + per-diff controller review.
- *Over-pruning load-bearing comments* → keep-if-rationale rule + diff scan for
  deletions.
- *Large diff hard to review* → per-area change reports; review organized by area
  even while uncommitted.
- *Malformed doc-comments breaking compilation* → caught by `cargo xtask check`
  when gates finally run; the user is told the pre-review tree is unverified-build.

## Done when

- Non-obvious code (esp. high-CRAP functions) carries intent/why comments;
  redundant mechanical narration is gone; modules have purpose headers.
- No behavior change; the gate (`cargo xtask validate --no-e2e`) is green — run
  *after* the user's diff review.
