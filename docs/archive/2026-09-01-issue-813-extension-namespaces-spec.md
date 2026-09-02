# Namespace-safe Atom extensions

Issue: [#813](https://github.com/jaunder-org/jaunder/issues/813)

## Outcome

Atom extension markup round-trips as namespace-valid XML across standalone Entry
and Feed documents and embedded Sources. Jaunder temporarily pins
`jaunder-org/atom` commit `921118c311d2117956d86e25052918e7c549ef00` until an
upstream `atom_syndication` release contains the same namespace-aware model and
behavior.

When Jaunder parses and serializes the same Member Entry value, an arbitrary
extension may declare namespaces on its own element, including nested prefix
rebinding, without producing an unbound-prefix document or changing the
extension's namespace meaning.

## Load-bearing decisions

### Namespace model

- `atom_syndication` 0.13.0 replaces the lossy extension maps with one
  canonical, recursive namespace-aware representation shared by Entry, Feed, and
  Source.
- An element or attribute has semantic identity `(namespace URI, local name)`.
  An optional source prefix is preferred-prefix serialization metadata, not part
  of semantic equality.
- Extension content is one ordered sequence of text and child-element nodes.
  This preserves mixed content, document order, duplicates, and same-local-name
  children from different namespace URIs.
- Attributes with distinct expanded names remain distinct. XML namespace rules
  apply: a default namespace applies to unprefixed element names, never to
  unprefixed attribute names.
- Namespace declarations retain source scope. Nested declarations may rebind a
  prefix without changing the namespace meaning of ancestors or siblings.
- Serialization may deterministically choose a serialized XML name or relocate
  declarations when necessary. The reserialized tree must preserve expanded-name
  semantics; exact prefix spelling and declaration placement are not contracts.
- A prefixed element or attribute with no in-scope declaration is malformed and
  fails parsing. The library never preserves an unresolved prefix only to emit
  invalid XML later.
- Entry and Feed expose the same extension collection. Source gains that
  collection and preserves unknown extensions when embedded in either document.
- Semantic equality compares namespace URI, local name, text, attributes, and
  ordered content. Preferred-prefix serialization metadata and equivalent
  declaration placement do not make otherwise equivalent trees unequal.

### Public API and compatibility

- The clean namespace-aware extension model is the only authority used by the
  parser and writer. The fork does not retain the old local-name maps as a
  second mutable representation.
- `ExpandedName` has public `namespace_uri: Option<String>`,
  `local_name: String`, and `preferred_prefix: Option<String>` fields. Its
  `PartialEq` compares only namespace URI and local name.
- `ExtensionAttribute` has public `name: ExpandedName` and `value: String`
  fields.
- `ExtensionContent` is an ordered public enum with `Text(String)` and
  `Element(Extension)` variants.
- `Extension` has public `name: ExpandedName`,
  `attributes: Vec<ExtensionAttribute>`, and `content: Vec<ExtensionContent>`
  fields.
- Entry, Feed, and Source expose `extensions: Vec<Extension>`. The old
  `ExtensionMap`, local-name `attrs` and `children` maps, split `value`, and
  root-only `namespaces` maps are removed.
- The parser resolves declarations into expanded names and discards declaration
  placement as non-semantic. The writer synthesizes the nearest deterministic
  declarations required by expanded names and preferred-prefix serialization
  metadata.
- Derived builders expose the same canonical fields and enum values. With the
  serde feature, field and variant names follow this public representation and
  round-trip it; backward compatibility with the 0.12 serialized shape is not a
  contract.
- This is an intentional breaking public-model change and the fork identifies
  itself as `atom_syndication` 0.13.0. Jaunder migrates every affected caller.
- The crate's existing feature surfaces remain supported: default and
  no-default-feature builds, builders, serde round-trip, and semantic
  `PartialEq` behavior.
- Atom field parsing and serialization outside extension/name handling retain
  their existing behavior and ordering unless required for namespace validity.

### Temporary fork lifecycle

- The implementation lands in `jaunder-org/atom`. The user reviewed and accepted
  commit `921118c311d2117956d86e25052918e7c549ef00`, closed fork PR #1, and will
  open the upstream pull request from that branch. Jaunder pins that exact
  commit directly; no branch, tag, or fork pull-request merge floats underneath
  a checked build.
- Jaunder uses one `atom_syndication` `[patch.crates-io]` entry plus a matching
  `flake = false` input and atom-only crane `overrideVendorGitCheckout` path so
  Cargo, Nix builds, and cargo-deny inspect identical fork bytes.
- The existing `jaunder-org` source allowance remains shared with the live
  `lettre` fork. This change neither removes nor duplicates that policy.
- The temporary decision is recorded separately from superseded ADR-0043 and
  qualifies ADR-0089's registry-only statement. The architecture view describes
  the fork while it is active. `CONTEXT.md` remains unchanged because no domain
  vocabulary changes.
- Upstream submission, landing, and release remain user-owned and outside this
  cycle.
- Exit requires a released upstream `atom_syndication` version containing the
  namespace-aware API and behavior. The exit change raises the registry
  requirement, removes only the Atom patch/input/vendor override, regenerates
  Cargo and flake locks, and then archives the fork. RSS, direct `quick-xml`,
  and `lettre` source policy remain unchanged.

## Acceptance

### Fork behavior

- Standalone Entry and Feed parse/write/reparse tests preserve semantic
  extension trees with an element-scoped namespace declaration. Source parity is
  exercised through an Entry and a Feed containing `atom:source`; no new
  standalone Source document API is introduced.
- Entry, Feed, and embedded Source cover nested prefix rebinding where one
  source prefix denotes different URIs in different scopes; reparsed expanded
  names retain the correct URI and local name.
- Tests preserve a default-namespaced extension subtree while keeping an
  unprefixed attribute outside the default namespace.
- Tests preserve distinct attributes with different expanded names but equal
  local names, ordered duplicate children, and equal-local-name children from
  different namespaces.
- A mixed-content test preserves text before and after a child element as one
  ordered semantic sequence.
- Unbound element and attribute prefixes are rejected with the crate's malformed
  input error rather than producing a writable unresolved node.
- Writer output is namespace-valid and reparses to a semantically equal tree;
  tests do not require original prefix spelling or declaration placement.
- Builder and serde feature tests construct and round-trip the canonical model,
  and the crate's stable/beta/nightly/MSRV, all-target, default/no-default
  feature matrix remains green.

### Jaunder behavior and integration

- The host standalone-Entry seam parses the issue's element-scoped declaration,
  serializes that same in-memory Entry through `entry_to_xml`, and reparses a
  namespace-valid, semantically equal extension tree.
- The host Collection renderer serializes a supplied Entry containing nested
  prefix rebinding without changing either expanded element name.
- Jaunder does not persist arbitrary foreign extensions across Post mapping;
  create/update/readback is not an acceptance seam for this issue.
- Existing `app:draft` and `j:slug` marker behavior, including collision-safe
  source-prefix ownership, remains green after migration to expanded names. The
  public Syndication Feed renderer remains behaviorally unchanged.
- Cargo.lock resolves `atom_syndication` 0.13.0 from the exact fork revision.
  Hermetic static, test, deny, and package derivations consume that same source.
- The proposed temporary-fork ADR and `docs/ARCHITECTURE.md` truthfully describe
  the active pin and its release-triggered removal.

## Boundaries

- No byte-for-byte XML preservation, prefix-spelling guarantee, or declaration-
  placement guarantee.
- No changes to AtomPub authentication, Post mapping, publication lifecycle,
  native-source policy, Syndication Feed membership, or non-Atom XML writers.
- No RSS fork or RSS dependency change. Direct `quick-xml` ownership for the
  Service Document, RSD, and shared non-Atom helpers remains unchanged.
- No indefinite Jaunder ownership of `atom_syndication`; the fork is a bounded
  bridge with an explicit upstream-release exit.
- No upstream pull-request submission or upstream release work in this cycle.
