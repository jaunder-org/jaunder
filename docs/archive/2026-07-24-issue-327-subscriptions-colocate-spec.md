# Spec — #327 web(subscriptions): co-locate the SubscribeButton UI

**Issue:** [#327](https://github.com/jaunder-org/jaunder/issues/327) —
web(subscriptions): converge the subscriptions vertical onto the co-located
Leptos layout. **Design floor:** ADR-0070 (four-file host/wasm split; supersedes
ADR-0056), re-scoped by #526 / #530.

## Context — current state

The subscriptions vertical's **server surface is already converged**: `mod.rs`
(wiring only), `api.rs` (the three `#[server]` fns `subscribe_to` /
`unsubscribe_from` / `is_subscribed_to` + their `Username` wire arg),
`server.rs` (host-only `resolve_author` plus its tests). There is **no
`pages/subscriptions.rs`** — the acceptance floor is already met for the server
surface.

The one piece of subscriptions **UI** — the `SubscribeButton` `#[component]`
(the follow/unfollow control on a user's timeline/profile page) — lives in
`web/src/posts/component.rs` (`fn SubscribeButton`, ~70 lines, private), a
leftover tenant from the posts convergence (#323). It has **zero posts-domain
coupling**: it takes a `Username`, calls
`crate::subscriptions::{is_subscribed_to, SubscribeTo, UnsubscribeFrom}`, reads
the shared session (`crate::auth::use_session`), and renders two `ActionForm`s.
It is used in exactly one place — the `UserTimeline` page header
(`posts/component.rs:1482`). The three subscription fns are imported into
`posts/component.rs` **solely** for this component.

This cycle gives the subscriptions vertical its own UI home so the vertical
boundary is honest: the _subscribe control_ is subscriptions' UI, not posts'.

## Scope

**Extract `SubscribeButton` from `posts/component.rs` into a new wasm-only
`subscriptions/component.rs`.** A pure relocation — no behavior change.

- **`subscriptions/component.rs`** (new): the `SubscribeButton` fn moved
  verbatim, made `pub`, with its subscription-fn imports rewritten
  `crate::subscriptions::{…}` → `super::{…}`. Keeps `crate::auth::use_session`,
  `common::username::Username`, leptos. Declared
  `#[cfg(target_arch = "wasm32")] mod component;` in `mod.rs`. Zero
  `#[cfg(...)]` inside; zero `cov:ignore`.
- **`subscriptions/mod.rs`**: add the gated `mod component;` +
  `#[cfg(target_arch = "wasm32")] pub use component::SubscribeButton;`. The
  vertical becomes the full four-file layout (`mod` / `api` / `server` /
  `component`).
- **`posts/component.rs`**: delete the `SubscribeButton` fn; replace the
  `use crate::subscriptions::{is_subscribed_to, SubscribeTo, UnsubscribeFrom};`
  import (used only by that fn) with
  `use crate::subscriptions::SubscribeButton;`; the `UserTimeline` call site
  renders `crate::subscriptions::SubscribeButton` unchanged otherwise.

No new tests: `SubscribeButton` is already exercised **end-to-end** by
`end2end/tests/visibility.spec.ts` (its `subscribeTo`/`unsubscribeFrom` helpers
click the "Subscribe"/"Unsubscribe" button on the author's profile and assert a
Subscribers-audience post surfaces/hides). The subscription endpoints +
visibility effect are covered dual-backend by
`server/tests/web/web_subscriptions.rs` and `web_posts.rs`.

## Acceptance criteria

- **AC1** `web/src/subscriptions/component.rs` exists, wasm-only, defining
  `pub fn SubscribeButton(username: Username)`; it contains no `#[cfg(...)]`
  line and no `cov:ignore` marker.
- **AC2** `subscriptions/mod.rs` is still wiring-only and now declares
  `#[cfg(target_arch = "wasm32")] mod component;` and re-exports
  `SubscribeButton`; the vertical is `mod`/`api`/`server`/`component`. The only
  `target_arch` gates in `web/src/subscriptions` are the `mod component;`
  declaration and its `pub use` re-export.
- **AC3** `web/src/posts/component.rs` no longer defines `SubscribeButton` and
  no longer imports `is_subscribed_to`/`SubscribeTo`/`UnsubscribeFrom` (it
  imports `crate::subscriptions::SubscribeButton` instead); the `UserTimeline`
  header still renders `<SubscribeButton username=… />`.
  `rg 'fn SubscribeButton' web/src/posts` yields nothing.
- **AC4** No fake host stub (ADR-0055); no new coverage exemption —
  `SubscribeButton` stays wasm-only (coverage-exempt wholesale), as it already
  was in `posts`.
- **AC5** Behavior unchanged: `cargo xtask validate` green including the e2e
  matrix; `visibility.spec.ts` (which clicks the extracted button) passes,
  proving the subscribe → content-surfaces → unsubscribe → content-hidden loop
  still works.
- **AC6** The stale reference in `web/src/posts/mod.rs:64-66` — which lists
  `SubscribeButton` among posts' "private helpers … stay unexported" — is
  scrubbed (drop `SubscribeButton` from that list), so posts no longer documents
  a component it no longer owns. (The behavioral mention in `posts/render.rs` —
  "the anonymous `SubscribeButton` renders nothing" — stays: it is still true
  and describes projector behavior, not ownership.)

## Out of scope

- Any change to the subscription server fns, storage, or the visibility/feed
  model.
- Restyling or restructuring the button UI (verbatim move only).
- The other frontier verticals (#325 sessions, #328 tags) and the #330 → #520
  chain.

## Decisions / ADRs

No new ADR. This is a locality move within the established ADR-0070 structure
(giving a server-only vertical its `component.rs` for the UI it owns).

## Verification

- Host gate: `devtool run -- cargo xtask check` while iterating.
- Wasm clippy on `subscriptions/component.rs` before committing (via
  `cargo xtask check`'s `wasm-clippy` step).
- Full local gate: `cargo xtask validate` (incl. e2e) — `visibility.spec.ts` is
  the behavioral proof.
