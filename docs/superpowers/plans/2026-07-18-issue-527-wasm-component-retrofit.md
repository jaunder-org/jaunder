# Issue #527 — wasm-only component retrofit Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with jaunder-iterate
> (delegating individual tasks to a subagent via jaunder-dispatch when useful).
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Retrofit the already-converged web verticals (ui leaves, audiences,
backup, subscriptions, tags, feed_discovery, forms) to ADR-0070's four-file
host/wasm split (as amended by #530): `#[component]` UI wasm-only in
`component.rs`, `#[server]` endpoints + wire DTOs in `api.rs`, `mod.rs` wiring
only, pure logic ungated and host-tested.

**Architecture:** A behavior-preserving refactor — code moves between files and
gains `#[cfg(target_arch = "wasm32")]` on `mod`/re-export wiring lines; no logic
changes. Each vertical becomes a directory (`mod.rs` + `api.rs`/`server.rs`/
`component.rs` as needed); `ui/` is dissolved into top-level leaf directories.
`mod.rs` re-exports keep every external call-site and server-fn-registrar path
(`web::<vertical>::<Type>`) byte-stable, so consumers don't move.

**Tech Stack:** Rust, Leptos (`#[component]`/`#[server]`), cargo, `cargo xtask`.

**Spec:**
`docs/superpowers/specs/2026-07-18-issue-527-wasm-component-retrofit.md` — read
it for the decisions (D1–D6) and acceptance criteria (AC1–AC7). This plan is the
"how"; the spec is the "what/why."

## Review header

**Scope (in):** the seven touched modules + their consumer path updates
(`render/mod.rs`, `pages/`, `lib.rs`) + this issue's doc slice (ADR-0070 §5
addendum, style-guide §6). **Scope (out):** #530's api.rs doc amendments (§8,
ADR Decision-1); `pages/` retrofit; the #520 xtask gate; `#[client_only]`
retirement. No behavior changes.

**Tasks (one line each):**

1. `tags` → api split (`api.rs` + wiring `mod.rs`; simplest, establishes the
   pattern).
2. `subscriptions` → api split (`api.rs` + `server.rs` + wiring `mod.rs`).
3. `feed_discovery` → directory (`labels.rs` pure + `component.rs` + wiring
   `mod.rs`).
4. `forms` → directory (`field.rs` pure + `component.rs` + wiring `mod.rs`).
5. `backup` → four-file (`api.rs`; rename `ui.rs`→`component.rs`; keep
   `server.rs`; wiring `mod.rs`).
6. `audiences` → four-file (`api.rs` + `server.rs` + `component.rs` w/ stores;
   wiring `mod.rs`).
7. `ui/` dissolved → top-level `avatar`/`icon`/`taglist`/`topbar` dirs +
   consumer sweep.
8. Docs: ADR-0070 §5 addendum + style-guide §6 (the `ui/` dissolution record).

**Key risks / decisions:**

- **Generated-type re-exports (registrar).** Each `mod.rs` must re-export the
  `#[server]` macro's generated **PascalCase** types (`ListTags`,
  `CreateAudience`, `SubscribeTo`, …), not just the snake_case fns — the
  registrar (`server/tests`) resolves
  `register_explicit::<web::<vertical>::<Type>>()` by real path. An omitted type
  re-export fails the registrar test binary's compile; the #426 syn-gate
  (matches by leaf name) would NOT catch it. This is the main trap.
- **`api.rs` is unconditional `mod api;`** (dual-compiled) — that's why `tags`'
  host `#[cfg(test)]` tests still run on host and why the client stub + host
  handler both exist. Never gate `mod api;`.
- **Tasks 4↔5 are coupled (discovered during execution).** `backup/ui.rs` is
  the sole HOST consumer of `ValidatedInput`; gating `ValidatedInput` wasm-only
  (Task 4) breaks the host build until backup's UI is gated (Task 5). They must
  land together (or backup first). Executed order: backup retrofit applied, then
  both committed in one atomic commit. The plan's Task 4-before-5 numbering is
  the wrong dependency order.
- **Coordination with #530.** #530's ADR/§8 amendment (`bcf095cc`) is not yet on
  `main`. Land #530 first, then rebase this branch so Task 8's §5/§6 edits apply
  atop the amended text. Set `#527` blocked-by `#530` before shipping.
- **`AudienceSummary` is dual-role** (wire DTO + keyed-store row): it stays in
  `audiences/api.rs` carrying its `Store`/`Patch` derives; `AudienceListData` in
  `component.rs` references `super::api::AudienceSummary` (Task 6).

## Global Constraints

- **Behavior-preserving refactor:** no new behavior, so **no new tests** are
  written. The safety net is the existing tests + the gate staying green after
  every task. Do not delete or weaken any existing test; relocate it with the
  code it covers.
- **Per-task gate (fast loop):** `cargo xtask check --no-test` — host static +
  `clippy` + **`wasm-clippy`** (`-p web -p client`, wasm target) + tools/xtask
  lints; auto-fixes fmt. Must be green before commit.
