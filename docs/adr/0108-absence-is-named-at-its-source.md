# ADR-0108: Absence is named where it can occur

- Status: accepted
- Date: 2026-08-10
- Issue: [#343](https://github.com/jaunder-org/jaunder/issues/343)

## Context

`sqlx::Error::RowNotFound` is what `fetch_one` returns when a query matched
nothing. A blanket `impl From<sqlx::Error> for InternalError` maps it — and
every other driver error — to kind `Storage`, class `Bug`, which per ADR-0011 is
ERROR-level and pages. Because it is a `From` impl it fires on a bare `?`, so a
storage helper returning `sqlx::Result` can wake an operator for a row that is
simply not there.

#343 proposed reclassifying `RowNotFound` to a quieter class. That was rejected
on two grounds. It is a **wire** change, not merely an observability one:
`project` turns kind `NotFound` into `WebError::NotFound { message }`, so a
masked 500 would silently become a 404 whose body is the conversion's public
message — and a `From` impl has no resource name to put there. And it would buy
quiet for a couple of sites by silencing the `INSERT … RETURNING` sites, where a
missing row genuinely means the database did something impossible.

An audit found the exposure was small and specific: `fetch_optional` already
outnumbered `fetch_one` in `storage`, so "absence is an `Option`" was the house
style, and exactly **two** sites could produce a `RowNotFound` in practice.

## Decision

**Where a row can genuinely be absent, name it. Everywhere else, leave the code
alone.**

1. **`MissingRow { what }`** is a standalone error naming an absent required
   row, with `From<MissingRow> for InternalError` → `InternalError::server`
   (kind `Internal`, class `Bug`). It **still pages** — a required row being
   absent is a real invariant violation. What changes is legibility: the
   operator is told _which_ row, instead of `"storage operation failed"` with
   `"no rows returned"` buried in the source chain.

2. **`RequireRow`** is the one-line partner:
   `…fetch_optional(pool).await? .require_row("the seeded 'local' channel row")?`.
   The driver error takes the path it always took; only absence is new.

3. **`fetch_one` remains correct, and is not discouraged, where the row is
   structurally guaranteed** — a bare aggregate (`SELECT COUNT(*)`,
   `SELECT COALESCE(SUM(x), 0)` with no `GROUP BY`), `SELECT EXISTS(…)`, or
   `INSERT … RETURNING` with no `ON CONFLICT`. Those always yield exactly one
   row, so `RowNotFound` is impossible. `fetch_one` states that intent, and if
   the impossible ever happened it returns an `Err` rather than panicking.

4. **The blanket `From<sqlx::Error> for InternalError` stays.** A `RowNotFound`
   arriving there means a call site used `fetch_one` on a row that can be absent
   — a caller defect, to be fixed at the call site.

**Whether a row can be absent is a per-query judgement, and no lint can make
it.** A mechanical ban on `fetch_one` was built and then removed: it cannot read
SQL, so it flagged the structurally-guaranteed sites too, forcing ~17 correct
calls into `fetch_optional` plus `unreachable!` or `.expect()` — longer, and
with a panic path where there had been a graceful error. The enforcement made
the code worse than the problem it policed.

## Consequences

- An operator paged by a missing required row now learns which row. The page
  still fires.
- The two sites that could actually produce a `RowNotFound` are closed:
  `subscribe` structurally (one atomic `ON CONFLICT … DO UPDATE … RETURNING`
  instead of an insert plus a racing second `SELECT`), and `local_channel_id` by
  naming its seed.
- `set_post_tags`' tag-id read-back also names its absence. Its insert is
  `ON CONFLICT DO NOTHING` / `INSERT OR IGNORE`, so the following `SELECT` may
  be reading a pre-existing row; it is unreachable only because nothing deletes
  a tag today — a fact about the data, not the statement (#883). A named error
  rather than an `unreachable!`, so that if tag deletion ever lands it reports
  which row went missing instead of panicking a request handler.
- **Not enforced.** A future `fetch_one` on a maybe-absent row will not be
  caught mechanically. That is accepted: the judgement is per-query, and the
  attempt to automate it cost more than it saved.
- **Rules out** re-classifying `RowNotFound` to a quieter class, and banning
  `fetch_one`.
- Supersedes #343's own first acceptance criterion, which asked that a benign
  `RowNotFound` stop logging at ERROR. Absence still pages; it is now legible.
  The issue body records the substitution.
