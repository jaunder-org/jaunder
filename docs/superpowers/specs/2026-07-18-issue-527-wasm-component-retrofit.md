# Spec — issue #527: retrofit converged verticals to the wasm-only `component.rs` layout

- Issue: [#527](https://github.com/jaunder-org/jaunder/issues/527) (amended
  2026-07-18 by [#530](https://github.com/jaunder-org/jaunder/issues/530))
- Design record:
  [ADR-0070](../../adr/0070-web-vertical-wasm-only-component-files.md), **as
  amended by #530** (four-file layout; `mod.rs` is wiring only). This cycle
  additionally amends ADR-0070 §5 — see Decision D2.
- Milestone: Web: canonical Leptos CSR convergence
- Status: draft, pending approval

## Problem

The verticals that converged under ADR-0056's old rule (`#[component]` UI
**ungated**, host-compiled as dead-but-exempt) must be retrofitted to ADR-0070's
file-level host/wasm split, **as amended by #530**:

```text
web/src/feature/
├── mod.rs        # module wiring only: mod declarations + re-exports; NO items of its own
├── api.rs        # shared wire DTOs + #[server] endpoints (dual-compiled; one grouped
│                 #   #[cfg(feature = "server")] `use super::server::*` import)
├── server.rs     # host-only support for #[server] bodies + server-gated tests (feature = "server")
└── component.rs  # #[component] UI + browser-bound code (wasm-only)
```

`#[cfg(target_arch = "wasm32")]` appears **only on module-wiring declarations**
— a `mod` declaration or its **paired re-export** (`pub use component::{…}`).
Pure, host-testable logic (`render()` twins, `Field<T>`, `field_error`, wire
codecs) stays ungated and host-tested; extraction precedes gating.

This is the **first** retrofit under ADR-0070 — #526 landed only the ADR
document and #530 only amends it — so the conventions chosen here template the
rest of the milestone.

## In scope (touched code)

- `web/src/ui/` leaf widgets: `avatar`, `icon`, `taglist`, `topbar`
- `web/src/audiences/` (vertical: components **and** api split)
- `web/src/backup/` (vertical: components **and** api split; already file-split)
- `web/src/subscriptions/` (vertical: **api split only** — no components)
- `web/src/tags/` (vertical: **api split only** — no components)
- `web/src/feed_discovery.rs`
- `web/src/forms.rs`
- Consumer path updates forced by the above (`render/mod.rs`, `pages/`,
  `lib.rs`)
- Doc fixes owned by **this** issue: ADR-0070 §5 (the `ui/` dissolution) and
  `docs/web-style-guide.md` §6 (shared-components list), plus in-file layout
  comments made stale by the change.

## Non-goals

- The **api.rs doc amendments** — ADR-0070 Decision point 1/§2 and
  `docs/web-style-guide.md` §8 — are **#530's** deliverable (already drafted in
  commit `bcf095cc`), not this cycle's. This spec consumes the amended doctrine;
  it does not restate or re-edit it.
- Retrofitting `pages/` (already wholly wasm-only via `lib.rs`'s
  `#[cfg(target_arch = "wasm32")] pub mod pages;`).