- **Host tests:** `cargo nextest run -p web` (package `web`) must stay green.
  Because default check skips `#[cfg(feature="server")]` web code, also confirm
  `cargo check -p web --all-features --all-targets` compiles after any task that
  moves `#[server]` code (#397 gotcha).
- **Registrar build (the generated-type-re-export gate):** the server-fn
  registrar (`server/tests/helpers/mod.rs`, package `jaunder`) resolves
  `web::<vertical>::<GeneratedType>` by **real path** — `cargo check -p web`
  never compiles it, so a missing generated-type re-export compiles clean in
  `web` and fails only here. After any api-touching task (1, 2, 5, 6), run
  `cargo check -p jaunder --tests --all-features` (this is also what the
  pre-commit `cargo xtask check` exercises). The #426 syn-gate matches by leaf
  name and will **not** catch an omitted re-export.
- **`reactive_stores::Field` ≠ `forms::Field`:** `audiences` imports
  `reactive_stores::{Field, Patch, Store}` (`mod.rs:29`) for its keyed store — a
  different `Field` from `forms::Field`. When splitting audiences (Task 6),
  route `reactive_stores` imports by where each type lands; never substitute
  `forms::Field`.
- **Commit:** the pre-commit hook runs the full `cargo xtask check`; run it
  first so it passes clean (**jaunder-commit**). One clean commit per task. **No
  `Co-Authored-By` trailer.**
- **`target_arch` discipline (AC1):** every `#[cfg(target_arch = "wasm32")]`
  introduced sits on a `mod` declaration or its paired `pub use component::{…}`
  re-export — never on an item inside a file.
- **`mod.rs` wiring-only (AC2):** a retrofitted vertical's `mod.rs` contains
  only `mod`/`pub use` lines — no `#[server]` fn, DTO, `#[component]`, or pure
  logic.
- **Worktree:** all paths under
  `.claude/worktrees/issue-527-wasm-component-retrofit`; branch
  `worktree-issue-527-wasm-component-retrofit`. Review base tag
  `wt-base-issue-527` (`git diff wt-base-issue-527..HEAD`).

---

### Task 1: `tags` → api split

Establishes the `api.rs` + wiring-`mod.rs` pattern on the simplest module (1
endpoint, 1 DTO, host tests, no server helper, no component).

**Files:**

- Create: `web/src/tags/api.rs`
- Modify: `web/src/tags/mod.rs` → wiring only
- No consumer files change (re-exports keep
  `tags::{TagSummary, list_tags, DEFAULT_TAG_LIMIT, MAX_TAG_LIMIT, ListTags}`
  stable).

**Interfaces:**

- Produces (re-exported from `tags/mod.rs`, unchanged paths):
  `tags::TagSummary`, `tags::list_tags`, the generated `tags::ListTags`,
  `tags::DEFAULT_TAG_LIMIT`, `tags::MAX_TAG_LIMIT`.

- [ ] **Step 1: Move the API surface into `api.rs`.** Move verbatim from
      `tags/mod.rs` into new `tags/api.rs`: the imports
      (`common::tag::{Tag, TagLabel}`, `leptos::prelude::*`,
      `leptos::server_fn::codec::Json`, `serde::{Deserialize, Serialize}`, the
      `#[cfg(feature="server")] use {std::sync::Arc, storage::PostStorage}`,
      `crate::error::WebResult`), the two consts, `pub struct TagSummary`, the
      `#[server(endpoint="/list_tags", input=Json)] pub async fn list_tags`, and
      the whole `#[cfg(test)] mod tests`. No code edits — pure relocation.

- [ ] **Step 2: Reduce `mod.rs` to wiring.** `web/src/tags/mod.rs` becomes
      exactly:

  ```rust
  //! Tag autocomplete endpoint + wire DTO. (module doc may stay here)
  mod api;

  pub use api::{list_tags, ListTags, TagSummary, DEFAULT_TAG_LIMIT, MAX_TAG_LIMIT};
  ```

  (Confirm the generated server-fn struct is named `ListTags`; if the
  `#[server]` macro derives a different PascalCase name, re-export that exact
  name.)

- [ ] **Step 3: Verify structure + gates.**
  - `rg -n '#\[server\]|struct TagSummary' web/src/tags/mod.rs` → no matches
    (wiring only).
  - `rg -n 'pub use api::' web/src/tags/mod.rs` → includes `ListTags`.
  - Run: `cargo xtask check --no-test` → PASS (host + wasm-clippy).
  - Run: `cargo nextest run -p web tags` → PASS (host tests
    `tag_label_validation_agrees_client_and_server`,
    `tag_summary_preserves_casing_with_canonical_slug` still run on host).
  - Run: `cargo check -p web --all-features --all-targets` → compiles (server
    handler).
  - Run: `cargo check -p jaunder --tests --all-features` → compiles (proves the
    registrar path `web::tags::ListTags` still resolves — `cargo check -p web`
    does NOT compile the registrar).

- [ ] **Step 4: Commit.**

  ```bash
  git add web/src/tags/
  git commit -m "refactor(web): split tags endpoints into tags/api.rs (#527)"
  ```

---

### Task 2: `subscriptions` → api split

Adds the `server.rs` half of the pattern (a server-only helper + server-gated
tests). No component, no DTO.

**Files:**

- Create: `web/src/subscriptions/api.rs`, `web/src/subscriptions/server.rs`
- Modify: `web/src/subscriptions/mod.rs` → wiring only

**Interfaces:**

- Produces (re-exported, unchanged paths):
  `subscriptions::{subscribe_to, unsubscribe_from, is_subscribed_to}` and the
  generated server-fn types `SubscribeTo`, `UnsubscribeFrom`, `IsSubscribedTo`.

- [ ] **Step 1: Create `server.rs` (host-only support).** Move into
      `web/src/subscriptions/server.rs`: the `resolve_author(...)` helper
      (currently `mod.rs:~32`) and the whole
      `#[cfg(all(test, feature = "server"))] mod tests` block. `server.rs` is
      declared `#[cfg(feature="server")]` from `mod.rs`, so its items need no
      per-item feature gate. **Change `resolve_author` to
      `pub(crate) async fn`** (so `api.rs`, a sibling module, can reach it via
      `use super::server::…`). Keep its body verbatim. Give `server.rs` the
      imports `resolve_author` uses: `crate::error::InternalError`,
      `common::ids::UserId`, `common::username::Username`,
      `storage::UserStorage`.

- [ ] **Step 2: Create `api.rs` (the 3 endpoints).** Move the three `#[server]`
      fns (`subscribe_to`, `unsubscribe_from`, `is_subscribed_to`) into
      `web/src/subscriptions/api.rs`. At its top:

  ```rust
  use crate::error::WebResult;
  use common::username::Username;
  use leptos::prelude::*;
  #[cfg(feature = "server")]
  use super::server::resolve_author;              // the pub(crate) helper
  #[cfg(feature = "server")]
  use {std::sync::Arc, crate::auth::require_auth,
       storage::{SubscriptionStorage, UserStorage}};
  ```

  The bodies call `resolve_author(...)` (from `server.rs`), `require_auth()`,
  `expect_context::<Arc<dyn …Storage>>()`, and
  `common::visibility:: ViewerIdentity::local(...)` (via full path). Bodies
  otherwise verbatim (keep `boundary!(...)`). `super::server::*` is NOT used for
  the external-crate imports — those live in `api.rs`'s own grouped
  `#[cfg(feature="server")]` block above.

- [ ] **Step 3: Reduce `mod.rs` to wiring.**

  ```rust
  //! (keep the module doc-comment describing the subscribe/unsubscribe surface)
  mod api;
  #[cfg(feature = "server")]
  mod server;

  pub use api::{
      is_subscribed_to, subscribe_to, unsubscribe_from,
      IsSubscribedTo, SubscribeTo, UnsubscribeFrom,
  };
  ```

  (Verify the generated PascalCase names against the `#[server]` expansion.)

- [ ] **Step 4: Verify + commit.**
  - `rg -n '#\[server\]|fn resolve_author' web/src/subscriptions/mod.rs` → none.
  - `cargo xtask check --no-test` → PASS.
  - `cargo check -p web --all-features --all-targets` → compiles.
  - `cargo check -p jaunder --tests --all-features` → compiles (registrar paths
    `web::subscriptions::{SubscribeTo, UnsubscribeFrom, IsSubscribedTo}`
    resolve).
  - `cargo nextest run -p web --all-features subscriptions` → PASS (the
    server-gated `is_subscribed_to_returns_false_when_viewing_own_profile`
    test).

  ```bash
  git add web/src/subscriptions/
  git commit -m "refactor(web): split subscriptions into api.rs + server.rs (#527)"
  ```

---

### Task 3: `feed_discovery` → directory (pure logic extracted, components gated)

Establishes the no-server directory shape: pure-logic file + wasm-only
`component.rs` + wiring `mod.rs`.

**Files:**

- Create: `web/src/feed_discovery/mod.rs`, `web/src/feed_discovery/labels.rs`,
  `web/src/feed_discovery/component.rs`
- Delete: `web/src/feed_discovery.rs`
- Modify: `web/src/render/mod.rs:214` (intra-doc link) — repoint
  `web::feed_discovery::surface_label` →
  `web::feed_discovery::labels::surface_label`.

**Interfaces:**

- Produces: `feed_discovery::{FeedDiscovery, RsdDiscovery}` (gated re-exports,
  unchanged paths for the wasm-only `pages/` consumers);
  `feed_discovery::labels::{surface_label, rsd_href}` (pure, host).

- [ ] **Step 1: `labels.rs` — pure logic + tests.** Move
      `fn rsd_href(username: &Username) -> String` and
      `fn surface_label(surface: &FeedSurface) -> String` and the whole
      `#[cfg(test)] mod tests` (5 tests) into
      `web/src/feed_discovery/labels.rs`. Make both fns `pub(crate)` (currently
      private) so `component.rs` can call them via `super::labels::…`. Imports:
      `common::feed::FeedSurface`, `common::username::Username`, and in the test
      module `use super::*;`.

- [ ] **Step 2: `component.rs` — the two components.** Move
      `#[component] pub fn FeedDiscovery(...)` and
      `#[component] pub fn RsdDiscovery(...)` (with their
      `#[expect(clippy::needless_pass_by_value, …)]` attrs) into
      `web/src/feed_discovery/component.rs`. Imports:
      `common::feed::{canonicalize, FeedFormat, FeedSurface}`,
      `common::username::Username`, `leptos::prelude::*`, `leptos_meta::Link`,
      and `use super::labels::{rsd_href, surface_label};`. No cfg inside the
      file.

- [ ] **Step 3: `mod.rs` — wiring.**

  ```rust
  //! Feed / RSD auto-discovery <link> tags + their pure URL/label helpers.
  mod labels;
  #[cfg(target_arch = "wasm32")]
  mod component;

  #[cfg(target_arch = "wasm32")]
  pub use component::{FeedDiscovery, RsdDiscovery};
  ```

  (`labels` is not re-exported publicly unless a consumer needs it; the
  `render/mod.rs` doc-link references `feed_discovery::labels::surface_label` by
  path, which resolves to the module even without a re-export.)

- [ ] **Step 4: `lib.rs` — declaration unchanged.** `pub mod feed_discovery;`
      (line 26) still works for the directory. No edit needed. Delete the old
      `web/src/feed_discovery.rs`.

- [ ] **Step 5: Verify + commit.**
  - `rg -n '#\[component\]' web/src/feed_discovery/` → only in `component.rs`.
  - `cargo xtask check --no-test` → PASS (wasm-clippy compiles the components;
    rustdoc-link lint on the repointed `render/mod.rs:214` link resolves).
  - `cargo nextest run -p web feed_discovery` → PASS (the 5 host tests).

  ```bash
  git add web/src/feed_discovery/ web/src/render/mod.rs
  git rm web/src/feed_discovery.rs
  git commit -m "refactor(web): split feed_discovery into labels.rs + component.rs (#527)"
  ```

---

### Task 4: `forms` → directory

**Files:**

- Create: `web/src/forms/mod.rs`, `web/src/forms/field.rs`,
  `web/src/forms/component.rs`
- Delete: `web/src/forms.rs`
- No consumer files change (`forms::{Field, field_error, ValidatedInput}` stay
  via re-exports).

**Interfaces:**

- Produces: `forms::Field<T>`, `forms::field_error` (pure, host, ungated);
  `forms::ValidatedInput` (gated component re-export).

- [ ] **Step 1: `field.rs` — pure state + validator + tests.** Move into
      `web/src/forms/field.rs`:
      `pub fn field_error<T>(input: &str) -> Option<String>`,
      `pub struct Field<T>` with its hand-written `Copy`/`Clone`/ `Default`
      impls and the whole `impl Field<T>` block (`new`, `prefilled`, `optional`,
      `optional_prefilled`, `error_for`, `is_valid`, `parsed`, `is_touched`,
      `touch`, `reset`), and the whole `#[cfg(test)] mod tests` (the
      `Owner`-scoped `Field` tests + `field_error` tests). Copy the imports the
      code uses (the validation trait bound, `leptos` signal types,
      `PhantomData`, etc.) verbatim from `forms.rs`.

- [ ] **Step 2: `component.rs` — `ValidatedInput`.** Move
      `#[component] pub fn ValidatedInput<T>(...)` into
      `web/src/forms/component.rs`, with `use super::field::Field;` and the
      `leptos` imports it needs. No cfg inside the file.

- [ ] **Step 3: `mod.rs` — wiring.**

  ```rust
  //! Client-side form primitives: Field<T> state, its validator, and the
  //! ValidatedInput widget.
  mod field;
  #[cfg(target_arch = "wasm32")]
  mod component;

  pub use field::{field_error, Field};
  #[cfg(target_arch = "wasm32")]
  pub use component::ValidatedInput;
  ```

- [ ] **Step 4: `lib.rs`** — `pub mod forms;` (line 28) unchanged. Delete
      `web/src/forms.rs`.

- [ ] **Step 5: Verify + commit.**
  - `rg -n '#\[component\]' web/src/forms/` → only in `component.rs`.
  - `cargo xtask check --no-test` → PASS.
  - `cargo nextest run -p web forms` → PASS (the `Field`/`field_error` host
    tests).

  ```bash
  git add web/src/forms/
  git rm web/src/forms.rs
  git commit -m "refactor(web): split forms into field.rs + component.rs (#527)"
  ```

---

### Task 5: `backup` → four-file

Already `mod.rs` + `server.rs` + `ui.rs`. Add `api.rs`, rename `ui.rs` →
`component.rs`, shrink `mod.rs` to wiring.

**Files:**

- Create: `web/src/backup/api.rs`
- Rename: `web/src/backup/ui.rs` → `web/src/backup/component.rs` (`git mv`)
- Modify: `web/src/backup/mod.rs` → wiring only; keep `web/src/backup/server.rs`

**Interfaces:**

- Produces (unchanged paths): `backup::{BackupBanner, BackupSettingsPage}`
  (gated component re-exports); the 4 `#[server]` fns + their generated types
  (`BackupWarningVisible`, `CurrentUserIsOperator`, `GetBackupSettings`,
  `UpdateBackupSettings` — verify exact names) + any wire types.

- [ ] **Step 1: `api.rs` — endpoints + wire imports.** Move the four `#[server]`
      fns (`/backup_warning_visible`, `/current_user_is_operator`,
      `/get_backup_settings`, `/update_backup_settings`, with their
      `#[cfg_attr(feature="server", …)]` attrs verbatim) into
      `web/src/backup/api.rs`. Carry the ungated wire imports
      (`use crate::error:: WebResult;`,
      `use common::backup::{BackupConfig, BackupMode, BackupSchedule, RetentionCount};`
      — the typed `#[server]` args, dual-compiled — and
      `use leptos::prelude::*;`). Replace the `mod.rs`-level
      `use server::require_operator;` with
      `#[cfg(feature = "server")] use super::server::*;` at the top of `api.rs`
      (`require_operator` is already `pub(crate)` in `server.rs`, so `*` reaches
      it). Backup defines no web-side wire DTO struct (its settings types come
      from `common::backup`), so there is no DTO to relocate. Keep the endpoint
      bodies verbatim.

- [ ] **Step 2: `git mv ui.rs component.rs`.**
      `git mv web/src/backup/ui.rs web/src/backup/component.rs`. The two
      `#[component]`s (`BackupSettingsPage`, `BackupBanner`) stay verbatim
      inside; the file gains no cfg (its `mod` declaration carries the gate).
      Update its internal `use crate::forms::…` etc. only if paths changed (they
      didn't — forms re-exports are stable).

- [ ] **Step 3: `mod.rs` — wiring.**

  ```rust
  //! Backup settings vertical: operator-gated settings endpoints + the banner/
  //! settings-page UI.
  mod api;
  #[cfg(feature = "server")]
  pub(crate) mod server;              // preserve pub(crate) — do not drop to `mod server;`
  #[cfg(target_arch = "wasm32")]
  mod component;

  pub use api::{
      backup_warning_visible, current_user_is_operator, get_backup_settings,
      update_backup_settings,
      BackupWarningVisible, CurrentUserIsOperator, GetBackupSettings, UpdateBackupSettings,
  };
  #[cfg(target_arch = "wasm32")]
  pub use component::{BackupBanner, BackupSettingsPage};
  ```

  Delete the now-stale `mod.rs:8–11` comment block documenting the old
  "components ungated … host-compiled for coverage" arrangement (D3).

- [ ] **Step 4: Verify + commit.**
  - `rg -n '#\[server\]|#\[component\]' web/src/backup/mod.rs` → none.
  - `rg -n 'ungated|host-compiled for coverage' web/src/backup/mod.rs` → none.
  - `cargo xtask check --no-test` → PASS.
  - `cargo check -p web --all-features --all-targets` → compiles.
  - `cargo check -p jaunder --tests --all-features` → compiles (registrar paths
    `web::backup::{BackupWarningVisible, CurrentUserIsOperator, GetBackupSettings, UpdateBackupSettings}`
    resolve).
  - `cargo nextest run -p web --all-features backup` → PASS (server.rs tests).

  ```bash
  git add web/src/backup/
  git commit -m "refactor(web): split backup into api.rs + component.rs, mod.rs wiring (#527)"
  ```

---

### Task 6: `audiences` → four-file

The most complex: 8 endpoints, 2 DTOs, 5 components, keyed-store types,
server-gated tests. Single `audiences/mod.rs` today.

**Files:**

- Create: `web/src/audiences/api.rs`, `web/src/audiences/server.rs`,
  `web/src/audiences/component.rs`
- Modify: `web/src/audiences/mod.rs` → wiring only

**Interfaces:**

- Produces (unchanged paths): the 8 `#[server]` fns + generated types
  (`CreateAudience`, `RenameAudience`, `DeleteAudience`, `ListMyAudiences`,
  `ListMySubscribers`, `AddSubscriberToAudience`,
  `RemoveSubscriberFromAudience`, `ListAudienceMembers` — verify against
  expansion); DTOs `AudienceSummary`, `SubscriberSummary`; components
  `AudiencesPage`, `CreateAudienceForm`, `AudienceRow`, `AudienceHeader`,
  `MemberChecklist` (gated).

- [ ] **Step 1: `api.rs` — endpoints + DTOs (incl. dual-role
      `AudienceSummary`).** Move into `web/src/audiences/api.rs`:
      `pub struct AudienceSummary` **with its `Serialize`/`Deserialize` AND
      `derive(Store, Patch)` derives intact** (dual-role — spec D6),
      `pub struct SubscriberSummary`, and the 8 `#[server]` fns. Top of
      `api.rs`:
  - ungated wire imports (`use crate::error::WebResult;`, `use common::…` for
    the DTO field types, `use leptos::prelude::*;`);
  - `use reactive_stores::{Patch, Store};` — **only** the derives
    `AudienceSummary` needs (NOT `Field`; `reactive_stores::Field` goes to
    `component.rs` with `AudienceListData`);
  - the endpoint bodies' server-side imports, carried verbatim from the current
    `mod.rs:32-38` block:

    ```rust
    #[cfg(feature = "server")]
    use {
        crate::auth::require_auth,
        common::ids::UserId,
        std::sync::Arc,
        storage::{AudienceStorage, SubscriptionStorage, UserStorage},
    };
    ```

  Do **not** add `use super::server::*` — audiences' `server.rs` holds only the
  test module (no named helpers to import). Bodies verbatim (keep `boundary!`).

- [ ] **Step 2: `server.rs` — server-only support + server-gated tests.** Move
      the vertical's `#[cfg(feature="server")]` helpers (whatever the current
      grouped `#[cfg(feature="server")]` block at `mod.rs:~32` supports beyond
      raw imports) and the `#[cfg(all(test, feature = "server"))] mod tests`
      block into `web/src/audiences/server.rs`. If audiences has no named server
      helper (only imports), `server.rs` holds just the test module; keep it
      (ADR: server.rs is warranted by the unit tests). `api.rs`'s
      `use super::server::*;` covers the imports.

- [ ] **Step 3: `component.rs` — components + store types.** Move the 5
      `#[component]`s and the keyed-store types `AudienceListData`
      (`reactive_stores` `Store`, keyed on `AudienceSummary`) and `AudienceList`
      (generated by the `invalidator_scope!` macro) into
      `web/src/audiences/component.rs`. `AudienceListData` references the DTO as
      `super::api::AudienceSummary`. Imports at the top:
  - `use super::api::{…};` for the endpoints + DTOs the components call;
  - `use reactive_stores::{Field, Store};` — the store-side
    `reactive_stores::Field` (**not** `forms::Field`);
  - `use crate::forms::Field as FormField;` **only if** a component uses the
    form `Field<T>` — alias it to avoid the `reactive_stores::Field` name clash
    (check the moved component bodies; alias only if both are referenced);
  - `use crate::ui::Topbar;` — **keep this `crate::ui::` path**; `ui/` still
    exists until Task 7, whose sweep repoints it to `crate::topbar::Topbar`. (Do
    not write `crate::topbar::` here — that module doesn't exist yet, would be
    RED.)
  - the leptos imports. No cfg inside the file. **Stale-comment fix (D3):** the
    moved `AudiencesPage` carries an inline-`<svg>` justification comment
    (`mod.rs:305-307`) whose premise — `<Icon>` is "unreachable from this
    dual-target module … relocates under #312" — is now false (`component.rs` is
    wasm-only). Update the comment to drop the dual-target/#312 reasoning (state
    simply that the inline `<svg>` is retained). **Do NOT switch the `<svg>` to
    `<Icon>`** — that is an out-of-scope behavior change.

