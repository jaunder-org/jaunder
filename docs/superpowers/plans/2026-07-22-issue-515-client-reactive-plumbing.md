# #515 — client reactive plumbing Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with
> **jaunder-iterate** (delegating an individual task to a subagent via
> **jaunder-dispatch** when useful — Task 2's mechanical call-site sweep is a
> good candidate). Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Delete the SSR-vestigial `server_resource`, then relocate the four
`#[client_only]` `Invalidator` helpers into the wasm-only `client` crate —
retiring the last reactive `#[client_only]` markers in `web`.

**Architecture:** Spec →
[`docs/superpowers/specs/2026-07-18-issue-515-client-reactive-plumbing.md`](../specs/2026-07-18-issue-515-client-reactive-plumbing.md).
Phase 1 (Task 2) is a pure deletion+mechanical-rewrite refactor:
`server_resource` guarded an SSR hazard that no longer exists (verified: zero
component-SSR entrypoints), so every call site becomes a bare
`leptos::prelude::Resource::new`. Phase 2 (Tasks 3–5) moves `ListState` to
`common`, adds `client::reactive` free functions (orphan rules + the
`web → client` edge forbid `impl Invalidator` in `client`), and rewrites the
sole consumer (`web::audiences`), dropping the markers.

**Tech Stack:** Rust, leptos (csr on wasm / ssr-backend on server),
`cargo xtask` gate, `cargo nextest`.

## Global Constraints

_Every task's requirements implicitly include these (verbatim from the spec):_

- **Dependency direction:** `web`/`csr` → `client`; **never** `client → web`.
  `client` may depend only on `common` (+ `leptos` via a forwarded `csr`
  feature, `serde`, and its existing `wasm-bindgen`/`web-sys`).
- **`common` gains no `leptos` dependency.** `ListState` moves there as a pure
  enum.
- **`Invalidator` core stays host-tested in `web`.**
  `new`/`notify`/`track`/`Default` and `invalidator_scope!` do not move; their
  `reactive.rs` host tests (`notify_changes_the_tracked_revision`,
  `scope_newtype_derefs_to_its_invalidator`) stay unchanged and green.
- **leptos feature exclusivity:** `client` must pull `leptos` **only** behind a
  forwarded `csr` feature (`csr = ["dep:leptos", "leptos/csr"]`), and `web`'s
  `csr` feature must forward `client/csr` — an unconditional `leptos` dep in
  `client` would let workspace unification light `leptos/csr` + `leptos/ssr`
  together, which leptos forbids.
- **No `#[client_only]` in `client`** (crate-level
  `#![cfg(target_arch = "wasm32")]` makes it empty-on-host — nothing to exempt).
- **Gate:** `cargo xtask check` (pre-commit hook runs the full check — fmt,
  clippy, Nix coverage/tests, csr wasm build) must pass clean before each commit
  (**jaunder-commit**). **No `Co-Authored-By` trailer.**

---

## Review header

**Goal:** As above — one clean deletion, then a three-commit relocation, ending
with zero reactive `#[client_only]` in `web`.

**Scope — in:** delete `server_resource`/`scoped_fetcher_future` + lift the
clippy #124 ban + rewrite 30 call sites; move `ListState` → `common`; add
`client::reactive` (4 free fns) + wire `client` deps/features; migrate
`web::audiences`; drop the 4 markers + the `use macros::client_only`.

**Scope — out:** `server_boundary`/`owner_ancestry_strong` owner-pinning (Task 1
files it as an _investigation_ follow-up); retiring the `#[client_only]` macro
itself (#520); any `leptos`-in-`common` change; touching non-`audiences`
verticals beyond the mechanical `server_resource` → `Resource::new` rename.

**Tasks:**

1. File the `server_boundary` owner-pinning **investigation** follow-up issue.
2. **Phase 1:** delete `server_resource`+`scoped_fetcher_future`, lift the #124
   ban, rewrite 30 call sites to `Resource::new`, delete the 3 dead host tests.
3. **Phase 2a:** move `ListState` to `common::list_state`; repoint `web`
   references.
4. **Phase 2b:** add `client::reactive` (4 free fns) + wire `client` deps and
   the `csr` feature-forwarding chain.
5. **Phase 2c:** migrate `web::audiences` to `client::reactive`; delete the 4
   `Invalidator` helper methods + the `#[client_only]` markers/import.

**Key risks/decisions:**

- **CSR-safety of deleting `scoped_fetcher_future`** (spec Problem §): safe
  because no CSR fetcher reads leptos reactive context _after_ an `.await` (the
  fetchers are server-fn HTTP calls). Verified at plan time; re-confirm no
  fetcher regressed during Task 2 by spot-reading the rewritten sites.
- **leptos csr/ssr unification** (Global Constraints): the Task 4 feature wiring
  is the load-bearing bit — get the `web csr → client csr → leptos/csr` chain
  right or the wasm build breaks / the host build double-lights leptos features.
- **Wasm-only code isn't host-compiled:** the Task 2 call-site rewrites (in
  `component.rs`/`pages/*`) and all of `client::reactive` are validated by the
  csr wasm build (`cargo build -p csr --target wasm32-unknown-unknown`, run
  inside `cargo xtask check`), not by host `cargo test`. Do not treat a green
  host build as proof.

