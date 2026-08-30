# ADR-0123: RenderedHtml decodes via a plain sqlx bridge, without sanitizing

- Status: accepted
- Date: 2026-08-11

## Context

`RenderedHtml` is the trusted-HTML newtype. On the write side it is a
first-class TEXT bind parameter via the shared `#[derive(SqlxBridge)]` codegen
(#502, #746), delegating to the inner `String` (`Type::compatible` delegates
too, so an equally-valid `VARCHAR` column is accepted). The question is the read
side: how may a `rendered_html` column decode into the trusted type?

An earlier stance was "deliberately NO `Decode`", on the argument that a decode
can bless ANY text column decoded into it — e.g. a raw, un-rendered `body`.

## Decision

The `rendered_html` column decodes straight into `RenderedHtml` (#445), like
every other domain column, through a common-private bridge that constructs the
crate-private field directly. This is not new outside data, so it does not call
`sanitize`; reading must preserve persisted rendered bytes without a second
parse, allocation, or failure path.

**The blessing risk is real and is accepted**: decoding some other column into
this type would still bless it. Typing a column as `RenderedHtml` is therefore a
deliberate, security-relevant review responsibility. The retired spelling gate
and its allowance markers cannot prove SQLx column correspondence; reviewers
must ensure every decode targets the rendered-HTML column.

## Rejected alternative

A _sanitizing_ decode would remove the wrong-column risk outright and heal any
pre-#445 row on read. Rejected: no deployed instance holds data, and a parse on
every post read would silently rewrite a representation that storage must return
verbatim. Revisit only if an instance ever accumulates rows written by a
pre-#445 build.

## Consequences

- `build_post_record` needs no raw-string reconstruction.
- Any new direct `RenderedHtml` decode or field remains a security-relevant
  review point: verify the SQL projection supplies rendered HTML, not a raw body
  or another text column.
