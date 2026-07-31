# ADR-0089: Delegate AtomPub Atom document I/O to `atom_syndication`; retire the fork bridge

- Status: accepted
- Date: 2026-07-31
- Issue: [#737](https://github.com/jaunder-org/jaunder/issues/737),
  [#199](https://github.com/jaunder-org/jaunder/issues/199)

## Context

Two temporary arrangements, created for unrelated reasons, expired within weeks
of each other. Both expired because upstream `rust-syndication` shipped.

**The bespoke Atom I/O.** `common/src/atompub/entry.rs` hand-rolled a
`quick-xml` SAX reader and four writers for Atom documents. The reason was
recorded in its own module doc: `atom_syndication` could only read and write
whole `<feed>` documents, because its entry-level `FromXml`/`ToXml` traits were
crate-private — while AtomPub (RFC 5023) exchanges _standalone_ `<entry>`
documents on POST, PUT, and member GET. So we populated and read the canonical
`atom_syndication::Entry` model but supplied our own XML layer: a
`Parser`/`Acc`/`Field` state machine, xhtml re-serialization, and entity
resolution — roughly 450 lines with its own bug surface.

`atom_syndication` **0.12.10** (2026-07-30) makes bare-entry I/O public:
`Entry::read_from`, `impl FromStr for Entry`, `Entry::write_to`, and
`Entry::write_with_config`. The stated reason for the bespoke layer no longer
holds.

**The fork bridge.** ADR-0043 cleared RUSTSEC-2026-0194/0195 by forking
`atom_syndication` and `rss` onto `quick-xml 0.41` and wiring the forks in
through `[patch.crates-io]` plus hermetic crane vendoring, rather than ignoring
the advisories. It named its own exit condition: upstream releases depending on
quick-xml ≥ 0.41. That condition is now met on both crates — `atom_syndication`
0.12.9+ and `rss` 2.1.0.

The two are coupled: `[patch.crates-io]` overrides the registry entirely, so
while the patch stands, `atom_syndication = "0.12"` resolves to a fork that
predates 0.12.10 and the new API is unreachable. The bridge must go before the
delegation can happen.

## Decision

**Retire the fork bridge, and delegate every Atom document to
`atom_syndication`.**

1. **Drop ADR-0043's apparatus in full** — both `[patch.crates-io]` entries,
   both flake inputs, the crane `overrideVendorGitCheckout`, and `jaunder-org`
   in `deny.toml`'s `[sources.allow-org]` — moving to registry
   `atom_syndication` 0.12.10 and `rss` 2.1. ADR-0043 becomes `superseded`. No
   `[advisories].ignore` is introduced: the advisories stay cleared by the same
   mechanism as before, a single quick-xml ≥ 0.41.

2. **Every Atom document is serialized by upstream.** Not only the standalone
   entry: `render_feed` builds an `atom_syndication::Feed`, and the RFC 5023
   §9.6 media-link entry is an `Entry` whose `Content` carries `src` and no
   value. The reason to go all the way is that upstream's _embedded_ entry
   serializer — the no-`xmlns` form used inside a `<feed>` — remains
   `pub(crate)`. Replacing only the standalone pair would force `render_feed` to
   keep a hand-rolled entry writer purely for embedding, leaving two entry
   serializers in one module that must be kept in agreement. Going through
   `Feed` is what actually deletes the bespoke layer.

3. **Upstream's output is accepted verbatim.** No post-processing restores
   today's bytes. This changes the XML declaration (no `encoding="utf-8"`),
   element order, and leaves a typeless `<content>` as `None` rather than
   defaulting it to `"text"`. Every consumer was checked first: the e2e specs
   assert substrings, the elisp client parses with `libxml-parse-xml-region`,
   and `wire_to_format` already treats `None` and `Some("text")` identically.
   Re-deriving today's byte layout would mean reintroducing the bespoke writer
   this decision exists to delete.

4. **Jaunder's foreign markup stays ours.** `app:control/app:draft` (RFC 5023
   §B) is not modeled first-class by `atom_syndication`, and the `j:` namespace
   (ADR-0023) is jaunder's own; both live in the entry's extension map behind
   the `is_draft`/`set_draft`/`j_slug`/`set_j_slug` helpers. Because upstream
   emits `xmlns:*` from `Entry::namespaces`, **each helper owns its prefix in
   that map** — inserting on set, removing on clear — which is what preserves
   the property that an entry declares `xmlns:app` or `xmlns:j` only when it
   actually carries the corresponding marker.

5. **Timestamps cross this boundary as domain values.** `FeedMeta` and
   `MediaLinkEntry` carried RFC-3339 `String`s that upstream would have to
   reparse. They become `UtcInstant` (ADR-0072/0063) — the type both producers
   already hold — which deletes a stringify/reparse round trip and leaves the
   serializers with no fallible step of their own. The residual, unreachable
   write error gets an `AtomPubError` variant distinct from `Malformed`, because
   `Malformed` maps to a client 400 and a serialization failure is ours, not the
   client's.

6. **What stays hand-rolled.** The AtomPub service document, the categories
   document, and RSD are not Atom, are not modeled by `atom_syndication`, and
   keep their `quick-xml` writers. `common` therefore keeps its direct
   `quick-xml` dependency.

## Consequences

- **Positive.** ~450 lines of bespoke XML parsing and serialization are deleted
  along with their bug surface, and Atom conformance becomes upstream's problem.
  The dependency graph loses two forks, two flake inputs, a crane vendor
  override, and a loosened `deny.toml` sources policy — the first git `[patch]`
  in this repo is gone, so it no longer sets a pattern for the next one.
- **Ingest narrows.** Upstream rejects three inputs the bespoke reader
  tolerated: an unparseable `<updated>`/`<published>`, a `<title>`/`<summary>`
  whose `type` is outside `text|html|xhtml`, and — the consequential one —
  namespace-prefixed atom elements. Our reader matched by _local_ name, so
  `<atom:entry>`/`<atom:title>` worked; upstream matches the qualified name, so
  a prefixed root is a 400 and a prefixed child is routed into the extension map
  with its field silently lost. The first two turn silently-wrong data into an
  honest error. The third is a real interop narrowing, accepted because
  preserving it would require keeping the bespoke reader this decision exists to
  delete, and because jaunder's own client emits the default-namespace form. If
  a real client is found to use the prefixed form, the answer is a normalizing
  pre-pass, not a return to hand-rolled parsing.
- **We are now bound to upstream's wire layout.** A future `atom_syndication`
  release that reorders elements or changes the declaration changes our
  responses. This is acceptable precisely because no consumer depends on layout,
  and it is the ordinary cost of delegating a format to its library.
- **Follow-up outside this branch.** The `jaunder-org/atom` and
  `jaunder-org/rss` forks should be archived on GitHub now that nothing
  references them.
- **Follow-up.** Migrating the storage records' own timestamps to `UtcInstant` —
  the last non-newtype fields on `PostRecord` — is tracked as
  [#748](https://github.com/jaunder-org/jaunder/issues/748).