---

### Task 1: File the `server_boundary` owner-pinning investigation follow-up

**Files:** none (tracker only).

**Interfaces:**

- Produces: [#594](https://github.com/jaunder-org/jaunder/issues/594) (filed),
  referenced by spec AC #11.

- [x] **Step 1: File the issue** (**jaunder-issues**)
  - Title:
    `web: investigate whether server_boundary owner-pinning (#89/#138) is SSR-vestigial`
  - Milestone: `client crate: browser glue out of web` (M14).
  - Labels: `web`.
  - Body (substance): `#515` established there is no component SSR (zero
    render/hydrate entrypoints), which made `server_resource`'s
    `scoped_fetcher_future` owner-pinning dead. `server_boundary` +
    `owner_ancestry_strong` (`web/src/error.rs`) apply the _same_ #89/#138
    owner-pinning mechanism to `#[server]`-fn bodies. Investigate whether it too
    is now vestigial — **note the open question**: server-fn bodies may still
    read leptos context (storage trait objects) across `.await`, unlike SSR
    rendering, so this is an investigation, not a presumed deletion. If
    vestigial, remove `server_boundary`'s wrap + `owner_ancestry_strong` + the
    `owner_lifetime` `server_boundary_*` tests; if not, document why it survives
    CSR-only.

- [x] **Step 2:** recorded #594 in this plan and spec AC #11. Docs edits ride
      with Task 2's commit.

---

### Task 2 (Phase 1): Remove SSR-vestigial `server_resource`

One atomic commit — the deletion, the ban lift, the 30 call-site rewrites, and
the 3 test deletions must land together or the crate won't compile.

**Files:**

- Modify: `web/src/error.rs` — delete `server_resource` (:126) +
  `scoped_fetcher_future` (:114); delete 3 tests in the `owner_lifetime` module
  (`server_resource_constructs_under_owner` :630,
  `scoped_fetcher_future_keeps_context_across_owner_drop` :757,
  `post_await_read_loses_ancestor_context_when_parent_owner_dropped` :819).
- Modify: `web/src/lib.rs` — delete `pub use error::server_resource;`.
- Modify: `clippy.toml` — delete the `leptos::prelude::Resource::new`
  `disallowed-methods` entry (:14).
- Modify (call sites, `crate::server_resource(…)` →
  `leptos::prelude::Resource::new(…)`): `web/src/reactive.rs:59` (inside
  `Invalidator::resource`); `web/src/posts/component.rs` (12 sites:
  286,423,1044,1169,1260,1344,1519,1666,1672,1943,2123,2301);
  `web/src/media/component.rs` (200,205); `web/src/backup/component.rs`
  (12,134); `web/src/home/component.rs` (38);
  `web/src/registration/component.rs` (36); `web/src/pages/invites.rs` (19,20);
  `web/src/pages/email.rs` (15,84); `web/src/pages/ui.rs` (78,95);
  `web/src/pages/profile.rs` (14,129); `web/src/pages/sessions.rs` (10);
  `web/src/pages/cockpit.rs` (32); `web/src/pages/site.rs` (12). Total: **30**
  sites.
- Test: no new tests. Existing `error.rs`/`reactive.rs` host tests + the
  audiences e2e are the safety net; the refactor is behavior-preserving.

**Interfaces:**

- Consumes: `leptos::prelude::Resource::new(source, fetcher)` — the exact
  replacement (same `(source, fetcher)` shape; drop only the
  `scoped_fetcher_future` wrap).
- Produces: `Invalidator::resource` (still `#[client_only]` in `web` until
  Task 5) now calls `Resource::new` directly. No `server_resource` symbol
  remains.

- [x] **Step 1: Rewrite the 30 call sites.** Mechanical:
      `crate::server_resource(a, b)` → `leptos::prelude::Resource::new(a, b)`.
      (Good jaunder-dispatch candidate — brief the subagent with the exact
      file:line list above and forbid `ctx_*`.) `action` is **not** a
      `server_resource` caller — do not touch it.

- [x] **Step 2: Delete the producers + ban + re-export.** Remove
      `server_resource` and `scoped_fetcher_future` from `error.rs`; remove the
      `pub use error::server_resource;` line from `lib.rs`; remove the
      `Resource::new` entry from `clippy.toml`.

- [x] **Step 3: Delete the 3 dead host tests** in `error.rs`'s `owner_lifetime`
      module (named above). Leave
      `server_boundary_keeps_ancestor_context_alive_across_await` (:864) and
      every other `owner_lifetime` test untouched — they exercise
      `server_boundary` or leptos's own `ScopedFuture`, not the removed symbols.

- [x] **Step 4: Verify removal + green gate.** (Deviation: all 30 sites
      normalized to bare `Resource::new` — the glob import
      `use leptos::prelude::*;` is present in all 13 files and the long path
      tripped `too_many_lines` in `profile.rs`; bare matches the repo's import
      discipline.)

  Run: `rg 'server_resource|scoped_fetcher_future' web/src` → Expected: **no
  matches**. Run: `cargo xtask check` → Expected: **PASS** (host clippy +
  tests + coverage + the `build-csr` wasm compile that validates the wasm-only
  call-site rewrites). Spot-check: re-read 3–4 rewritten fetchers to confirm
  none reads leptos context post-`.await` (CSR-safety, spec Problem §).

- [x] **Step 5: Commit** (run the gate clean first — **jaunder-commit**).

```bash
git add web/src/error.rs web/src/lib.rs web/src/reactive.rs clippy.toml web/src/posts web/src/media web/src/backup web/src/home web/src/registration web/src/pages
git commit -m "refactor(web): delete SSR-vestigial server_resource; call Resource::new directly (#515)"
```

---

### Task 3 (Phase 2a): Move `ListState` to `common`

**Files:**

- Create: `common/src/list_state.rs` — the `ListState` enum (moved verbatim from
  `web/src/reactive.rs:170-184`, minus any leptos coupling — it has none).
- Modify: `common/src/lib.rs` — add `pub mod list_state;` (alphabetical, after
  `invite`).
- Modify: `web/src/reactive.rs` — delete the local `ListState` definition;
  `patched`'s return type becomes `Signal<common::list_state::ListState>`; add
  `use common::list_state::ListState;` (or reference fully-qualified).
- Modify: `web/src/audiences/component.rs:13` — source `ListState` from
  `common::list_state` instead of `crate::reactive`.

**Interfaces:**

- Produces: `common::list_state::ListState` —
  `#[derive(Clone, Debug)] pub enum ListState { Loading, Empty, Loaded, Error(String) }`
  with its existing doc comment. No leptos, no serde required.

- [x] **Step 1: Create `common/src/list_state.rs`** with the enum + doc comment,
      and add the `pub mod list_state;` line to `common/src/lib.rs`.

- [x] **Step 2: Repoint `web`.** Delete the enum from `reactive.rs`; update
      `patched`'s signature and body reference to
      `common::list_state::ListState`; update the `audiences/component.rs`
      import.

- [x] **Step 3: Verify green.**

  `ListState` is a derive-only enum — no unit test to add; the
  `reactive`/`audiences` recompiles are the behavioral check. Run:
  `cargo xtask check` → Expected: **PASS** (confirms `common` gained no leptos
  dep — a `leptos` import in `common` would fail the wasm dep guards and
  clippy).

- [x] **Step 4: Commit.**

```bash
git add common/src/list_state.rs common/src/lib.rs web/src/reactive.rs web/src/audiences/component.rs
git commit -m "refactor(common,web): move ListState to common::list_state (#515)"
```

---

### Task 4 (Phase 2b): Add `client::reactive` + wire deps and the `csr` chain

**Files:**

- Create: `client/src/reactive.rs` — the four free functions (below).
- Modify: `client/src/lib.rs` — add `pub mod reactive;` (after
  `pub mod storage;`).
- Modify: `client/Cargo.toml` — add `common`, `serde`, optional `leptos`, and
  the `csr` feature.
- Modify: `web/Cargo.toml` — extend the `csr` feature:
  `csr = ["leptos/csr", "client/csr"]`.

**Interfaces:**

- Consumes: `common::list_state::ListState` (Task 3); leptos `Resource`,
  `ServerAction`, `Effect`, `RwSignal`, `Signal`, `ServerFn`.
- Produces (`client::reactive`, all free functions — no `Invalidator` type
  crosses the crate boundary; the `Invalidator` core supplies `track`/`notify`
  as closures):

```rust
pub fn resource<T, Fut>(
    track: impl Fn() -> u32 + Send + Sync + 'static,
    fetch: impl Fn() -> Fut + Send + Sync + 'static,
) -> Resource<T>
where
    T: serde::Serialize + serde::de::DeserializeOwned + Send + Sync + 'static,
    Fut: std::future::Future<Output = T> + Send + 'static;

pub fn action<A>(notify: impl Fn() + Send + Sync + 'static) -> ServerAction<A>
where
    A: ServerFn + Send + Sync + Clone + 'static,
    A::Output: Send + Sync + 'static,
    A::Error: Send + Sync + 'static;

pub fn patched<T, Fut, E>(
    track: impl Fn() -> u32 + Send + Sync + 'static,
    fetch: impl Fn() -> Fut + Send + Sync + 'static,
    patch: impl Fn(Vec<T>) + 'static,
) -> Signal<ListState>
where
    T: Clone + serde::Serialize + serde::de::DeserializeOwned + Send + Sync + 'static,
    E: Clone + std::fmt::Display + serde::Serialize + serde::de::DeserializeOwned + Send + Sync + 'static,
    Fut: std::future::Future<Output = Result<Vec<T>, E>> + Send + 'static;

pub fn sticky<T, Fut, E>(
    track: impl Fn() -> u32 + Send + Sync + 'static,
    fetch: impl Fn() -> Fut + Send + Sync + 'static,
) -> Signal<Option<Result<T, E>>>
where
    T: Clone + serde::Serialize + serde::de::DeserializeOwned + Send + Sync + 'static,
    E: Clone + serde::Serialize + serde::de::DeserializeOwned + Send + Sync + 'static,
    Fut: std::future::Future<Output = Result<T, E>> + Send + 'static;
```

Bodies are the current `Invalidator::{resource,action,patched,sticky}` bodies
(`web/src/reactive.rs:53-161`) with `self.track()` → `track()`, `this.notify()`
→ `notify()`, and the internal `self.resource(fetch)` calls →
`resource(track, fetch)` / inline
`Resource::new(move || track(), move |_| fetch())`. No behavior change — the
helpers are e2e-exercised (audiences flows in CI), not host-unit-tested.

- [ ] **Step 1: Wire `client/Cargo.toml`.**

```toml
[dependencies]
wasm-bindgen = { workspace = true }
web-sys = { workspace = true, features = ["Window", "Storage"] }
common = { workspace = true }
serde = { workspace = true }
leptos = { workspace = true, optional = true }

[features]
csr = ["dep:leptos", "leptos/csr"]
```

- [ ] **Step 2: Forward `web`'s `csr` feature** to `client`: in
      `web/Cargo.toml`, `csr = ["leptos/csr", "client/csr"]`.

- [ ] **Step 3: Write `client/src/reactive.rs`** with the four functions to the
      exact signatures above (bodies ported from `web/src/reactive.rs`), and add
      `pub mod reactive;` to `client/src/lib.rs`. Import leptos +
      `common::list_state::ListState`.

- [ ] **Step 4: Verify the wasm build compiles `client::reactive`** (host build
      leaves `client` empty, so it proves nothing here).

  Run: `cargo build -p csr --target wasm32-unknown-unknown` → Expected: **PASS**
  (the csr → web[csr] → client[csr] → leptos/csr chain resolves;
  `client::reactive` type-checks on wasm). Run: `cargo xtask check` → Expected:
  **PASS** (host build: `client` empty-on-host, no leptos pulled → no csr/ssr
  unification collision).

- [ ] **Step 5: Commit.**

```bash
git add client/Cargo.toml client/src/lib.rs client/src/reactive.rs web/Cargo.toml Cargo.lock
git commit -m "feat(client): add client::reactive Invalidator helper free fns (#515)"
```

---

### Task 5 (Phase 2c): Migrate `web::audiences`; retire the `#[client_only]` markers

**Files:**

- Modify: `web/src/audiences/component.rs` — rewrite the 8 helper call sites
  (patched :54; sticky :62,:282; action :144,:224,:225,:280,:281) to
  `client::reactive::*`, threading `move || inv.track()` /
  `move || inv.notify()`.
- Modify: `web/src/reactive.rs` — delete the four `Invalidator` methods
  `resource` (:53), `action` (:70), `patched` (:97), `sticky` (:144); delete the
  four `#[client_only]` attributes and the `use macros::client_only;` import
  (:15). Keep `new`/`notify`/`track`/`Default` + `invalidator_scope!`.

**Interfaces:**

- Consumes: `client::reactive::{resource, action, patched, sticky}` (Task 4);
  `Invalidator::{track, notify}` (unchanged core).
- Produces: zero reactive `#[client_only]` in `web`.

- [ ] **Step 1: Rewrite the audiences call sites.** Examples (exact per current
      code):
  - `:54`
    `let state = list.patched(list_my_audiences, move |rows| store.audiences().patch(rows));`
    →
    `let state = client::reactive::patched(move || list.track(), list_my_audiences, move |rows| store.audiences().patch(rows));`
  - `:62` `let subscribers: RosterSignal = roster.sticky(list_my_subscribers);`
    → `client::reactive::sticky(move || roster.track(), list_my_subscribers)`
  - `:224` `let rename_action = list.action::<RenameAudience>();` →
    `let rename_action = client::reactive::action::<RenameAudience>(move || list.notify());`
  - `:144` — the inline-`expect_context` site: **bind first**, then thread the
    closure:
    ```rust
    let list = expect_context::<AudienceList>();
    let create_action = client::reactive::action::<CreateAudience>(move || list.notify());
    ```
    Apply the same shape to `:225`, `:280`, `:281`, `:282`. `AudienceList`
    derefs to `Invalidator`, so `list.track()`/`list.notify()` resolve.
    (`Invalidator` is `#[derive(Clone, Copy, Debug)]`, so a reused local —
    `list`, `roster`, `members` — survives capture into several `move` closures;
    do not drop that `Copy`.)

- [ ] **Step 2: Delete the four helper methods + markers** from `reactive.rs`;
      remove `use macros::client_only;`. Confirm `Invalidator`'s core +
      `invalidator_scope!` + the two host tests remain.

- [ ] **Step 3: Verify zero markers + green gate + e2e.**

  Run: `rg '#\[client_only\]|macros::client_only' web/src` → Expected: **no
  matches** (the `forms/field.rs:200` prose comment is not an attribute; if `rg`
  still shows only that comment line, that's acceptable — but the
  attribute/import form must be gone). Run:
  `cargo build -p csr --target wasm32-unknown-unknown` → Expected: **PASS**.
  Run: `cargo xtask check` → Expected: **PASS**. Run:
  `cargo xtask validate --no-e2e` → Expected: **PASS** (spec AC #10's named
  conformance gate — verify-only). Audiences revalidation e2e runs in CI's
  matrix; local e2e is reaped here, so let CI gate it.

- [ ] **Step 4: Commit.**

```bash
git add web/src/audiences/component.rs web/src/reactive.rs
git commit -m "refactor(web): consume client::reactive; retire the last reactive #[client_only] (#515)"
```

---

## Self-review (done at write time)

- **Spec coverage:** AC1→T5; AC2/AC3→T2; AC4→T2/T4/T5 (wasm build); AC5→T2;
  AC6→T4/T5; AC7→T3; AC8→T3/T5 (core untouched); AC9→T4; AC10→T5 (CI e2e);
  AC11→T1. All 11 mapped.
- **Placeholders:** none — every step has exact files, signatures, and commands.
  (`#TBD` in T1 is the issue number produced _by_ T1, resolved in its Step 2.)
- **Type consistency:** `ListState` is `common::list_state::ListState` from T3
  onward; the T4 free-fn signatures match the T5 call sites (`track`/`notify`
  closures, `action::<A>(notify)`); `Invalidator::{track,notify}` names are
  stable.