- [ ] **Step 4: `mod.rs` — wiring.**

  ```rust
  //! Audiences vertical: audience/subscriber endpoints + wire DTOs, and the
  //! management UI.
  mod api;
  #[cfg(feature = "server")]
  mod server;
  #[cfg(target_arch = "wasm32")]
  mod component;

  pub use api::{
      add_subscriber_to_audience, create_audience, delete_audience,
      list_audience_members, list_my_audiences, list_my_subscribers,
      remove_subscriber_from_audience, rename_audience,
      AddSubscriberToAudience, CreateAudience, DeleteAudience, ListAudienceMembers,
      ListMyAudiences, ListMySubscribers, RemoveSubscriberFromAudience, RenameAudience,
      AudienceSummary, SubscriberSummary,
  };
  #[cfg(target_arch = "wasm32")]
  pub use component::{
      AudienceHeader, AudienceRow, AudiencesPage, CreateAudienceForm, MemberChecklist,
  };
  ```

  (`AudienceSummary`/`SubscriberSummary` re-exported ungated — they're wire DTOs
  used by `render`/`taglist`-adjacent host code and the `#[server]` returns.)

- [ ] **Step 5: Verify + commit.**
  - `rg -n '#\[server\]|#\[component\]|struct AudienceSummary' web/src/audiences/mod.rs`
    → none.
  - `cargo xtask check --no-test` → PASS (wasm-clippy compiles the 5
    components + store types).
  - `cargo check -p web --all-features --all-targets` → compiles.
  - `cargo check -p jaunder --tests --all-features` → compiles (all 8 registrar
    paths `web::audiences::CreateAudience` … `ListAudienceMembers` resolve).
  - `cargo nextest run -p web --all-features audiences` → PASS.

  ```bash
  git add web/src/audiences/
  git commit -m "refactor(web): split audiences into api/server/component, mod.rs wiring (#527)"
  ```

