# Spec — issue #935: collapse the sqlx-bridge trait-bound comment

## Problem

The rationale for generic-backend trait bounds — "`X` binds as itself via the
sqlx bridge (#438), which delegates to `String`; these bounds make that bridge
available on the generic backend" — repeats at ~18 sites across `storage/src/`,
near-verbatim. ADR-0071 (`docs/adr/0071-sqlx-string-newtype-bridge.md`) already
records both halves of that rationale: the bridge delegates to the inner
`String` (one impl, both backends), and the generic `…Store<DB>` impls restate
their types as bounds.

## Decision (from interview)

ADR-0071 is the canonical home. Every site becomes a one-liner citing ADR-0071
(and #438 where already present) plus only its local specifics — which types
bind at that site, and any genuinely local nuance (e.g. `Option<&PostTitle>`
covering the nullable `title` bind, the `&'q str` bound for `#[text_enum]`
tokens). No new comment site; no ADR amendment.

## Scope

The repeated bridge-bound comments in `storage/src/`: `posts.rs` (~8),
`media.rs` (2), and one each in `users.rs`, `sessions.rs`, `invites.rs`,
`email.rs`, `feed_cache.rs`, `feed_events.rs`, `audiences.rs`, `password.rs`,
`site_config.rs`; plus the two-site verbatim "Not residue: the ADR-0071 bridge
_delegates_ to `i64`…" duplicate (`posts.rs`/`users.rs`) — keep one site's
fuller wording only if it carries a nuance the ADR lacks, otherwise both become
the same one-liner.

A comment that already carries a site-local nuance keeps that nuance; only the
shared restatement collapses.

## Non-goals

- No code changes (comment-only diff; the gate stays green).
- No ADR-0071 edit.
- No touching gate-marker comments or their ADR-0094 adjacency.

## Deliverable

One commit, `docs(storage): cite ADR-0071 for bridge bounds (#935)`, full
`cargo xtask check` green (pre-commit).
