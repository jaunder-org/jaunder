# Spec — issue #942: `mod.rs` assembles the module surface, nothing more

**Issue:** [#942](https://github.com/jaunder-org/jaunder/issues/942) — "Only use
`mod.rs` for assembling the module" **Milestone:** Correctness & data integrity
**Branch:** `issue-942-mod-rs-assembly-only` (fork-point tag
`issue-942-mod-rs-assembly-only-base`)

## Problem

`mod.rs` has two jobs in this repo and does them badly at once. It is supposed
to state a module's surface — what submodules exist, what the module exports —
which is the first thing a reader (human or agent) looks at to orient. In 17 of
the 53 `mod.rs` files it also holds implementation, so the surface is buried in
the body and the file has no single reason to change.

`server/src/atompub/mod.rs` is a fair sample: 498 lines carrying the router, an
axum metrics middleware, three handler guards, an error enum, ten `From` impls,
and a test module — and, underneath all of it, the five `pub mod` lines that are
what the file is actually named for.

ADR-0070 already states the rule — "module wiring only", no items of its own —
but binds it to `web/` **verticals**. The two `web/` offenders, `web/src/error/`
and `web/src/reactive/`, are precisely the non-vertical support directories
(`docs/ARCHITECTURE.md:928` identifies them as such), so they are outside
ADR-0070's reach and are not violations of it. That is the gap: the rule is
right, and its scope is an accident of where it was first written down. This
issue **extends** it to the workspace rather than enforcing it where it already
applied.

## The rule (normative)

A `mod.rs` may contain **only**:

- `mod` / `pub mod` declarations,
- `use` / `pub use` / `pub(crate) use` re-exports,
- `//!` module-level documentation,
- attributes — inner (`#![cfg(…)]`, `#![allow(…)]`) and outer attributes on the
  `mod` and `use` items above.

Everything else is forbidden: `fn`, `struct`, `enum`, `trait`, `impl`, `const`,
`static`, `type` (including public aliases), `macro_rules!`, and any inline
`#[cfg(test)] mod tests { … }` body.

The rule is **workspace-wide with no exemptions** — production crates, test
trees, `xtask/`, and `tools/` alike. An exemption list is the part that rots,
and the two worst offenders are both in `server/tests/`, so exempting tests
would exempt the problem.

### Consequences that are part of the rule

- **Public paths do not change.** Code moved out of a `mod.rs` is re-exported
  from it, so existing `use crate::foo::Item` call sites are untouched.
- **Re-exports are explicit**, `pub use thing::{A, B};`, not `pub use thing::*;`
  — a glob states nothing, and stating the surface is the point. A glob is
  permitted **only** when a single sibling's export list exceeds 25 items, and
  the commit message must state the count. Within this branch's scope exactly
  one sibling can qualify (`common/src/test_support`'s largest split); every
  other re-export is an explicit list.
- **`pub(crate)` items re-export as `pub(crate) use`.** A bare `pub use` of a
  `pub(crate)` item does not compile.
- **Gated wiring stays in `mod.rs`.** The `target-arch-placement` check permits
  a `target_arch` cfg only on a `mod` or `use` item in a `mod.rs`/`lib.rs`, so
  those gated lines are assembly by definition and do not move.
- **Docs follow their subject.** An item's `///` doc comment moves with the
  item, always. The `//!` module narrative stays in `mod.rs` by default, since
  it describes the module; a paragraph of it that describes only relocated code
  moves to the new sibling's own `//!` header. Where a file has no `//!` today,
  none is invented — that is separate work.

## Scope

**16 files.** The 17th, `server/tests/storage/mod.rs`, is deferred — see
"Deferred" below. The 36 `mod.rs` files that are already assembly-only are
untouched.

| File                             | New siblings                                                          |
| -------------------------------- | --------------------------------------------------------------------- |
| `client/src/perf/mod.rs`         | `names.rs`                                                            |
| `common/src/atompub/mod.rs`      | `ns.rs`, `error.rs`                                                   |
| `common/src/feed/mod.rs`         | `config.rs`                                                           |
| `common/src/test_support/mod.rs` | `identity.rs`, `content.rs`, `media.rs`, `urls_time.rs`, `numbers.rs` |
| `server/src/atompub/mod.rs`      | `router.rs`, `error.rs`, `guards.rs`                                  |
| `server/src/mailer/mod.rs`       | `factory.rs`                                                          |
| `server/src/projector/mod.rs`    | `shell.rs`, `document.rs`, `handlers.rs`                              |
| `server/src/websub/mod.rs`       | `contract.rs`, `factory.rs`                                           |
| `server/tests/helpers/mod.rs`    | `registrar.rs`, `session.rs`, `atompub.rs`, `http.rs`                 |
| `server/tests/projector/mod.rs`  | `fixtures.rs`, `permalink.rs`, `listing.rs`, `tags.rs`, `caching.rs`  |
| `storage/src/postgres/mod.rs`    | `atomic.rs`, `open.rs`                                                |
| `storage/src/sqlite/mod.rs`      | `atomic.rs`, `open.rs`                                                |
| `web/src/error/mod.rs`           | `wire.rs`                                                             |
| `web/src/reactive/mod.rs`        | `invalidator.rs`                                                      |
| `xtask/src/coverage/mod.rs`      | `model.rs`, `run.rs`                                                  |
| `xtask/src/pr/mod.rs`            | `types.rs`, `invocation.rs`, `execute.rs`                             |

Sibling names state a concern; no `inner.rs` / `impl.rs` catch-alls.

**The table fixes the sibling names; the plan fixes which item lands in which
sibling.** That per-item partition is a judgement call, and settling it in the
plan is deliberate — it is reviewed at plan approval, so by commit time the move
is mechanical again and the "pure move" property of criterion 5 is meaningful.
An implementer should not be inventing the partition at the keyboard.

Three rules decide the partition, in order:

1. **Existing banners win.** Where a file already draws `// ───` sections
   (`storage/src/{postgres,sqlite}`, `xtask/src/pr`), those are the boundaries.
2. **Cohesion beats theme purity.** Where a private helper's only consumer would
   land in a different sibling, they stay together instead — even if that makes
   a sibling less thematically tidy.
3. **Tie-break toward fewer files.** If an item plausibly belongs in two
   siblings, put it in the larger one rather than creating a third.

Visibility is widened to `pub(super)` only where an item genuinely has consumers
in two siblings, and never further — except where today's
`pub(super)`-at-`mod.rs` semantics require `pub(crate)` to preserve existing
reach (see Prep 2).

Two names are forbidden by `clippy::module_inception`, since ADR-0067
deliberately promoted these files _to_ `mod.rs` to avoid it: no
`server/tests/projector/projector.rs`, no `server/tests/storage/storage.rs`.

## Method — prep, then move

Each file is landed as **a prep commit (only if needed) followed by a pure-move
commit**. A move commit is "pure" when each new file's body is the removed lines
verbatim plus a `use` header, and `mod.rs` gains only `mod` and re-export lines.
Anything else — visibility widening, rewriting `use super::*`, reordering,
splitting an `impl` block — lands **first**, in its own commit, against the
still-intact `mod.rs`, where the diff is small and readable.

**Every commit gates green.** `.githooks/pre-commit` runs `cargo xtask check`,
so a prep commit that cannot stand on its own is not a prep commit. Two of the
three prep items fail that test and are therefore folded into their move commit,
each with the reason recorded in the commit message:

- The **per-file `expect` suppression** (Prep 3) cannot be written before the
  files it annotates exist.
- The **registrar constant** (Prep 1) cannot change alone, because
  `cargo xtask check` runs both the registrar gate and xtask's own test suite,
  and that suite `expect()`s the registrar path to be readable. Changing the
  constant without moving the file fails the hook; moving the file without
  changing the constant fails it too.

Only Prep 2 (sqlite visibility) is a genuine standalone commit.

### Prep work already identified

1. **Registrar gate.** `xtask/src/steps/server_fn_registrar_check.rs:62`
   hardcodes `const REGISTRAR: &str = "server/tests/helpers/mod.rs"` and parses
   that exact file for `register_explicit::<…>()` calls. Moving
   `ensure_server_fns_registered` to `server/tests/helpers/registrar.rs` breaks
   it. Prep: re-point `REGISTRAR` at the new path — and nothing else.

   The check is **already** fail-loud, so no hardening is needed: an unreadable
   registrar is a hard failure (`run`, lines 324–332), and a registrar that
   enumerates zero entries makes `problems()` emit a "not registered" line for
   every web `#[server]` fn (lines 286–294), so the gate goes red rather than
   silent. The unit test `enumeration_of_web_src_matches_the_registrar` asserts
   non-emptiness and count equality besides.

2. **Sqlite visibility.** `storage/src/sqlite/mod.rs` declares
   `pub(super) async fn open_sqlite_database` and `database_is_empty`. At
   `mod.rs`, `pub(super)` means "visible at the crate root", which is why
   `storage/src/db.rs:15,289` can call them. In `sqlite/open.rs` the same
   keyword means "visible in `crate::sqlite`" and those callers break. Prep:
   widen both to `pub(crate)`, re-exported `pub(crate) use open::…`.
3. **Test-support suppression** — _not_ prep; part of the move, and the one
   documented exception to the pure-move rule. `common/src/test_support/mod.rs`
   carries an inner `#![expect(clippy::expect_used)]` covering the whole
   subtree, whose own doc says it "self-flags if the scaffolding ever stops
   using `expect`". Left at the top it stays fulfilled as long as _any_
   descendant trips it, so it would outlive its reason silently. It must become
   a per-file inner `#![expect(clippy::expect_used)]` in each new sibling that
   actually calls `expect()`, omitted from those that don't — which cannot be
   written before the siblings exist, so it cannot precede the move. It
   therefore lands _in_ the move commit, and that commit's message must say so.

### Known hazards for the move commits

- `server/src/atompub`: the `From<anyhow::Error>` impl reaches
  `crate::media::map_error` — the **server crate's** `media`, not the sibling
  `atompub::media` module. The moved code must not let that path re-resolve to
  the sibling.
- `server/tests/*` and `storage`: an `#[apply(backends)]` site needs **three**
  things in scope per file (ADR-0124) — `use rstest::*;`,
  `use rstest_reuse::*;`, **and the template itself imported by bare name**
  (`use storage::test_support::backends;`). Each new sibling repeats all three;
  omitting the third leaves `#[apply(backends)]` unresolved.
- `server/src/projector`: the test module uses
  `include_str!("../../../csr/index.html")`, resolved relative to the containing
  file. A same-directory sibling keeps the path valid; a deeper nesting would
  not.
- `web/src/error`: the `FromServerFnError` impl calls
  `server::emit_arg_decode_failure` behind a statement-level
  `#[cfg(feature = "server")]`, and `mod server;` is private — so the new file
  must be a sibling _within_ `web/src/error/` to reach it.
- `storage/src/postgres/mod.rs:335` holds the only `#[apply(postgres_only)]`
  test in scope; it moves to `postgres/open.rs`, still under `postgres/`, so
  ADR-0053's homing rule still passes.
- Import preambles are partitioned per sibling; copying a whole `use` block into
  every new file trips denied `unused_imports`.

## Records

- **A new ADR** stating the rule, its workspace-wide scope, and that it is
  enforced **by review rather than by a gate** — because whether an item earns
  its own file is a judgement a syntactic check would get wrong in both
  directions. It cites ADR-0070 as precedent; ADR-0070 itself is unchanged and
  stays web-scoped. Authored as a numberless draft in `docs/adr/drafts/` per
  `jaunder-adr`, numbered at ship by `cargo xtask adr promote`. It does **not**
  name the `jaunder-review` overlay — that artifact lives in another repository
  and a cross-repo citation would rot unseen.
- **`CONTRIBUTING.md`** gains the rule for human contributors, pointing at the
  ADR.
- **`docs/ARCHITECTURE.md`** takes two changes, for two different reasons:
  1. **The new ADR is projected into it** while the draft is still numberless,
     per `jaunder-adr-projection`. `adr-view-parity` requires every accepted ADR
     to be cited there, and `promote` rewrites the draft path to the assigned
     number for free if the citation already exists — leaving it until ship
     means owing prose against a file that does not exist yet.
  2. **The 19 existing `mod.rs` citations are corrected.** 17 carry line numbers
     and are invalidated by this branch — one of those, at line 705, cites
     `mod.rs:28` with no path at all and needs one. The other 2 name a file with
     no line (lines 834, 2083) and need only their path re-pointed if the cited
     code moved. `rg -c` reports 22 `mod.rs` mentions; the other three (lines
     916, 926, 935) are prose about the layout rule rather than citations — 916
     states the rule this branch generalises and is updated to say so, and the
     other two are counts that stay true.

  A full projection _replay_ of the whole view is a separate concern and is not
  in scope.

## Acceptance criteria

1. Each of the 16 files listed in Scope contains only `mod`/`pub mod`,
   `use`/`pub use`/`pub(crate) use`, `//!` docs, and attributes. Verifiable by
   inspection of each file.
2. No public or crate-visible path changes: no call site outside the 16
   directories is edited to follow moved code. Verifiable — the branch diff
   touches no file outside the 16 module directories except
   `docs/adr/drafts/<slug>.md` (and, after `promote` at ship, its numbered path
   plus the generated `docs/README.md` table), `CONTRIBUTING.md`,
   `docs/ARCHITECTURE.md`, `docs/superpowers/{specs,plans}/`, and
   `xtask/src/steps/server_fn_registrar_check.rs`.
3. Re-exports name their items explicitly. A `pub use x::*;` appears only where
   that sibling exports more than 25 items, and its commit message states the
   count. Verifiable — grep the branch diff for `use .*::\*;` and check each hit
   against the sibling's export count.
4. The sqlite visibility widening (Prep 2) is a **separate, earlier commit**
   than its move. The other two prep items are folded into their move commit,
   each with the reason stated in the commit message. Every commit on the branch
   passes `cargo xtask check` on its own. Verifiable from `git log` and by
   checking out any commit and running the gate.
5. Each move commit's diff shows only lines leaving `mod.rs`, the same lines
   arriving in a sibling, a `use` header, and new `mod`/re-export lines in
   `mod.rs` — plus, in exactly two commits, the folded-in prep named in
   criterion 4. Verifiable by reading the commit.
6. `server_fn_registrar_check`'s `REGISTRAR` constant names the relocated
   registrar file, and `cargo xtask check` passes. Verifiable — the constant is
   a one-line read, and the gate is red if the path is wrong. No behavioural
   change to the check is in scope.
7. `REGISTERED_SERVER_FN_COUNT` still equals the enumerated count and the gate
   still passes.
8. The only `#[allow(...)]`/`#[expect(...)]` change in the branch is narrowing
   the existing `common/src/test_support` suppression: the subtree-wide inner
   `#![expect(clippy::expect_used)]` is removed from `mod.rs` and reappears as a
   per-file inner attribute **only** in those new siblings that call `expect()`.
   No other suppression is added anywhere. Verifiable by grepping the branch
   diff for `allow(`/`expect(`.
9. No new sibling is named `projector.rs` under `server/tests/projector/` or
   `storage.rs` under `server/tests/storage/`; `cargo xtask check` reports no
   `clippy::module_inception`.
10. `cargo xtask validate` passes on the branch.
11. A new ADR exists in `docs/adr/drafts/` stating the rule, its workspace-wide
    scope, and the review-not-gate enforcement decision with its rationale, and
    citing ADR-0070.
12. `CONTRIBUTING.md` states the rule and links the ADR.
13. All 19 `mod.rs` citations in `docs/ARCHITECTURE.md` resolve: the 17 with
    line numbers point at the correct file _and_ line (including line 705, which
    gains the path it currently lacks), and the 2 without a line point at the
    correct file. `docs/ARCHITECTURE.md:916` describes the rule as
    workspace-wide. Verifiable by checking each citation against the tree.
14. The new ADR is cited in `docs/ARCHITECTURE.md` with its decision projected
    into the relevant section, by path (`docs/adr/drafts/<slug>.md`) while it
    remains a draft. Verifiable — `cargo xtask adr promote` rewrites the path
    and `adr-view-parity` passes at ship.
15. No new xtask check is added.
16. An issue exists for `server/tests/storage/mod.rs`, carrying the constraints
    in "Deferred" below.

## Deferred — `server/tests/storage/mod.rs`

Filed as its own issue, as the plan's first task. At 6090 lines, 183 item
definitions and ~190 test functions it is roughly half of all the code in scope
— and it is different work: the other 16 are "lift items into a themed sibling,"
this one is "design a layout for a 190-test suite." Bundling it would block 16
straightforward files behind one hard one and make the branch unreviewable.

The issue must carry:

- ~16 natural clusters, well past the 2–4 the other files need; the file's six
  existing `// ──` banners are a starting point, not the answer (tags alone is
  ~35 tests).
- Its ~15 private helpers are shared _across_ clusters, so each needs
  `pub(super)` widening plus a `use super::fixtures::…` in every consumer.
- The single large `storage::{…30 items}` preamble must be partitioned per file
  or it trips denied `unused_imports`.
- `rstest::*` + `rstest_reuse::*` repeated per file (ADR-0124).
- No sibling named `storage.rs` (`clippy::module_inception`, ADR-0067).
- **Homing (ADR-0053):** `server/tests/storage/` is not a dialect directory and
  its tests are all dual-backend, so siblings stay concern-named (`tags.rs`,
  `media.rs`) and must not acquire `sqlite/`/`postgres/` subdirectories unless a
  test is genuinely dialect-only.

## Related work, landing elsewhere

The reviewer-side half of the enforcement decision is a `jaunder-review` overlay
of the vendored `code-review` skill. It lives in `~/src/agent-configuration`,
not this repo — the jaunder repo does not own its review skill, and `.claude/`
here is generated and untracked. It is fully specified in
`docs/superpowers/specs/2026-08-12-issue-942-jaunder-review-overlay-handoff.md`
and implemented in a separate session against that checkout. Nothing in this
branch depends on it.

## Out of scope

- Splitting `server/tests/storage/mod.rs` (deferred above).
- Any new xtask check enforcing the rule.
- A full `docs/ARCHITECTURE.md` projection replay.
- Changes to ADR-0070.
- The 36 `mod.rs` files that are already assembly-only.
- Behaviour changes of any kind: this branch moves code and does not alter it.
