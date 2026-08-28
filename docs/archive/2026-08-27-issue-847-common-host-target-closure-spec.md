# Issue #847 — common/host target closure

## Outcome

Issue #847 closes the workspace target boundary: `common` contains only shared,
dual-target domain and rendering concepts; `host` receives each current `common`
item with no CSR or other dual-target consumer. Issue #847 subsumes #855; this
is one architectural cutover, not two compatible migrations.

The CSR dependency closure contains exactly the CSR target's dependencies.
Current `common` host-only modules, protocol serializers, rendering operations,
password-storage concerns, and dependencies are unreachable from the CSR build.
The optional `common/sqlx` manifest and feature bridge is excluded from that
exact closure.

AtomPub and the public Syndication Feed remain distinct protocol surfaces.
AtomPub serves authenticated, editor-facing Collections and native source for
lossless round-trip; a Syndication Feed is public, unauthenticated, and
serialized as rendered HTML for feed readers. This cutover changes neither
surface's bytes, audience, nor behavior.

## Load-bearing decisions

- `common` is dual-target, not a general-purpose shared bucket. A type, module,
  operation, feature, or dependency belongs there only with a CSR or other
  dual-target consumer.
- `host` is strictly host-focused: it receives each current `common` item with
  no such consumer, including its supporting machinery. Semantic classification
  remains review-owned; the graph gate does not replace it.
- AtomPub moves wholesale from `common` to `host`: Collection-facing types,
  serializers, parsing, and protocol machinery. AtomPub routing remains in
  `server`, and no AtomPub compatibility surface remains in `common`.
- Host-only outbound Syndication Feed types, settings, path parsing, events,
  windows, representation models, and Atom/RSS/JSON rendering move to `host`.
  `FeedFormat`, `FeedSurface`, and `canonicalize` remain in `common` because CSR
  reaches that grammar; the rendered-HTML contract remains unchanged.
- `RenderOutput` and the sanitizer-gated module and machinery move to `host`.
  Qualified `host` rendering and sanitization free functions establish
  `RenderedHtml` from scrubbed output.
- `RenderedHtml` remains in `common` for CSR and wire use. Its gate-policed
  trusted reconstruction remains the cross-crate minting seam.
- Password, password-hashing errors, and `StoredPasswordHash` move to `host`.
  `ProfferedPassword` and its validation remain in `common` for dual-target
  supplied-password consumers.
- `RenderedHtml`, `PostFormat`, ETag, Org normalization, `croner`,
  `BackupSchedule`, `FeedFormat`, `FeedSurface`, and `canonicalize` remain in
  `common` for CSR or other dual-target consumers. Both closed configuration
  registries, `SiteConfigKey` and `UserConfigKey`, move to `host`.
- ETag construction, rendering, and password hash and verify operations become
  qualified `host` module free functions, not inherent `Password` methods.
  Shared values acquire no host behavior through `common` methods or traits.
- `common/sqlx` remains the sole ownership-forced optional manifest and feature
  bridge in `common`. It is excluded from the CSR closure and is no precedent
  for new host dependencies in `common`.
- Features, dependency declarations, and module surfaces cut over cleanly: no
  compatibility aliases, reexports, deprecated paths, or duplicate ownership
  shims remain.
- The cargo-metadata gate asserts that `host` has no runtime workspace
  dependency other than `common`, with the existing build-time `macros`
  exception, and that forbidden workspace crates are absent from the exact,
  target- and feature-resolved CSR closure. It reports violating paths and has
  no external-package allowlist; the existing wasm build proves compilation.
- No bundle-size benefit is claimed. This is enforceable target ownership and
  dependency boundaries, not an unmeasured output-size promise.
- The ADR draft at `docs/adr/0159-common-host-target-closure.md` records the
  decision; the architecture view projects current state, while dated notes
  truthfully annotate prior ADR history.

## Acceptance

- `common` contains no AtomPub Collection types, serializers, parsing, or
  protocol machinery. Callers use the `host` ownership surface directly, while
  AtomPub routing remains in `server`.
- `common` contains no host-only outbound Syndication Feed types, settings,
  parsing, events, windows, representation models, or rendering machinery.
  `FeedFormat`, `FeedSurface`, and `canonicalize` remain as its CSR-reached
  grammar; Feed producers and callers use the host-owned surface directly.
- `RenderOutput`, the sanitizer-gated module and machinery, Password,
  password-hashing errors, and `StoredPasswordHash` are absent from `common`'s
  public and internal module surface, and every caller is cut over.
- The `common/sanitize` feature and its now-unused dependencies are removed.
- `ProfferedPassword`, supplied-password validation, `RenderedHtml`,
  `PostFormat`, ETag, Org normalization, `croner`, `BackupSchedule`,
  `FeedFormat`, `FeedSurface`, and `canonicalize` remain available to existing
  CSR or dual-target consumers. Both `SiteConfigKey` and `UserConfigKey` are
  host-owned. Client and server share the one `BackupSchedule::FromStr`; trusted
  `RenderedHtml` reconstruction is the gate-policed cross-crate minting seam.
- ETag construction, rendering, and password hash and verify operations use
  qualified `host` module free functions, not inherent `Password` methods. No
  moved operation, including protocol rendering, remains a `common` method,
  trait, alias, or reexport.
- Cargo metadata proves the host-floor subset invariant, forbidden workspace
  crate absence from the exact CSR wasm closure, and `common/sqlx` exclusion. A
  focused check rejects a non-`common` host runtime workspace dependency outside
  the `macros` exception, or a forbidden CSR feature/dependency path, and
  reports that path.
- The existing wasm build compiles the post-cutover CSR graph; together with the
  metadata gate it distinguishes closure conformance from target compilation.
- No compatibility alias, reexport, deprecated import path, or duplicate
  definition preserves a moved item's former `common` location.
- AtomPub Collection native-source round-trip behavior and protocol bytes remain
  unchanged for the same inputs. Outbound Syndication Feed rendered-HTML
  serialization, representation selection, and protocol bytes also remain
  unchanged.
- Password validation, hashing, stored-hash handling, errors, ETag, Org
  normalization, and rendering/sanitization behavior—including scrubbed
  `RenderedHtml` output—remain unchanged for the same inputs.
- The new draft and `docs/ARCHITECTURE.md` agree on the target boundary,
  excluded `common/sqlx` bridge, wholesale AtomPub move, host-only Syndication
  Feed move, CSR-reached feed grammar, both host-owned configuration registries,
  and #847's subsumption of #855. Dated ownership/call-shape notes truthfully
  annotate ADR-0018, ADR-0023, ADR-0058, ADR-0063, ADR-0065, ADR-0072, ADR-0073,
  ADR-0079, ADR-0089, ADR-0090, ADR-0095, ADR-0102, and ADR-0112.
- `cargo xtask validate --no-e2e` passes with the metadata gate, coverage,
  doctest, and wasm compilation proofs enabled. PR CI owns the unchanged
  `{sqlite,postgres} × {chromium,firefox}` e2e matrix.

## Boundaries

- This issue does not change the public behavior, authentication, audience,
  serialization, or wire format of an AtomPub Collection or Syndication Feed.
- This issue does not merge AtomPub Collections with Syndication Feeds, share
  their serializers, or call either surface merely a "feed."
- This issue does not add a second optional manifest or feature bridge to
  `common`, make `common/sqlx` CSR-reachable, or freeze external dependencies.
- This issue does not claim a CSR bundle-size reduction, alter unrelated domain
  behavior, or use graph automation to make semantic ownership decisions.
