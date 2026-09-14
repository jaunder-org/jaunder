# Fix Demo Post Image Links Implementation Outline

> Execute with `jaunder-iterate`, using `jaunder-dispatch` for a bounded task
> when useful. This outline exists because the approved spec changes a versioned
> cross-process seed contract and storage-backed Media publication behavior.

## Scope

In:

- Version 2 of the sandbox seed manifest with a complete Media collection.
- Production-path placement of the four existing SVG fixtures before curated
  Post creation.
- Dual-backend profile proof, exact public Media-route proof, and staged failure
  cleanup proof.

Out:

- Renderer, sanitizer, Media identity/layout, public route, production
  bootstrap, and browser-test changes.
- Removal or redesign of existing demo content.

## Task outline

- [x] Task 1: Advance the generic seed manifest wire shape.
  - Contract: `SandboxSeedManifest` exposes `media: Vec<SandboxMedia>` and JSON
    schema version 2 serializes that collection; Media-free profiles emit `[]`.
  - Verification: focused `test-support` serialization tests prove version 2,
    ordered Media metadata, and empty-array behavior using supplied manifest
    values without assuming any content-addressed URL.

- [x] Task 2: Publish every demo Media object and materialize dependent content.
  - Contract: the demo seed places the baseline text fixture and each SVG
    through the production Media placement abstraction before Post creation,
    creates each Media Record for its declared owner, and constructs both the
    complete manifest and curated Post source from returned canonical
    identities.
  - Contract: no fixture independently maintains or rederives a
    content-addressed URL; newly placed files are cleaned up when the seed write
    fails without erasing the unexpected failure.
  - Verification: `#[apply(backends)]` profile tests prove four Users, 73 Posts,
    five owner-correct Media Records, exact stored bytes and metadata, complete
    manifest ordering, native Markdown/Org source, rendered HTML, and stored
    references.

- [ ] Task 3: Preserve named sandbox replacement on seed failure.
  - Contract: ADR-0180's staged replacement remains the publication boundary;
    failed reset preparation never replaces the current named workspace.
  - Verification: an `xtask` sandbox lifecycle test injects deterministic seed
    failure, proves the existing workspace remains intact, and proves the
    unpublished `.reset-new` workspace is removed.

- [ ] Task 4: Prove the reported image links resolve at the serving boundary.
  - Contract: all four distinct curated SVG URLs are requested through the
    existing public Media route; no serving or rendering policy changes.
  - Verification: focused HTTP integration coverage proves status success,
    `image/svg+xml`, and byte-for-byte fixture bodies, followed by
    `devtool run -- cargo xtask check` for the integrated branch.

## Risk checks

- Preserve ADR-0180's SQLite-only host sandbox and staged replacement lifecycle;
  backend parity applies to the test-support seed contract.
- Preserve ADR-0084's canonical byte-identical Media name/path/URL spelling by
  consuming the production placement result rather than deriving layout.
- Preserve ADR-0090's render-derived, owner-correct Post-to-Media references.
- Keep the manifest producer and every Rust/TypeScript consumer on schema
  version 2 in the same change.
- Keep the standard and empty profile content unchanged apart from serializing
  the empty Media collection.
- Do not add lint or coverage suppressions without explicit approval.
