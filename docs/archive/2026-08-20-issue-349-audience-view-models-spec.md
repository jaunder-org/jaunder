# #349 - extract audience view-model assembly

Issue: [#349](https://github.com/jaunder-org/jaunder/issues/349). Milestone:
Code quality ratchet.

## Summary

`web/src/audiences/api.rs` currently owns two jobs inside the `#[server]`
boundary:

- converting `storage::AudienceRecord` into the wire/UI `Summary`; and
- resolving each subscriber row into the presentation label for
  `SubscriberSummary`. This is a relational projection: local-channel
  subscriptions display the joined local username when that user row exists, and
  all other rows fall back to the raw `subscriber_ref`.

That keeps effectful view-model assembly in the thin web layer and leaves the
label-resolution behavior testable only through Leptos server-function context.
Jaunder will move the DTOs and the assembly code into a dedicated
`web::audiences::model` leaf and make the web server functions plain
call-throughs.

## Decision

Add `web/src/audiences/model.rs` for named-audience presentation models and
assembly:

- `Summary` remains the id/name audience row shown in the audiences screen and
  post-editor picker.
- `SubscriberSummary` remains the assignment-checklist row containing
  `subscription_id` and the resolved display `label`.
- `list_audiences(author_user_id, &dyn AudienceStorage)` returns `Vec<Summary>`
  by mapping `AudienceRecord` rows.
- `SubscriptionStorage` exposes a SQL-backed subscriber-label projection that
  returns active subscribers in subscription order with the label already
  resolved by the database.
- `list_subscribers(author_user_id, &dyn SubscriptionStorage)` returns
  `Vec<SubscriberSummary>` by mapping that storage projection to the web DTO.

`web::audiences::model` is presentation/application assembly local to the
audiences vertical, not shared domain vocabulary. This keeps the seam in the
existing `web` crate until more than one vertical needs a cross-crate
presentation-model home. The DTOs live there because the code that produces them
lives there; they do not move to `common`. The label projection itself belongs
in SQL because it is a join over persisted subscription, channel, status, and
user rows; Rust should not reimplement that relational work by hand.

`web/src/audiences/api.rs` should import those DTOs from `model`, re-export them
through `web::audiences` as today, and delegate `list_mine` /
`list_my_subscribers` to the extracted functions after retrieving authentication
and per-trait Leptos contexts. The other audience mutation server functions stay
unchanged except for imports forced by the DTO move.

Extend `storage::SubscriptionStorage` with a read projection for the audience
subscriber checklist, e.g.
`list_subscriber_summaries(author_user_id) -> sqlx::Result<Vec<SubscriberSummaryRecord>>`.
The concrete implementation should issue one SQL query that filters active
subscriptions, resolves the seeded `local` channel inline, `LEFT JOIN`s `users`
only for local-channel rows, and falls back to `subscriber_ref` when no local
user row matches. Query failure fails the server function; there is no
degraded-success swallowed-error path for this projection.

## Label-resolution contract

The storage projection resolves labels in SQL:

1. Return only active subscriptions for the authenticated author.
2. Preserve the existing subscription ordering (`subscription_id` order).
3. If a row's `channel_id` is the seeded `local` channel and `subscriber_ref`
   matches an existing local `users.user_id`, the label is that user's username.
4. If a row's `channel_id` is not the seeded `local` channel, the label is the
   original `subscriber_ref`. Remote refs are opaque even when they look
   numeric.
5. If a row is on the local channel but no matching local user row exists, the
   label is the original `subscriber_ref`.
6. A database/query error fails the projection and therefore the server
   function. Do not catch that error to return partial/raw labels.

## Acceptance criteria

AC1. `web/src/audiences/api.rs` no longer builds `Summary` or
`SubscriberSummary` values inline in `list_mine` / `list_my_subscribers`; those
server functions delegate to the extracted presentation assembly after auth and
context lookup.

AC2. `Summary` and `SubscriberSummary` live in `web::audiences::model` with
their producing functions, and `web::audiences` continues to expose the same DTO
names to existing UI and server-function callers.

AC3. Dual-backend storage tests cover the SQL label projection: local subscriber
resolves to username, non-local channel with numeric-looking ref remains raw,
missing local user remains raw, inactive subscribers are excluded, and ordering
matches `subscription_id` order.

AC4. `web::audiences::model` has direct tests proving `list_mine` maps
`AudienceRecord` to `Summary` and `list_my_subscribers` maps the storage
projection to `SubscriberSummary` without changing field names or order.

AC5. Existing audience server-function behavior remains unchanged for both
storage backends: `devtool run -- cargo xtask e2e-local audiences.spec.ts`
passes and `devtool run -- cargo xtask check` passes.

## Out of scope

- Changing audience authorization or author scoping.
- Changing audience membership storage semantics.
- Changing endpoint paths, server-function request shapes, JSON field names, or
  UI behavior.
- Moving audience presentation DTOs to `common` or to a new crate before a
  second vertical needs a cross-crate presentation-model seam.
- Adding `UserStorage` batch lookup, Rust-side deduplication, caching,
  cross-request memoization, or a degraded-success swallowed-error path for
  subscriber label projection.
