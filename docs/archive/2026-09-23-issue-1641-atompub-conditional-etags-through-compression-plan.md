# Issue #1641 — AtomPub validator delivery implementation outline

> Execute with `jaunder-iterate`; use `jaunder-dispatch` only for bounded,
> independent work. This outline exists because the fix crosses a public HTTP
> validator contract, Emacs Protocol Client transport, and a separately owned
> production Caddy configuration. The approved
> [spec](2026-09-23-issue-1641-atompub-conditional-etags-through-compression-spec.md)
> is authoritative.

## Scope

In: Jaunder's AtomPub `no-transform` response contract, Emacs identity request,
independent proxy-path proofs, Jaunder documentation, and an
uncommitted/unpushed/undeployed change to
`/home/mdorman/src/tendentious-nix/services/caddy/default.nix`.

Out: accepting suffixed `If-Match`, changing conditional-write semantics,
migrating saved Emacs markers, altering the legacy Caddy site, or
committing/pushing/deploying the operator checkout.

## Task outline

- [x] **Capture the original failure:** Drive an AtomPub create/GET/conditional
      PUT through a real Caddy fixture with unconditional `encode zstd gzip` and
      no `no-transform` response header.
  - Verification: with `Accept-Encoding: zstd`, assert `Content-Encoding: zstd`,
    a suffixed/changed strong ETag, replay of that wire ETag yielding a false
    `412`, and unchanged Post state. Keep this red-capable fixture for the green
    proof, without treating the red state as a shippable commit.
- [x] **Protect the entire AtomPub response surface:** Add server-owned
      `Cache-Control: no-transform` to every `/atompub/*` response without
      dropping any existing cache directives; leave `/~{username}/rsd.xml`
      outside that path contract.
  - Verification: direct route-census assertions for the Service Document, Posts
    Collection GET/POST, Member GET/PUT/DELETE, Media Collection POST and Member
    GET/DELETE, including bodyless `204` and representative error responses;
    prove pre-existing directives survive. Canonical content ETag and exact
    `If-Match` remain unchanged.
- [x] **Prove the server defense independently:** Rerun the original
      unconditional-`encode` Caddy fixture with a Zstandard-advertising client
      and no operator path exclusion.
  - Verification: identity Post bytes, no `Content-Encoding`, unsuffixed
    canonical ETag and a successful conditional mutation despite that Caddy
    encoder. This fixture must fail again if only the server header is removed.
- [x] **Protect the Emacs transport:** Send `Accept-Encoding: identity` through
      the central authenticated AtomPub `plz`/curl request path, without
      changing `JAUNDER_SYNCED` or `If-Match` interpretation.
  - Verification: focused request-header tests plus live create/read →
    conditional mutation, exact validator round-trip, and a specifically
    `local-ahead` reconciliation push; stale validators still fail.
- [x] **Exclude AtomPub in operator Caddy source:** Capture external
      HEAD/status/intended-file diff, then change only
      `/home/mdorman/src/tendentious-nix/services/caddy/default.nix` so the
      Jaunder site's encoder excludes `/atompub/*`; preserve its separate legacy
      site.
  - Verification: test the path matcher **without relying on server
    `no-transform`** using a synthetic upstream: AtomPub remains identity while
    a non-AtomPub compressible response is Zstandard-encoded. Recheck external
    HEAD/status/diff. Do not commit, push, or deploy the external checkout.
- [x] **Prove the complete supported-proxy flow:** Exercise the resulting
      configuration with both `Accept-Encoding: zstd` and `identity` across
      create, Member GET, successful PUT and DELETE; current `If-Match` succeeds
      on PUT/DELETE, stale `If-Match` is `412` without mutation, and
      `local-ahead` reconciliation pushes successfully.
  - Verification: record the full wire matrix separately from both independent
    defense fixtures; ensure the public Syndication Feed and unrelated content
    remain eligible for compression.
- [x] **Record the contract and handoff:** Commit a tracked, numberless draft
      ADR and project it into `docs/ARCHITECTURE.md`; update public deployment
      and Emacs documentation without private host details. Do not promote the
      ADR or edit `docs/README.md`.
  - Verification: documentation gates and final branch review; hand the user the
    exact external patch and independent commit/deploy steps, plus before/after
    state and explicit no-commit/no-push/no-deploy attestation. Commit only the
    Jaunder checkout through `jaunder-commit` and its enforced gate.

## Risk checks and gates

- Keep the two Caddy fixtures independent: unconditional encoding tests the
  server header; a synthetic upstream lacking `no-transform` tests the operator
  path matcher.
- Preserve all pre-existing Cache-Control restrictions and never relax canonical
  `If-Match` comparison. Other application/legacy-site compression stays
  available.
- Focused red/green proof precedes each change; tick task boxes before the
  relevant Jaunder commit gate. The separate external checkout remains
  uncommitted and undeployed throughout.
- After deliverable review, use the pre-push hook and PR CI for branch proof;
  use a local broad gate only when the finding/risk calls for it.
