# Issue #138 — backup `#[server]` fns panic on `expect_context<UserStorage>` under authenticated SSR

**Date:** 2026-06-28
**Issue:** [#138](https://github.com/jaunder-org/jaunder/issues/138) (follow-up to #124; blocks #129)
**Status:** spec — approved design, reproduction pending

## Problem

The backup `#[server]` fns `backup_warning_visible()` (`web/src/backup/mod.rs:32,33`)
and `current_user_is_operator()` (`:66`) call `expect_context::<Arc<dyn UserStorage>>()`
(and `Arc<dyn SiteConfigStorage>`), which **panics under authenticated page-render SSR**:

```
web/src/backup/mod.rs:66 — expected context of type
"alloc::sync::Arc<dyn storage::users::UserStorage>" to be present
(reactive_graph .../owner/context.rs:306)
```

These widgets (`Sidebar` operator check, `BackupBanner`) render on every authenticated
page, so `main` hits this intermittently — a contributor to the ~80% e2e pass rate, and a
hard blocker for #129's `{backend}×{browser}` matrix. It is the same SSR-storage-context
class as #89/#124, but **#124's owner-capturing `server_resource` does not cover it.**

This bug class has recurred for months (#89 → #124 → #138). The spec therefore aims not
only to fix the two fns but to **statically prevent reintroduction of the broken pattern.**

## Investigation findings (grounding, 2026-06-28)

The open question from the issue's scoping update — *why backup uniquely panics when ~75
sibling reactive-context reads don't* — resolves to **pre-await vs. post-await**:

- **Works:** `list_user_posts` reads `expect_context::<Arc<dyn PostStorage>>()` as the
  **first statement, before any `.await`** (`web/src/posts/listing.rs:60`). The `Arc` is
  copied out before suspension, so it survives an owner drop. Proven by the existing
  `context_read_before_await_survives_owner_drop` test (`web/src/error.rs:1005`).
- **Panics:** the backup fns read storage **after `require_auth().await`**
  (`backup/mod.rs:32,66`) — a post-await reactive-context read.
- The sibling `current_user` resource survives because `require_auth` reads
  `SessionStorage` from `Parts.extensions` (the value it already holds,
  `web/src/auth/server.rs:64-67`), **not** from reactive context. `SessionStorage` is
  layered as an axum `Extension` (`server/src/lib.rs:124`); `UserStorage`/`SiteConfigStorage`
  are provided **only** as reactive context (`server/src/context.rs:26,37`).

**Unresolved puzzle (the reason for reproduction-first):** `server_boundary`
(`web/src/error.rs:419-423`) *does* wrap the body in `ScopedFuture` when an owner is
current, and the existing `server_boundary_keeps_context_alive_across_await` test
(`:1091`) passes — yet real e2e still panics. So the current deterministic harness does
**not** reproduce the real failure. Update 2's hypothesis (the owner is empty / `None` at
the real boundary, e.g. via `ScopedFuture`'s `Owner::current().unwrap_or_default()` at
`:415`) is plausible but **unconfirmed**. We do not pick a fix until a test is red.

## Resolution (implemented 2026-06-28)

Reproduction confirmed the mechanism deterministically (`owner_lifetime` test
`post_await_read_loses_ancestor_context_when_parent_owner_dropped`): the storage
contexts are provided in an **ancestor** owner (the SSR root), but `ScopedFuture`
holds a strong ref only to the **current (leaf)** owner; when the SSR runtime drops
the ancestor mid-await, the post-await ancestry walk finds nothing.

The fix is **C3 (root-cause the helper)**, not the originally-planned C1 (hoist) +
static lint. The pivot was prompted by the question "could `boundary!` handle this
internally?" and made feasible by `reactive_graph`'s `Owner::parent()`
(owner.rs:224), which upgrades the internally-*weak* parent link to a strong
`Owner`. `server_boundary` now walks `Owner::current()` to the root via
`owner_ancestry_strong()` and holds the full ancestry strong for the future's
lifetime, so **every** `#[server]` fn's post-await reactive-context read is safe —
independent of read ordering. This *eliminates* the class rather than policing it,
so the per-fn hoist and the static lint are both **dropped** (no non-working
pattern remains to reject; "can't silently return" is satisfied structurally).

- Fix: `web/src/error.rs` — `owner_ancestry_strong()` + `server_boundary` holds it.
- Guard (deterministic): `server_boundary_keeps_ancestor_context_alive_across_await`
  (red before the fix, green after) plus the characterization test above.
- Guard (end-to-end): the e2e zero-panic gate on authenticated `/` and `/posts/new`.
- Backup fns are **unchanged** (their post-await reads are now safe), so the e2e
  gate genuinely exercises the helper fix on the real path.

## Guiding principle

**Reproduction-first (TDD).** No fix is chosen, and nothing is hoisted or layered, until a
test reproduces the panic and is **red on `main`**. Candidate fixes are classified
working/non-working **by that test**, not by argument.

## Phases

### Phase 1 — Deterministic reproduction

Extend the `owner_lifetime` harness (`web/src/error.rs`) to faithfully model the backup
SSR path — the missing fidelity is that the existing tests model an owner that already
contains the context. The faithful model:

- provide storage in a **root** owner;
- create a **child** resource owner (as `Sidebar` does at `web/src/pages/ui.rs:1060`);
- build the fetcher through the real `server_resource` → `scoped_fetcher_future` →
  `server_boundary` stack, with a body that awaits (like `require_auth().await`) **then**
  does a post-await `expect_context`;
- drive it with SSR's owner-drop timing (drop strong refs between hand-polls).

If the unit harness genuinely cannot reproduce (reactive-owner internals differ under real
SSR), **escalate** to an SSR-level integration test using the existing server test harness
(authenticated render of `/`, assert no panic / the widget renders).

**Exit criterion:** a test that fails on `main`.

### Phase 2 — Root-cause

From the red test, pin the actual mechanism: Is `Owner::current()` `None` at boundary
entry? Is the captured owner empty? Is the resource owner not an ancestor of the
`provide_app_state_contexts` owner? Document the answer — it drives Phase 3 classification.

### Phase 3 — Classify every candidate empirically

Run the red test against each candidate; label each working / non-working **by the test**:

1. **Hoist** — read `UserStorage`/`SiteConfigStorage` *before* `require_auth().await`.
   (Issue warns this may be a non-fix if the captured owner is empty.)
2. **A — axum Extension mirror** — layer `Arc<dyn UserStorage>` (and `SiteConfigStorage`)
   as `Extension`s in `server/src/lib.rs`; backup fns read from `Parts.extensions` like
   `require_auth` reads `SessionStorage`. Owner-independent.
3. **B — helper-class fix** — root-cause and fix `server_boundary`/`server_resource` so any
   post-await reactive-context read survives SSR.

### Phase 4 — Choose & report all working options

Per user direction, surface **every** working option (not just the chosen one) in a matrix:

| Option | Boilerplate repetition | Substantial downsides |
|---|---|---|
| … | per-fn cost × call sites touched | e.g. A adds a layer + read-pattern per storage trait; B risks all ~75 reactive-read sites; hoist relies on per-fn discipline |

Recommend one with reasoning. **HALT for plan approval includes this comparison.**

### Phase 5 — Statically reject non-working patterns

For each option the test proved non-working — at minimum the current bug, a **post-await
reactive-context storage read in a `#[server]` fn** — add a static check that recognizes
and rejects it, wired into `cargo xtask check`'s static pass (so pre-commit/CI catch
regressions). This is also the issue's "can't silently return" acceptance criterion.

