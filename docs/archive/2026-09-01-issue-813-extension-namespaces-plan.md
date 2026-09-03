# Namespace-safe Atom extensions implementation outline

> Execute with `jaunder-iterate`, using `jaunder-dispatch` for isolated slices.
> This outline exists because issue #813 changes a public dependency API, spans
> two repositories, and adds a temporary hermetic git-source boundary.

## Scope

In:

- `jaunder-org/atom`: a complete `atom_syndication` 0.13.0 namespace-aware
  extension model, parser, writer, builders/serde/equality behavior, and tests,
  accepted by the user at commit `921118c311d2117956d86e25052918e7c549ef00`.
- Jaunder: an exact-revision Cargo/Nix/crane pin, full caller migration,
  host-level namespace regressions, proposed ADR, and architecture projection.
- The user closed fork PR #1 and will open the upstream pull request from the
  accepted branch; Jaunder pins the exact commit directly.

Out:

- submitting, landing, or releasing the upstream `rust-syndication/atom` pull
  request;
- persisting arbitrary Atom extensions in Posts;
- AtomPub, Syndication Feed, RSS, or non-Atom XML policy changes;
- compatibility with the 0.12 extension-map or serde representation.

## Task outline

- [x] Task 1: Ship the namespace-aware 0.13.0 model to `jaunder-org/atom`
  - Completed input: user-reviewed and accepted commit
    `921118c311d2117956d86e25052918e7c549ef00`; fork PR #1 is closed. The user
    will open the upstream pull request from that branch.
  - Contract: public
    `ExpandedName { namespace_uri, local_name, preferred_prefix }`,
    `ExtensionAttribute { name, value }`, ordered
    `ExtensionContent::{Text, Element}`, and recursive
    `Extension { name, attributes, content }`; Entry, Feed, and Source expose
    `Vec<Extension>`. `ExpandedName` equality ignores preferred-prefix
    serialization metadata; `ExtensionAttribute` equality uses expanded name and
    value; `Extension` equality compares name, attributes by
    expanded-name/value, and ordered content while inheriting prefix-insensitive
    expanded-name equality.
  - Parser contract: resolve expanded names with source scope, apply default
    namespaces to elements but not unprefixed attributes, preserve mixed
    content/order/duplicates, and reject unbound or duplicate expanded attribute
    names.
  - Writer contract: synthesize deterministic nearest-scope declarations,
    choosing serialized XML names when needed; output must reparse to a
    semantically equal tree.
  - Verification: fork unit/integration tests cover standalone Entry and Feed,
    Source embedded directly in an Entry and in an Entry within a Feed, nested
    rebinding, default namespaces, expanded-name attributes, ordered mixed
    content, duplicates, malformed prefixes, semantic equality across differing
    preferred-prefix metadata, builders, serde, and default/no-default features;
    at accepted commit `921118c311d2117956d86e25052918e7c549ef00`, the
    applicable stable/default/no-default/all-feature test, doctest,
    format/clippy, and rustdoc lanes passed locally; GitHub had no check runs on
    that final head.
  - Boundary: Jaunder's immutable input is the accepted exact branch commit, not
    a fork pull-request merge. Upstream submission, landing, and release are
    user-owned.

- [x] Task 2: Pin the fork and migrate Jaunder's Atom boundary
  - Depends on: accepted `jaunder-org/atom` commit
    `921118c311d2117956d86e25052918e7c549ef00`.
  - Contract: require `atom_syndication` 0.13.0; patch crates.io to the exact
    fork revision; add the matching `flake = false` input and atom-only crane
    `overrideVendorGitCheckout`; Cargo, flake, Nix package/check derivations,
    and cargo-deny must consume identical bytes.
  - Migration: replace every `ExtensionMap`/`attrs`/`children`/`namespaces`
    caller with the canonical model. Preserve `app:draft` and `j:slug` helper
    semantics and collision-safe preferred-prefix behavior.
  - Verification: host tests first reproduce the issue through direct Entry
    parse → `entry_to_xml` → reparse and through Collection rendering; both pass
    with nested rebinding. Existing marker, standalone Entry, Collection, and
    Syndication Feed tests remain green.

- [x] Task 3: Verify and document the temporary bridge as one Jaunder change
  - Depends on: Task 2's completed exact pin, Cargo lock, and flake lock state.
  - Contract: the proposed temporary-fork ADR, architecture projection, Cargo
    pin, and Nix vendor bridge state one matching revision and one complete
    upstream-release exit condition. `CONTEXT.md` remains unchanged because no
    domain term changes; `docs/README.md` remains promoter-owned.
  - Verification: focused host tests, `cargo xtask check`, and the commit gate
    cover Rust/wasm/static behavior; `cargo xtask validate --no-e2e` proves the
    hermetic Nix source, deny, package, doctest, and coverage surfaces before
    shipping if CI has not yet exercised the exact pin.

## Risk checks

- Namespace equality is semantic, but parser/writer state still must terminate
  deterministically under nested prefix conflicts and default rebinding.
- Mixed text/element content cannot be flattened into separate value/child
  fields or reordered by a map.
- Source parity is tested as `atom:source` embedded directly in an Entry and in
  an Entry within a Feed; no standalone Source document API is introduced.
- A Cargo git patch without the matching flake input/vendor override is
  incomplete and must not be committed or pushed.
- The atom-only exit must not remove RSS registry use, direct `quick-xml`, or
  the independently required lettre source allowance.
- No fork commit or Jaunder commit may add a lint suppression without explicit
  approval, and no commit carries a `Co-Authored-By` trailer.