---

### Task 7: dissolve `ui/` → top-level leaf directories + consumer sweep

Promote each `ui/` leaf to a top-level directory (`mod.rs` wiring + `markup.rs`
pure + `component.rs` gated), delete `ui/`, and repoint every `crate::ui::`
consumer.

**Files:**

- Create per leaf `L ∈ {avatar, icon, taglist, topbar}`: `web/src/L/mod.rs`,
  `web/src/L/markup.rs`, `web/src/L/component.rs`
- Delete: `web/src/ui/` (all of `mod.rs`, `avatar.rs`, `icon.rs`, `taglist.rs`,
  `topbar.rs`)
- Modify: `web/src/lib.rs` (module declarations), `web/src/render/mod.rs`
  (imports + doc-links), `web/src/pages/ui.rs` + `web/src/pages/mod.rs`
  (re-exports/imports), `web/src/audiences/component.rs` +
  `web/src/backup/component.rs` (the `crate::ui::Topbar` →
  `crate::topbar::Topbar` repoint).

**Interfaces:**

- Produces: `crate::avatar::{render, avatar_parts, Avatar}`,
  `crate::icon::{render, Icon, Icons}`, `crate::taglist::{render, TagList}`,
  `crate::topbar::{render, Topbar}` — pure `render`/`avatar_parts`/`Icons`
  ungated, the components gated.