Constraints (user direction): the class has dogged the project for months, so this is
**worth real effort** — but **no false positives**, and don't "move heaven and earth."
Mechanism candidates (finalized in plan), cheapest-sound-first:

- an **ast-grep relational rule** (`expect_context`/`use_context` of a storage trait that
  `follows` an `.await`, scoped inside a `#[server]` fn), wired into the xtask static pass;
- a **syn-based xtask walker** if the relational rule can't express the ordering soundly.

**Fallback:** if no sound, false-positive-free static check is feasible, the deterministic
repro test is the guard, and the tradeoff is flagged explicitly rather than shipping a
flaky lint.

### Phase 6 — Regression guard + ADR

- The repro test (now green) is the permanent regression guard.
- Add an **ADR-0016 addendum** recording the pre-vs-post-await reactive-read rule and the
  chosen fix; add/refresh its row in `docs/README.md` if needed.

## Acceptance criteria

- Authenticated SSR of `/` and `/posts/new` renders **without panic**; the e2e zero-panic
  gate (ADR-0032) passes for the backup paths.
- A regression test covers authenticated SSR of an operator-gated backup widget so this
  class can't silently return.
- Non-working patterns identified in Phase 3 are statically rejected by `cargo xtask check`
  (or, per the Phase 5 fallback, the tradeoff is documented and the repro test guards).
- `cargo xtask validate` is green.

## Out of scope / separable concerns

- The full e2e `{backend}×{browser}` matrix is **#129** (already `blocked-by` #138); it
  resumes once this lands.
- No wholesale migration of the ~75 working pre-await reactive reads — they are correct.

## Affected files (initial)

- `web/src/backup/mod.rs` (`:32,33,66,80,128`), `web/src/backup/server.rs:9`
- `web/src/error.rs` (`owner_lifetime` harness; possibly `server_boundary`)
- `server/src/lib.rs`, `server/src/context.rs` (only if fix A)
- xtask static-check wiring + ast-grep rule (Phase 5)
- `docs/adr/0016-dependency-injection-and-appstate.md`, `docs/README.md`
