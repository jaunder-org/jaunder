# Gate RenderedHtml-typed fields

- Issue: [#701](https://github.com/jaunder-org/jaunder/issues/701)
- Milestone: Domain-value type safety (newtypes)

## Problem

`RenderedHtml` carries the invariant that its contents are safe to emit
unescaped. `RenderedHtml::sanitize` establishes that invariant, and
`RenderedHtml::from_trusted` only inherits it for values Jaunder already
sanitized and round-tripped through its own storage or wire. The
`rendered-html-from-trusted` gate currently protects that second door by
requiring an in-source marker on every non-test `from_trusted` mention.

That does not cover the storage decode side. `RenderedHtml` has a plain sqlx
decode bridge: if a `FromRow` field typed `RenderedHtml` were accidentally
pointed at a raw `body` column, the row would mint trusted HTML without naming
either door. ADR-0079 and ADR-0123 deliberately accepted that residual risk only
as a review point, and #701 exists to make it mechanical.

The issue title says "row fields", but the same source-level risk is not limited
to `#[derive(sqlx::FromRow)]`: a production struct field typed `RenderedHtml` is
a deliberate trust-carrying surface. Some are legitimate storage rows, domain
records, Syndication Feed items, or seed/wire DTOs. Those sites should stay, but
each should carry a local reason that explains why the field may hold trusted
rendered HTML.

## Decision

Extend the existing `rendered-html-from-trusted` step rather than adding a
separate xtask step. It guards the same invariant and should continue to produce
one gate result with one recovery path.

Add a second population to that step: non-test Rust struct fields whose type is
`RenderedHtml`, including path-qualified and owner-aliased spellings the scanner
can resolve. The population is deliberately **direct field types only** for this
issue: `field: RenderedHtml`, `field: common::render::RenderedHtml`, and aliases
that resolve to that type. Borrowed and container forms such as `&RenderedHtml`,
`Option<RenderedHtml>`, `Vec<RenderedHtml>`, and `Box<RenderedHtml>` are out of
scope because they are not live production field shapes today and would require
a recursive type walker with its own policy. The gate documentation must state
that boundary as an unreadable/out-of-scope class rather than silently implying
coverage.

Every production field in that direct-type population must carry a marker on the
line immediately above the field:

```rust
// rendered-html-from-trusted:allow <reason>
pub rendered_html: RenderedHtml,
```

The marker token intentionally reuses the existing step name. This keeps the
operator model simple: the gate's marker means "this site is a reviewed
`RenderedHtml` trust surface", whether the site is an inheriting door call or a
field that can carry trusted HTML.

The scanner must parse Rust with `syn`, fail loudly on parse errors, and reuse
the existing source-scan roots and test-code exemption semantics as much as
possible. It must not inspect SQL text or infer safety from a field name. A
field is in scope because its written type is `RenderedHtml`, not because it is
named `rendered_html`.

Type resolution must fail closed where it affects the trust boundary. A field
whose direct type resolves to some other local or imported type is out of the
population. A field whose direct type is a single unresolved identifier remains
in the population, because the gate cannot prove it is not an alias of
`RenderedHtml`. As with the existing `from_trusted` gate, the module docs must
state the limits of syntactic resolution: multi-hop aliases, renaming re-exports
outside the policed roots, and generic or associated types are not fully
type-resolved by `syn`.

The legitimate in-tree production fields should be marked in source with short
reasons. At minimum this covers the current storage/domain/wire/feed surfaces:

- `storage/src/helpers.rs` `PostRow.rendered_html`, the storage row decode
  surface whose SQL projection must remain reviewable;
- `storage/src/posts.rs` `PostRecord.rendered_html` and
  `PostRevisionRecord.rendered_html`, post read models whose values come from
  render/sanitize before storage;
- `common/src/seed.rs` `RenderedPost.rendered_html`, the server-to-client seed
  and wire DTO surface that deserializes through the reviewed trusted-rebuild
  helper;
- `common/src/feed/metadata.rs` `FeedItem.content_html`, the Syndication Feed
  item carrying already-rendered post content;
- any other non-test, non-generated `RenderedHtml` field currently under the
  policed roots after the structural scanner is implemented.

Update ADR-0079 and ADR-0123 so they no longer describe #701 as an unclosed
mechanical gap. They should instead state that direct `RenderedHtml`-typed
fields are now part of the gate population and require local markers, while
preserving the reason that decode itself is not sanitizing. Project the same
current truth into `docs/ARCHITECTURE.md`; it must not continue to say row
fields are only a review point "until #701 widens the gate."

No public `RenderedHtml` constructor, serde surface, sqlx bridge behavior, SQL
text, storage behavior, rendering behavior, or wire shape changes.

## Boundaries

This is a static-gate hardening task. It should not change how rendered HTML is
created, sanitized, stored, decoded, serialized, deserialized, or displayed.

Do not replace `RenderedHtml` fields with `String` to satisfy the gate. The
right outcome is explicit reviewed trust surfaces, not weaker typing.

Do not add a new ADR. The work closes a gap already recorded in ADR-0079 and
ADR-0123; updating those accepted ADRs is enough.

Do not broaden this into general column-to-field SQL correspondence. Adjacent
same-typed column transposition is #751, not this issue.

## Acceptance Criteria

- `rendered-html-from-trusted` fails on a non-test struct field typed
  `RenderedHtml` with no marker.
- A marker on the line immediately above a `RenderedHtml` field exempts that
  field only when the marker has a non-empty reason and the field is the only
  marked site on that line.
- Stale/orphan field markers fail, just as stale `from_trusted` markers do.
- Test-only `RenderedHtml` fields remain exempt under the same test-code policy
  as the existing gate.
- The scanner catches at least these field-type spellings in unit tests:
  `RenderedHtml`, `common::render::RenderedHtml`, an in-file `use ... as ...`
  alias, and a type alias whose target is `RenderedHtml`.
- The scanner has unit tests showing the direct-type boundary: `&RenderedHtml`,
  `Option<RenderedHtml>`, and `Vec<RenderedHtml>` are either ignored with the
  out-of-scope documentation updated, or deliberately included by an implemented
  recursive walker. The implementation must choose one behavior explicitly.
- The scanner does not require a marker for unrelated fields whose type name
  merely contains the same substring or for fields typed as a different
  resolvable type.
- The scanner fails closed on an unresolved direct field type identifier by
  requiring a marker, and unit tests cover both the marked and unmarked cases.
- All existing production `RenderedHtml` fields under the policed roots carry
  reviewed `rendered-html-from-trusted:allow` markers.
- ADR-0079, ADR-0123, and `docs/ARCHITECTURE.md` are updated to say the #701
  residual is now mechanically guarded by the field population.
- Existing `from_trusted` call-site behavior and recovery wording continue to
  work; #701 must not weaken or remove that population.
- `cargo xtask check` passes.
