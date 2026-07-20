# Spec — #91: strongly-typed `#[server]` boundary (typed UTC timestamps)

- Issue: [#91](https://github.com/jaunder-org/jaunder/issues/91)
- Milestone: #13 Domain-value type safety (newtypes)
- Date: 2026-07-19
- Status: awaiting approval

## Problem

The web `#[server]` boundary carries timestamps as bare RFC3339 `String` /
`Option<String>` at **~27 sites** (8 params, 19 return-DTO fields). Parsing and
validation live at the edges (`web/src/posts/mod.rs::parse_publish_at`),
compile-time guarantees are lost, and "valid string, wrong meaning" bugs are
invited. #70 followed this `String` convention for `publish_at` deliberately, on
the belief that `chrono` is unavailable in the wasm client.

**That belief is obsolete.** `chrono` is already compiled into the CSR/wasm
bundle today via the unconditional `web → common → chrono` chain
(`common/Cargo.toml` has `chrono.workspace = true`, ungated, and uses
`DateTime<Utc>` in public `common::feed` API), with chrono 0.4's default
`wasmbind` feature making it wasm-clean. The `web`-level
`chrono = { optional, server-only }` gate only keeps `web`'s _own_ `use chrono`
sites out of the CSR feature set; it does not keep the crate out of the bundle.
So a `DateTime`-backed timestamp newtype defined in `common` compiles in the CSR
build exactly as the existing `common::feed` chrono API already does. (The
client is CSR / `mount_to_body`, not hydrate — ADR-0040 — so the non-server
build is `csr`.)

## Decision

Introduce **`common::time::UtcInstant`**, a newtype wrapping
`chrono::DateTime<Utc>`, and thread it through every timestamp site on the
`#[server]` boundary in place of the RFC3339 `String`. Additionally adopt
**one** already-existing domain type at a boundary site that regressed to a
primitive (no new type needed). Record the timestamp-newtype convention as a new
ADR extending ADR-0063.

### The `UtcInstant` newtype (`common/src/time.rs`)

```rust
pub struct UtcInstant(chrono::DateTime<chrono::Utc>);
```

- **Wire form: RFC3339 string, via chrono's own serde.** `common` enables
  chrono's `serde` feature
  (`chrono = { workspace = true, features = ["serde"] }` — a one-line addition;
  chrono is already a `common` dependency), so
  `DateTime<Utc>: Serialize / Deserialize` is available in **every** build, CSR
  included. `UtcInstant` therefore `#[derive(Serialize, Deserialize)]`: as a
  serde-transparent newtype it (de)serializes exactly as chrono's
  `DateTime<Utc>` — an RFC3339 string, wire-compatible with the current String
  fields, the datetime control's output, and cursor round-trips. chrono's
  `Deserialize` parses RFC3339 into `DateTime<Utc>` (normalizing any offset to
  UTC and rejecting malformed input), so decode-time wire validation is
  preserved with **no hand-written bridge** — reusing the already-available impl
  instead of duplicating it. The reliance on the feature is not a silent-failure
  risk: if it were ever disabled the CSR/wasm target would fail to compile,
  which the pre-push e2e build catches loudly (criterion 4).
- **A newtype, not a raw `DateTime<Utc>`**, because a timestamp is a domain
  value (the milestone's thesis) and the newtype is the single home for its
  `FromStr` validation, `Display`/formatting, and the ADR-0065 client `Field`
  hook.
- **`FromStr`** (hand-written — the one chokepoint we still author, required for
  the client `Field<UtcInstant>` path, where the datetime control yields a
  string) parses RFC3339 and canonicalizes to UTC (`.with_timezone(&Utc)`).
  `Err = InvalidInstant` implements `Display` (via `thiserror::Error`) so the
  type works directly with `Field<UtcInstant>` / `ValidatedInput<UtcInstant>`
  (ADR-0065).
- **Accessors:** `Display` (user-facing rendering), and
  `as_datetime() -> DateTime<Utc>` (for server-side construction from storage
  records and client-side chrono formatting). A `now()` constructor if a call
  site needs it.
- **Not a secret**, so — per the ADR-0065 findings — it needs **no**
  `Proffered*` twin and **no** new xtask gate; it is usable directly as both
  `#[server]` arg and return, like `Slug` / `Username`.

### Adoption — the timestamp sites

**Params (8) → `UtcInstant` / `Option<UtcInstant>`:**

- `create_post.publish_at`, `update_post.publish_at`: `Option<String>` →
  `Option<UtcInstant>` (`web/src/posts/mod.rs`).
- `cursor_created_at` on the six paginated listers (`list_drafts`,
  `list_user_posts`, `list_local_timeline`, `list_home_feed`,
  `list_posts_by_tag`, `list_user_posts_by_tag`): `Option<String>` →
  `Option<UtcInstant>`.

**Return-DTO fields (19) → `UtcInstant` / `Option<UtcInstant>`:** the
`created_at` / `updated_at` / `scheduled_at` / `published_at` / `expires_at` /
`used_at` / `last_used_at` / `next_cursor_created_at` fields on `DraftSummary`,
`CreatePostResult`, `UpdatePostResult`, `PublishPostResult`, `PostResponse`,
`TimelinePostSummary`, `TimelinePage`, `MediaItem`, `InviteInfo`, and
**`SessionInfo`** (`created_at`, `last_used_at` — returned by `list_sessions`,
`web/src/sessions/mod.rs:14-42`).

**Server-side production** currently `.to_rfc3339()`s a `DateTime<Utc>` from
storage into each String field — the **authoritative production surface is the
19 `.to_rfc3339()` call sites** across
`web/src/{posts/mod.rs,posts/server.rs, posts/listing.rs,invites/mod.rs,media/mod.rs,sessions/mod.rs}`
(e.g. `listing.rs:79,224` cursor production, `sessions/mod.rs:37-38`). Each
becomes `UtcInstant(dt)` — the ad-hoc `parse_publish_at`
(`web/src/posts/mod.rs:221`) and every per-field `.to_rfc3339()` are deleted;
arg-decode validates `publish_at`.

**Client-side consumption:**

- `crate::render::format_post_time` (`web/src/pages/ui.rs:147`) and any other
  string-formatting of these fields switch to taking `&UtcInstant` /
  `UtcInstant` and format via `as_datetime()` + chrono (legitimately client-side
  now), replacing string munging. Direct-render sites
  (`web/src/pages/invites.rs:86-88` `expires_at`/`used_at`,
  `web/src/pages/media.rs:166` `created_at`, the `scheduled_badge` at
  `web/src/pages/posts.rs:1018`, and the sessions list rendering `created_at`/
  `last_used_at`) format via the newtype.
- **Cursor round-trip:** the client signals holding `next_cursor_created_at`
  (`web/src/pages/posts.rs`, `web/src/pages/timeline.rs`) become
  `Option<UtcInstant>`; the value received from a page is passed straight back
  as the next `cursor_created_at` — opaque, never user-entered, no validation
  UI.
- **`publish_at` input:** the existing `js_sys::Date` datetime control
  (`web/src/pages/ui.rs::local_datetime_to_utc_rfc3339`) is **kept** — the
  browser's local→UTC wall-clock conversion is a genuine browser concern,
  orthogonal to chrono (migrating it to chrono is ADR-0056's separate scope).
  Its RFC3339 output is parsed into `UtcInstant`; the submit path constructs
  `UtcInstant::from_str(..)`. An empty datetime field yields `None` (the
  control's helper returns `None` on empty input), which on **create** publishes
  immediately. Wire mechanics (unchanged by the typing): an empty wire string is
  rejected; an omitted value decodes to `None` (serde_qs skips `None`). Note:
  there is **no** "unschedule an already-scheduled post by clearing the field"
  flow in the current UI (the edit page hides the control once
  `published_at.is_some()`) — that is a pre-existing product gap, unrelated to
  #91's typing, filed as #549.

### Adoption — the free existing-type win (no new type)

- `create_invite.recipient_email`: `String` → `Email`
  (`web/src/invites/mod.rs:37`; a plain `#[server]` param; type exists in
  `common::email`).

**Deliberately NOT converted — `AudienceSummary.audience_id`/`name`.** These
look like regressions to raw `i64`/`String`, but they are a **sanctioned
ADR-0063 carve-out** (#475): `AudienceSummary` derives
`reactive_stores::Store, Patch` (`web/src/audiences/api.rs:35`), and `Patch`
requires every field to be `PatchField` (a foreign trait with primitive-only
impls and no blanket impl) — so a newtype field breaks the `Patch` derive / hits
the orphan rule. The type stays `i64`/`String` and converts to
`AudienceId`/`AudienceName` at the edges. Converting them here would undo a
documented exception; excluded on purpose.

### ADR

Record as a **new ADR extending ADR-0063** (authored as a numberless draft via
the `jaunder-adr` draft-out-of-git flow —
`docs/adr/0072-timestamps-cross-boundary-as-utcinstant.md`; numbered at ship
by `cargo xtask adr promote`): the `DateTime`-backed timestamp-newtype variant
(hand-written RFC3339 serde bridge over chrono **core**, distinct from the
`StrNewtype`/`IdNewtype` derives because neither fits a `DateTime` backing), and
the corrected architectural fact that chrono is already in the wasm bundle
(retiring #70's `String` rationale). A new ADR rather than an in-place edit of
the still-`proposed` ADR-0063: it introduces a third backing flavor and records
a cross-cutting finding that deserves top-level visibility.

## Out of scope (each has a home)

Other weak boundary primitives are **not** in #91 — they need their own new
types and are tracked separately: post `format` (#498), password-reset /
email-verification `token`s and session-token returns (#500), media
`content_type` (#495), composed URLs / permalinks (#448), and post
`summary`/excerpt and profile `bio` (need new `PostSummary` / `Bio` types —
sibling milestone-13 issues). Migrating the client datetime helper from
`js_sys::Date` to chrono is ADR-0056's scope.

## Acceptance criteria (observable)

1. **`UtcInstant` exists** in `common::time`, wrapping `DateTime<Utc>`, deriving
   `Serialize`/`Deserialize` (via chrono's `serde` feature, enabled in
   `common`), with a hand-written `FromStr` (RFC3339 → UTC-canonicalized,
   `Err: Display`), `Display`, and `as_datetime()`. Unit tests assert: serde
   round-trip preserves the instant; an offset-bearing input decodes to the
   equivalent UTC instant; `FromStr` rejects malformed input; serde
   `deserialize` of an invalid string fails (chrono rejects it).
2. **No timestamp crosses the `#[server]` boundary as `String`.** A
   **universal** grep over _all_ `#[server]` fns and their DTOs in `web/src/**`
   (not just the enumerated subset) finds zero timestamp fields/params typed
   `String`/`Option<String>`, and zero `.to_rfc3339()` marshalling calls remain
   in `web/src`. `parse_publish_at` is gone; validation lives in `UtcInstant`,
   not at the edge.
3. **The free adoption** lands: `create_invite.recipient_email` is `Email`.
   `AudienceSummary` is deliberately left `i64`/`String` (the #475 `Patch`
   carve-out).
4. **Both target builds compile:** the `server` build and the CSR/wasm build
   (`cargo check -p csr` / the leptos CSR build, which the pre-push e2e gate
   runs). `UtcInstant` appears in `#[server]` signatures without breaking the
   wasm client. This is also the guard that a `chrono/serde` feature regression
   cannot ship silently — the CSR build fails loudly if the feature is ever
   disabled.
5. **Behavior preserved end-to-end:** scheduled publishing still works — an e2e
   drives the datetime control, schedules a post (`publish_at` set), and
   verifies the scheduled badge on `/drafts`. The
   create-with-omitted-`publish_at` → publish-now path is covered by the
   `create_post_publish_without_publish_at_is_live_now` integration test (there
   is no unschedule-via-clear flow in the product — see the Decision note; a
   separate product gap, #549). Cursor pagination still pages correctly (the
   typed cursor round-trips — 3 existing e2e "Load more" tests). Timestamp
   display in drafts/timeline/invites/media is unchanged for the user.
6. **wasm-bundle impact noted:** `cargo xtask audit-wasm` delta recorded in the
   PR; expected ≈ 0 (chrono is already in the bundle).
7. **A new ADR extending ADR-0063** (draft-out-of-git, numbered at ship) records
   the instant-backed newtype variant and the chrono-in-wasm finding.
8. The full local gate (`cargo xtask validate`) passes, including coverage and
   the `server_fn_registrar_check` gate for any touched `#[server]` fns.

## Risks / notes

- `UtcInstant`'s serde deliberately relies on chrono's `serde` feature (enabled
  in `common`). The CSR/wasm compile in the e2e gate is the guard that the
  feature stays enabled — a regression is a build failure, not a silent one
  (criterion 4).
- Display formatting moves from ad-hoc string handling to chrono; verify no
  locale/format regression in `format_post_time` output (criterion 5).
- The datetime control's RFC3339 output must parse under `UtcInstant::from_str`;
  a round-trip test through the control's helper covers this.
