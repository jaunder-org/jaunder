# ADR-DRAFT: Temporary Atom namespace-aware fork

- Status: proposed
- Date: 2026-09-01
- Issue: [#813](https://github.com/jaunder-org/jaunder/issues/813)

## Context

[ADR-0089](../0089-upstream-atom-document-io.md) delegated every Atom document
to upstream `atom_syndication` and retired ADR-0043's quick-xml advisory bridge.
That delegation exposed an upstream model defect: an element-scoped namespace
declaration on an extension is parsed as an ordinary local-name attribute and is
then serialized without its `xmlns:` declaration, leaving the element prefix
unbound (#813). Hoisting Jaunder's own markers is not a general repair: it loses
scope, cannot represent conflicting prefix bindings, and does not cover Feed or
Source extensions.

Atom's expanded-name identity is a namespace URI plus a local name; a source
prefix is only preferred-prefix serialization metadata. The current model
instead loses namespace-qualified attributes and cannot faithfully retain
ordered duplicate extension children, default namespaces, or nested namespace
rebinding across Entry, Feed, and Source.

This is not a return to ADR-0043's superseded two-crate quick-xml advisory
bridge. It is a new, temporary, atom-only decision that qualifies
[ADR-0089](../0089-upstream-atom-document-io.md): upstream continues to own Atom
document serialization, but Jaunder needs an upstream-compatible model that can
represent the XML it delegates to it.

## Decision

Fork `rust-syndication/atom` as `jaunder-org/atom` and implement the complete,
breaking namespace-aware `atom_syndication` 0.13.0 model there. The model MUST:

- use namespace URI plus local name as the semantic identity of elements and
  attributes, treating source prefixes solely as preferred-prefix serialization
  metadata;
- preserve namespace-qualified attributes and ordered mixed content, including
  duplicate child elements, with default namespaces and nested namespace
  rebinding in Entry, Feed, and Source; and
- reject XML with an unbound prefix rather than emitting or accepting malformed
  namespace state.

The user reviewed and accepted `921118c311d2117956d86e25052918e7c549ef00`,
closed fork PR #1, and will open the upstream pull request from that branch.
Jaunder pins that exact commit directly through Cargo `[patch.crates-io]`;
neither Cargo nor the flake may follow a branch, tag, or fork pull-request
merge. The fork is a `flake = false` input, and an atom-only crane
`overrideVendorGitCheckout` vendors that same pinned source so Nix builds remain
hermetic. The existing `jaunder-org` cargo-deny source allowance remains shared
with the live `lettre` fork and is neither removed nor duplicated. This pin
machinery is atom-only; no RSS dependency, lettre revision, or direct
`quick-xml` dependency changes.

Upstream submission, landing, and release remain user-owned and outside this
cycle. Jaunder exits the temporary fork only when upstream releases the same
complete namespace-aware public API and behavior: Entry, Feed, and Source
parity; URI-plus-local-name semantic equality with source prefixes as
serialization metadata; namespace-qualified attributes; ordered mixed content
and duplicates; nested and default namespace handling; and unbound-prefix
rejection. At that point remove the atom-only Cargo patch, flake input, and
crane vendoring machinery together, resolve to the released crate, and archive
`jaunder-org/atom`; retain the unrelated lettre source policy.

## Consequences

Jaunder keeps the upstream Atom serialization boundary from ADR-0089 while
carrying one deliberately pinned fork until upstream releases that complete
public API and behavior. This proposed, registry-only qualification of accepted
ADR-0089 does not rewrite it. Atom XML with namespace-sensitive extensions
round-trips as namespace-correct XML instead of silently changing declarations
into ordinary attributes. The 0.13.0 breaking model may require upstream
consumers to migrate to expanded names and explicit namespace-aware extension
structure. `CONTEXT.md` remains unchanged because no domain vocabulary changes.

The fork creates a temporary maintenance and reproducibility obligation: the
exact revision, flake lock data, Cargo resolution, and crane vendor source must
move as one reviewed unit. RSS remains a registry dependency, `quick-xml`
continues to serve Jaunder's non-Atom Service Document, RSD, and shared XML
helpers, and lettre retains its independently governed revision; none changes
under this decision. The release exit removes only the atom-specific apparatus,
not lettre's independently governed pin.