- [ ] **Step 1: Build the four leaf directories.** For each leaf `L`, from the
      current `web/src/ui/L.rs`:
  - `web/src/L/markup.rs` — the pure `pub(crate) fn render(...)` twin, plus
    `avatar_parts` (avatar only), plus the leaf's `#[cfg(test)] mod tests`
    (unit/parity tests). Keep item names identical (`render` stays `render`).
  - `web/src/L/component.rs` — the `#[component]` (`Avatar`/`Icon`/`TagList`/
    `Topbar`) verbatim, with `use super::markup::…;` for any pure helper it
    calls (e.g. avatar's component uses `avatar_parts`) and its leptos imports.
    No cfg inside.
  - `web/src/L/mod.rs` — wiring:

    ```rust
    mod markup;
    #[cfg(target_arch = "wasm32")]
    mod component;

    pub use markup::render;                    // + avatar_parts for avatar (pub(crate))
    #[cfg(target_arch = "wasm32")]
    pub use component::Avatar;                 // Icon / TagList / Topbar per leaf
    ```

  - **icon special-casing:** `icon/mod.rs` also re-exports `Icons` **ungated**:
    `pub use crate::render::Icons;` (definition stays in `render/mod.rs`). So
    `icon::Icons` (host) and `icon::Icon` (gated) split as the spec requires.
    `icon/markup.rs`'s `render` may reference `Icons` — keep that reference via
    `crate::render::Icons`.

- [ ] **Step 2: `lib.rs` module declarations.** Replace `pub mod ui;` (line 44)
      with:

  ```rust
  pub mod avatar;
  pub mod icon;
  pub mod taglist;
  pub mod topbar;
  ```

  (Alphabetical placement among the existing `pub mod` list.)

- [ ] **Step 3: Repoint `render/mod.rs` (host projector).**
  - `web/src/render/mod.rs:18`:
    `use crate::ui::{avatar, icon, taglist, topbar};` →
    `use crate::{avatar, icon, taglist, topbar};`. The `avatar::render` /
    `icon::render` / `taglist::render` / `topbar::render` / `Icons::SEARCH`
    call-sites are unchanged (names stable).
  - Doc-links referencing `crate::ui::Icon` / `crate::ui::icon::render` (`:493`)
    and any `pages::ui::…` prose → repoint to `crate::icon::Icon` /
    `crate::icon::render`. `rg -n 'crate::ui' web/src/render/mod.rs` must return
    nothing after.

- [ ] **Step 4: Repoint the remaining `crate::ui::` consumers.**
  - `web/src/pages/ui.rs:31`:
    `pub use crate::ui::{Avatar, Icon, Icons, TagList, Topbar};` →
    `pub use crate::{avatar::Avatar, icon::{Icon, Icons}, taglist::TagList, topbar::Topbar};`
    (all inside wasm-only `pages/`, so gating is implicit). Update the stale
    section comment above it (`:29`, "moved to web::ui (strangler shims, #522)")
    to reflect the top-level leaf modules. Update its doc-links (`:24,:26`)
    `crate::ui::taglist::render` / `crate::ui::TagList` → `crate::taglist::…`.
  - `web/src/pages/mod.rs:16`: the `pub use ui::{…}` there is `pages::ui`
    (unchanged); confirm it still resolves to the repointed re-exports.
  - `web/src/audiences/component.rs` and `web/src/backup/component.rs`:
    `use crate::ui::Topbar;` → `use crate::topbar::Topbar;`.
  - Final sweep: `rg -n 'crate::ui\b|\bui::' web/src` returns only `pages::ui`
    references (the distinct page-composite module), never the deleted top-level
    `ui`.

- [ ] **Step 5: Delete `ui/` and verify.** `git rm -r web/src/ui/`.
  - `rg -n '#\[component\]' web/src/{avatar,icon,taglist,topbar}` → only in each
    `component.rs`.
  - `cargo xtask check --no-test` → PASS (host projector compiles against the
    pure twins; wasm-clippy compiles the leaf components).
  - `cargo nextest run -p web` → PASS (avatar/icon/taglist/topbar unit + parity
    tests run on host).
  - `cargo check -p web --all-features --all-targets` → compiles.

- [ ] **Step 6: Commit.**

  ```bash
  git add web/src/avatar/ web/src/icon/ web/src/taglist/ web/src/topbar/ \
          web/src/lib.rs web/src/render/mod.rs web/src/pages/ \
          web/src/audiences/component.rs web/src/backup/component.rs
  git rm -r web/src/ui/
  git commit -m "refactor(web): dissolve ui/ into top-level leaf directories (#527)"
  ```

---

### Task 8: Docs — ADR-0070 §5 addendum + style-guide §6

Record the `ui/` dissolution (this issue's doc slice; #530 owns §8 /
Decision-1).

**Files:**

- Modify: `docs/adr/0070-web-vertical-wasm-only-component-files.md` (§5
  addendum)
- Modify: `docs/web-style-guide.md` §6 ("Shared components")

- [ ] **Step 1: ADR-0070 §5 addendum.** Under Decision point 5 (or as a dated
      amendment Note near the header, matching #530's Note style), add: shared
      presentation leaves (`avatar`, `icon`, `taglist`, `topbar`) are
      **top-level directory modules** (`mod.rs` wiring + `markup.rs` pure twin +
      wasm-only `component.rs`), not a `ui/` sub-tree — `ui/` is dissolved
      (#527). Keep Status `accepted`. Do not touch Decision point 1/§2/§7 (those
      are #530's).

- [ ] **Step 2: style-guide §6.** In `docs/web-style-guide.md` §6 ("Shared
      components"): change "leaf primitives live in `web/src/ui/`" (`:123`) and
      "lift it into `web/src/ui/`" (`:147`) to name the top-level leaf modules
      (`web/src/avatar/`, `web/src/icon/`, …) and the markup/component split.
      Leave the `pages/ui.rs` line (§6 mentions #312) intact.

- [ ] **Step 3: Verify + commit.**
  - `prettier -w docs/web-style-guide.md docs/adr/0070-web-vertical-wasm-only-component-files.md`
    before staging (pre-commit prettier restages prose otherwise).
  - `rg -n 'web/src/ui/' docs/web-style-guide.md` → no matches (except any
    intentional `pages/ui.rs`).
  - `cargo xtask check --no-test` → PASS (doc-only; sanity).

  ```bash
  git add docs/adr/0070-web-vertical-wasm-only-component-files.md docs/web-style-guide.md
  git commit -m "docs: record ui/ dissolution — ADR-0070 §5 addendum + style-guide §6 (#527)"
  ```

---

## Final verification (before ship)

- [ ] `cargo xtask validate --no-e2e` → green (full static + coverage gate).
- [ ] `cargo xtask validate` → green (adds e2e all four backend×browser combos;
      the touched surfaces — audiences, backup, feed-discovery, forms fields,
      tag autocomplete, leaf widgets — behave unchanged).
- [ ] `git diff wt-base-issue-527..HEAD --stat` review: only the intended files;
      no stray `crate::ui`, no ungated `#[component]`, every vertical `mod.rs`
      wiring only.
- [ ] Set `#527` blocked-by `#530` (native GitHub dependency) and coordinate
      land order (#530 first, then rebase this branch) per the spec's
      coordination note.

## Self-review notes

- **Spec coverage:** D1 (gated re-exports) → every task's `mod.rs`. D2 (ui
  dissolution) → Task 7 + Task 8. D3 (cleanup + doc fixes) → Task 5 Step 3
  (backup comment) + Task 8. D4 → Task 3. D5 → Task 4. D6 → Tasks 1,2,5,6.
  AC1/AC2 → per-task `rg` checks. AC3 → host-test steps. AC4 → Task 7 Step 4
  sweep. AC5 → Tasks 5,6. AC6 → Task 8. AC7 → Final verification.
- **Type consistency:** the generated `#[server]` PascalCase type names in the
  re-export lists (`ListTags`; `SubscribeTo`/`UnsubscribeFrom`/`IsSubscribedTo`;
  `BackupWarningVisible`/`CurrentUserIsOperator`/`GetBackupSettings`/
  `UpdateBackupSettings`; the 8 audiences types) are copied verbatim from the
  registrar `server/tests/helpers/mod.rs:32-84` — the authoritative list — so
  they match by construction.
- **No new behavior:** confirmed — every task relocates code and adds wiring
  cfgs; the existing tests are the safety net.
