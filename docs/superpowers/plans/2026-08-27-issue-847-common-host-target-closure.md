# Issue #847 common/host target closure implementation outline

> Execute with `jaunder-iterate`, delegating bounded tasks through
> `jaunder-dispatch`. This outline exists because the approved spec changes a
> durable crate boundary, protocol implementation ownership, and cross-task
> interfaces.

## Scope

In:

- Move every approved host-only type and operation from `common` to the existing
  `host` crate, then migrate all callers without compatibility paths.
- Add the Cargo graph invariant and preserve the existing wasm compilation
  proof.
- Preserve protocol bytes, password behavior, rendering/sanitization, ETag
  behavior, Org normalization, and shared client/server schedule validation.
- Keep the proposed ADR, architecture projection, and historical annotations
  aligned with the delivered code.

Out:

- User-visible behavior, protocol, authentication, storage schema, or backend
  changes.
- Bundle-size claims, external-package allowlists, and automatic semantic
  classification of host-only source.
- Any optional `common` host bridge beyond the retained `common/sqlx` bridge.

## Task outline

- [x] Task 1: Gate the target-resolved workspace graph.
  - Contract: an xtask repository-shape step evaluates Cargo metadata and
    rejects a runtime workspace dependency from `host` other than `common` (with
    build-time `macros` retained), or a CSR wasm closure containing `host`,
    `storage`, `server`, or the `common/sqlx` feature. Failures report the
    violating dependency path; external packages are not allowlisted.
  - Verification: focused xtask tests cover each forbidden edge/feature path and
    an allowed external dependency, using the xtask workspace test lane.

- [x] Task 2: Move the password domain and storage surface to `host`.
  - Contract: `ProfferedPassword` and shared shape validation stay in `common`;
    `Password`, password hashing/verification errors, and `StoredPasswordHash`
    move to `host`. Hash and verify are module-qualified `host::password` free
    functions preserving current results and typed error sources.
  - Verification: focused common/host/storage tests prove shared inbound
    validation, hash/verify success and failures, dummy-hash parity, persisted
    hash handling, and caller migration.

- [x] Task 3: Move rendering, sanitization, and ETag construction to `host`.
  - Contract: `RenderedHtml`, `PostFormat`, ETag, and Org normalization stay in
    `common`. `host::render` owns `sanitize(&str) -> RenderedHtml`,
    `render(&PostBody, &PostFormat) -> RenderedHtml`,
    `render_with_media(&PostBody, &PostFormat) -> RenderOutput`, and
    `extract_media_refs(&str) -> Vec<MediaReference>`. `host::etag` owns
    `sha256_of`, `from_sha256`, `from_content_hash`, `post_content_etag`, and
    `feed_etag` with their current parameters and ETag results. Host
    sanitization uses the gate-policed trusted reconstruction door;
    `common/sanitize` is removed.
  - Verification: focused tests preserve rendered and scrubbed HTML, media
    references, ETag values, Org normalization, SQLx decode behavior, and both
    RenderedHtml construction gates.

- [x] Task 4: Move AtomPub implementation ownership wholesale to `host`.
  - Contract: AtomPub models, parsing, extensions, Service Document/RSD/XML
    machinery, and serializers move as one deep host module; Axum routing
    remains in `server`. Server adapters import the host surface directly.
  - Verification: moved unit tests and focused AtomPub integration tests prove
    byte-identical documents, native-source round-trip, extension semantics,
    service discovery, and unchanged error behavior.

- [x] Task 5: Move the host-only outbound Syndication Feed surface to `host`.
  - Contract: this task follows Task 3 and consumes its `host::render` and
    `host::etag` interfaces. `FeedFormat`, `FeedSurface`, and `canonicalize`
    remain in `common` because CSR consumes that grammar. FeedPath parsing, both
    closed configuration registries, settings/events/windows, representation
    models, and Atom/RSS/JSON rendering move together. Server and storage
    callers use host-owned types directly. AtomPub Collection serializers remain
    separate.
  - Verification: moved unit tests and focused feed/storage/server/web tests
    prove typed CSR discovery, byte-identical representations, rendered-HTML
    items, ETag/cache/event behavior, and unchanged representation selection on
    both storage backends where the existing contract is backend-parametric.

- [x] Task 6: Reconcile the target boundary and run the branch gate.
  - Contract: remove obsolete common features/dependencies/modules and every old
    import; retain only `common/sqlx` as the ownership-forced optional host
    bridge. Keep `croner`, `BackupSchedule`, and CSR-reached `FeedFormat`,
    `FeedSurface`, and `canonicalize` in `common`, with client and server
    sharing `BackupSchedule::FromStr`; move both closed configuration registries
    to `host`. Reconcile the ADR draft, architecture view, and dated
    ownership/call-shape annotations in ADR-0018, ADR-0023, ADR-0058, ADR-0063,
    ADR-0065, ADR-0072, ADR-0073, ADR-0079, ADR-0089, ADR-0090, ADR-0095,
    ADR-0102, and ADR-0112 against delivered code; leave `CONTEXT.md` and
    generated `docs/README.md` unchanged.
  - Verification: focused shared-parser tests prove the client/server schedule
    contract; source/reference checks find no legacy ownership path; a final
    document review checks every named projection and annotation against the
    delivered ownership and call shapes; the focused proofs above remain green;
    `cargo xtask validate` passes.

## Risk checks

- `common` retains every CSR-reachable validation/value path: ProfferedPassword,
  RenderedHtml, PostFormat, ETag, Org normalization, croner, BackupSchedule,
  FeedFormat, FeedSurface, and canonicalize; host owns both closed configuration
  registries.
- The SQLx orphan-rule bridge remains excluded from the exact CSR closure and
  does not broaden into a second host feature.
- Host-floor dependency direction remains acyclic; `host` does not learn
  storage, server, or web abstractions.
- AtomPub Collections and Syndication Feeds retain separate serializers,
  audiences, and native-source versus rendered-HTML contracts.
- No moved symbol survives through an alias, reexport, deprecated path, or
  duplicate definition.
- Every exported-symbol move includes reference migration before its old owner
  is removed.
