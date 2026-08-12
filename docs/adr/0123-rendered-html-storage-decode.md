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

The column decodes straight into `RenderedHtml` (#445), like every other domain
column, through a plain bridge that constructs the private field directly.
Neither door is involved: this is not new outside data (so not `sanitize`), and
routing it through `from_trusted` would put a gate-policed door on a path the
gate cannot inspect.

**The blessing risk is real and is accepted**: decoding some other column into
this type would still bless it. The decision rests on one argument only — typing
a column as `RenderedHtml` is a deliberate, reviewable act. Note what does _not_
back it: the `rendered-html-from-trusted` gate does **not** catch this — its
population is `from_trusted` on this type, and a `FromRow` field typed
`RenderedHtml` over the wrong column names no door at all. Widening the gate to
flag `RenderedHtml`-typed row fields would close the hole — filed as #701.

## Rejected alternative

A _sanitizing_ decode would remove the risk outright and heal any pre-#445 row
on read. Rejected: no deployed instance holds data, so it would guard only
against a write path that forgot to sanitize — which the gate already catches —
at the cost of an html5ever parse on every post read, forever. Revisit only if
an instance ever accumulates rows written by a pre-#445 build.

## Consequences

- `build_post_record` needs no `from_trusted` rebuild.
- Any new `RenderedHtml`-typed `FromRow` field is a security-relevant review
  point until #701 lands.
