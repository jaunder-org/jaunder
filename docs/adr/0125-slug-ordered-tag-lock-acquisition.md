# ADR-0125: Slug-ordered tag lock acquisition

- Status: accepted
- Date: 2026-08-11

## Context

`set_post_tags` reconciles a post's tags inside a transaction. The tag upsert
holds a Postgres row lock on each `tags` row until commit. Two concurrent
reconciles adding overlapping tags in caller-supplied order can acquire those
locks in opposite orders and deadlock (#876). SQLite is unaffected —
`BEGIN IMMEDIATE` locks database-wide — but a shared rule keeps the backends
identical.

## Decision

Every transaction sorts the tags it will touch by slug before acquiring locks,
so all transactions take `tags` row locks in the same global order. The sort is
`sort_by_key` (stable), not `sort_unstable_by_key`: `desired` may carry two
labels sharing a slug, and the FIRST occurrence's casing must win
(`set_post_tags_is_idempotent_and_absorbs_duplicate_slugs`).

## Consequences

- Deadlock-free concurrent tag reconciles on Postgres, at the cost of a sort
  that is free at these sizes.
- Any new code path that locks multiple `tags` rows in one transaction must
  follow the same slug order.