- Writing the `#520` "wiring-only" xtask gate (future).
- Retiring `#[client_only]` / the `#[component]` coverage exemption / the A1
  guard (milestone-14 endgame, #520). None appear in the touched files.
- Any behavior change to the host projector (`web/src/render/`), `#[server]`
  bodies, wire types, or validation logic. Server-fn **registrar paths**
  (`web::<vertical>::<fn>`, #426 gate) stay byte-stable via `mod.rs` re-exports.

## Decisions

### D1 — wasm-only components are exposed via gated `pub use` re-exports

Each `mod.rs` re-exports its component behind the gate so call sites stay
stable:

```rust
#[cfg(target_arch = "wasm32")]
mod component;
#[cfg(target_arch = "wasm32")]
pub use component::AudiencesPage;
```

The amended ADR-0070 §2 **explicitly blesses this**: `target_arch` may sit on "a
`mod` declaration **or its paired re-export**." No open tension, and the future
#520 gate is defined (by #530) to permit these wiring re-exports.

### D2 — the four `ui/` leaves become top-level **directory** modules; `ui/` is dissolved

Each leaf is promoted out of `ui/` into its own top-level directory, following
the wiring-only shape (no `api.rs`/`server.rs` — these are pure-logic + UI
only):

```
web/src/avatar/mod.rs        # wiring only: mod decls + re-exports
web/src/avatar/markup.rs     # ungated: avatar_parts, the pure render() twin, unit/parity tests
web/src/avatar/component.rs  # #[cfg(target_arch="wasm32")]: #[component] Avatar
```

…and likewise `icon/`, `taglist/`, `topbar/`. `web/src/ui/` (the module, its
`mod.rs`, and `pub mod ui;` in `lib.rs`) is **removed**.

- The pure file is named `markup.rs` (the pure twin produces markup strings) to
  avoid a module/fn `render` name clash; the re-exported fn keeps its name so
  `render/mod.rs` still calls `avatar::render` / `icon::render` / etc.
  unchanged. (File name is a nicety — the plan may pick a better one; the
  constraint is only that the re-exported symbol names are stable.)
- `Icons` (defined in `render/mod.rs`) is re-exported **ungated** from
  `icon/mod.rs` (`pub use crate::render::Icons;`); `Icon` is the gated component
  re-export. This preserves the issue's "`Icon` gated, `Icons` not" split.

Rationale: these four are shared **presentation leaves** (a pure `render()` twin
the host projector calls, plus a reactive twin — no `#[server]` fns, no wire
types); `ui/` added only a namespace hop. This **amends ADR-0070 §5** ("Shared
widgets (`ui/`) become wasm-gated shared component files the same way"): shared
presentation leaves are **top-level directory modules**, not a `ui/` sub-tree.
Recorded as an ADR-0070 §5 addendum and in `docs/web-style-guide.md` §6.

Note the distinct, out-of-scope `web/src/pages/ui.rs` (`crate::pages::ui`, a
wasm-only page-composite module) — only its import paths update.

### D3 — cleanup latitude: split + necessary doc fixes

The mechanical split plus doc fixes it falsifies (D2's ADR §5 addendum +
style-guide §6; `backup/mod.rs`'s "components ungated … host-compiled for
coverage" comment). No unrelated refactors.

### D4 — `feed_discovery/` becomes a directory (pure logic extracted, components gated)

`feed_discovery.rs` holds two `#[component]`s (`FeedDiscovery`, `RsdDiscovery`)
**and pure host-tested logic** — `surface_label` (`:65`), `rsd_href` (`:60`),
and five `#[cfg(test)]` unit tests (`:74–121`). No `#[server]` fns → no
`api.rs`. It splits into a directory:

- `feed_discovery/mod.rs` — wiring only: mod decls + gated `pub use` re-exports
  of `FeedDiscovery` / `RsdDiscovery`.
- `feed_discovery/labels.rs` — ungated: `surface_label`, `rsd_href`, the five
  tests.
- `feed_discovery/component.rs` — `#[cfg(target_arch="wasm32")]`: the two
  components (calling `super::labels::{surface_label, rsd_href}`).

Gating the whole module (deleting the pure fns + tests from the host build) is
rejected — it violates ADR-0070 §6.

### D5 — `forms/` becomes a directory

`forms.rs` splits into a directory (no `#[server]` fns → no `api.rs`):

- `forms/mod.rs` — wiring only: mod decls + re-exports
  (`pub use field::{Field, field_error};`, gated
  `pub use component::ValidatedInput;`).
- `forms/field.rs` — ungated: `Field<T>`, `field_error`, and the host tests
  (which exercise `Field` under an `Owner`).
- `forms/component.rs` — `#[cfg(target_arch="wasm32")]`: the sole component
  `ValidatedInput<T>`.

### D6 — api split for the four server verticals; subscriptions/tags are api-only

Move each vertical's `#[server]` endpoints + wire DTOs out of `mod.rs` into
`api.rs`; `mod.rs` becomes wiring only; re-exports keep registrar/call-site
paths stable (D1 / #530).

- **audiences** (8 `#[server]` fns, 2 DTOs
  `AudienceSummary`/`SubscriberSummary`, 5 components, the keyed-store types
  `AudienceListData` (`reactive_stores` `Store`, keyed on `AudienceSummary`
  rows) and `AudienceList` (generated by the `invalidator_scope!` macro),
  server-gated tests): `api.rs` (endpoints + DTOs + grouped
  `#[cfg(feature="server")] use super::server::*`), `server.rs` (server-only
  support + server-gated tests), `component.rs` (5 components +
  `AudienceListData`/`AudienceList`), `mod.rs` wiring.
  - **Dual-role `AudienceSummary`:** it is both a wire DTO (`Serialize`/
    `Deserialize`, returned by `list_my_audiences`) **and** a keyed-store row
    (`derive(Store, Patch)`). It stays in `api.rs` carrying its
    `reactive_stores` derives (so `api.rs` imports `reactive_stores`);
    `AudienceListData` in `component.rs` references it as
    `super::api::AudienceSummary`. This is the one place the "DTOs→api.rs /
    store types→component.rs" line has an intentional overlap.
- **backup** (4 `#[server]` fns + wire types; existing `server.rs`; `ui.rs` → 2
  components): `api.rs` (endpoints + wire types; absorbs the `mod.rs`-level
  `use server::require_operator`), keep `server.rs`, rename `ui.rs` →
  `component.rs`, `mod.rs` wiring.
- **subscriptions** (3 `#[server]` fns, server-only helper `resolve_author`,
  server-gated tests, **no components, no DTOs**): `api.rs` (the 3 endpoints),
  `server.rs` (`resolve_author` + the server-gated tests), `mod.rs` wiring. No
  `component.rs`.
- **tags** (1 `#[server]` fn `list_tags`, DTO `TagSummary`, consts
  `DEFAULT_TAG_LIMIT`/`MAX_TAG_LIMIT`, **host** tests, **no components**):
  `api.rs` (endpoint + DTO + consts + the `#[cfg(test)]` host tests, which run
  on host since `api.rs` is dual-compiled), `mod.rs` wiring. No `server.rs`, no
  `component.rs`.

## Acceptance criteria

Each criterion is stated so ship-time conformance review can tell delivered from
not.

### AC1 — no ungated `#[component]`; every `target_arch` cfg is a wiring line

- `rg -n '#\[component\]'` across the touched modules shows every `#[component]`
  inside a file whose `mod` declaration carries `#[cfg(target_arch = "wasm32")]`
  (a `component.rs`).
- Every `target_arch = "wasm32"` cfg introduced sits on a `mod` declaration or
  its paired `pub use component::{…}` re-export — never on a `#[component]`, a
  struct, or an fn directly inside a file.

### AC2 — every retrofitted vertical's `mod.rs` is wiring-only; endpoints live in `api.rs`

- `audiences/mod.rs`, `backup/mod.rs`, `subscriptions/mod.rs`, `tags/mod.rs`
  contain only `mod`/`pub use` lines (module wiring + re-exports) — no
  `#[server]` fn, no DTO, no `#[component]`, no pure logic of their own.
- Each vertical's `#[server]` endpoints and wire DTOs live in that vertical's
  `api.rs`; `api.rs` carries at most one grouped
  `#[cfg(feature = "server")] use super::server::*;` for its bodies.
- Each `mod.rs` re-exports the **generated server-fn types** (the PascalCase
  structs the `#[server]` macro emits — `CreateAudience`, `ListTags`,
  `BackupWarningVisible`, `SubscribeTo`, …), not merely the snake_case fns +
  DTOs. The registrar (`server/tests`) resolves
  `register_explicit::<web::<vertical>:: <Type>>()` by real path, so an omitted
  type re-export breaks the registrar test binary's compile (the #426 syn-gate,
  which matches by leaf name, would **not** catch it). Registrar paths
  (`web::audiences::CreateAudience`, `web::subscriptions::SubscribeTo`,
  `web::tags::ListTags`, `web::backup::*`) still resolve.

### AC3 — pure/host surfaces untouched in behavior and still host-tested

- The pure `render()` twins (`avatar`/`icon`/`taglist`/`topbar`),
  `avatar_parts`, and the `Icons` re-export stay ungated and host-compilable;
  `render/mod.rs` still calls them, unchanged apart from import/doc-link paths.
- `feed_discovery::{surface_label, rsd_href}` + their five tests stay
  host-tested (in `feed_discovery/labels.rs`). The intra-doc link in
  `render/mod.rs:214` (`web::feed_discovery::surface_label`) is repointed to
  `web::feed_discovery::labels::surface_label` so the rustdoc-link lint stays
  green.
- `forms::Field<T>` + `forms::field_error` + their `Owner`-scoped host tests
  stay ungated (in `forms/field.rs`); `ValidatedInput<T>` is the only gated
  `forms` item.
- `tags`' host tests (`tag_label_validation_agrees_client_and_server`,
  `tag_summary_preserves_casing_with_canonical_slug`) still run on host.
- The `ui`-leaf parity/unit tests stay host-side and pass — they assert on the
  pure twin, never instantiate the component.

### AC4 — `ui/` is gone; consumer paths updated

- `web/src/ui/` does not exist; `lib.rs` declares
  `pub mod avatar; pub mod icon; pub mod taglist; pub mod topbar;` (and no
  `pub mod ui;`).
- No `crate::ui::` / top-level `ui::` reference remains in `web/src`; every
  former consumer (`render/mod.rs:18`+doc-links, `audiences`, `backup`,
  `pages/ui.rs:31`+doc-links) resolves to the new top-level path.
- `crate::pages::ui` and `crate::backup::ui`→`backup::component` handled per D6;
  `crate::pages::ui` is otherwise untouched except its import paths.

### AC5 — layout details

- `backup/ui.rs` is renamed to `backup/component.rs`; `backup/mod.rs`'s stale
  "ungated for coverage" comment is corrected.
- `feed_discovery/` and `forms/` follow D4/D5; each leaf directory follows D2.
- audiences' keyed-store types (`AudienceListData`, `AudienceList`) move into
  `audiences/component.rs`; the wire DTOs stay in `api.rs` (with
  `AudienceSummary` keeping its `Store`/`Patch` derives there per D6's dual-role
  note).

### AC6 — docs reflect reality (this issue's slice only)

- ADR-0070 **§5** carries the D2 `ui/`-dissolution addendum (Status stays
  accepted).
- `docs/web-style-guide.md` **§6** ("Shared components") no longer says leaf
  primitives live in `web/src/ui/`; it names them as top-level modules.
- (The §8 / ADR Decision-1 api.rs edits are #530's, not asserted here.)

### AC7 — gates green

- Host build (`cargo check -p web --all-features --all-targets`) compiles.
- Host `web` tests pass (twins, parity, `Field<T>`, `field_error`, `tags` host
  tests, server-gated vertical tests).
- `wasm-clippy` (`-p web -p client`) is clean.
- `cargo xtask validate` (static + coverage + e2e all four backend×browser
  combos) is green; e2e behavior for the touched surfaces is unchanged.

## Coordination note

Both #527 and #530 amend `docs/adr/0070-…md` and `docs/web-style-guide.md`
(different sections: #530 owns Decision-1/§8; #527 owns §5/§6). #530's amendment
(`bcf095cc`) is **not yet on `main`**. Land order: #530 first, then rebase this
branch onto it so the §5/§6 edits apply atop the amended text (avoids a
same-file merge conflict).

## Verification notes

- Host coverage aggregate shifts (component lines leave the host denominator);
  re-scoping per ADR-0070, not regression. Pure twins / `Field` / `tags` tests /
  server fns keep their host coverage.
- `--all-features --all-targets` on host is the load-bearing local check for the
  server-gated web code (default check skips `#[cfg(feature="server")]` bodies).
