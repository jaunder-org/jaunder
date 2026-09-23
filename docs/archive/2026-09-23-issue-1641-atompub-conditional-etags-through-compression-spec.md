# Issue #1641 — Preserve AtomPub conditional ETags through response compression

## Outcome

A strong ETag returned for an AtomPub Post remains usable as `If-Match` through
a supported response-compressing deployment. The Emacs Protocol Client can
create, read, update, delete, and reconcile Posts without a compression-induced
false stale-write response; genuinely stale writes still fail closed.

## Load-bearing decisions

- The server's canonical strong content ETag remains the only accepted Post
  validator. Do not strip or trust proxy-specific suffixes such as `-zstd`,
  normalize arbitrary `If-Match` values, or weaken conditional writes.
- AtomPub responses carry a server-owned `Cache-Control: no-transform` contract,
  preserving any other applicable cache restrictions. A compliant intermediary
  must not change their representation or rewrite their ETags, regardless of
  whether the Protocol Client advertises a content coding. The public
  Syndication Feed and unrelated application responses retain their own
  compression policy.
- The supported production Caddy site additionally restricts `encode zstd gzip`
  to non-`/atompub/*` paths. Keep compression on other application responses; do
  not change the independent legacy site. This operator-specific rule lives in
  `/home/mdorman/src/tendentious-nix/services/caddy/default.nix`, which
  currently appends an unconditional `encode` to the Jaunder virtual host. Edit
  that source file only after approval; **do not commit or deploy** that
  checkout as part of this cycle.
- The Emacs Protocol Client requests `Accept-Encoding: identity` on its AtomPub
  transport as a second line of defense. It does not invent an alternative
  validator or silently accept a suffixed strong ETag.
- Previously saved suffixed `JAUNDER_SYNCED` markers have already been handled
  outside this issue. Do not add migration, suffix stripping, or automatic
  marker recovery.
- The Jaunder source, tests, and public operator/Emacs documentation belong to
  issue #1641's branch and PR. The separate operator source edit remains local
  and uncommitted. Its handoff identifies the exact intended patch and the
  independent commit/deployment steps for the operator to perform later; this
  cycle performs neither.

## Acceptance

1. A regression test reproduces an intermediary turning a current AtomPub Post
   ETag into a compression-suffixed validator and demonstrates the false `412`
   before the fix.
2. Direct responses across `/atompub/*` assert `Cache-Control: no-transform`,
   including create, Member GET, PUT, and DELETE; adding it preserves any
   pre-existing `Cache-Control` directives. With an otherwise **unconditional**
   Caddy `encode zstd gzip` fixture and `Accept-Encoding: zstd`, create and
   Member GET still produce identity bytes and unsuffixed strong ETags; a
   conditional mutation with one succeeds. This independently proves the
   server-owned defense even without the operator's path exclusion.
3. Create, Member GET, successful PUT, and successful DELETE through the
   supported proxy path expose or accept reusable validators. A client
   requesting Zstandard and one requesting identity both work. A current
   `If-Match` succeeds on PUT/DELETE; an actually stale `If-Match` still returns
   `412` without changing the Post. A reconciliation `local-ahead` push no
   longer fails solely because of the response coding.
4. The Emacs transport explicitly requests identity coding and retains the exact
   unsuffixed ETag in `JAUNDER_SYNCED` on create and read; the next conditional
   mutation sends it unchanged.
5. Independently of `no-transform`, the operator's Caddy source restricts
   compression away from `/atompub/*` while a non-AtomPub compressible response
   can still be Zstandard-encoded. Record the external checkout's HEAD/status
   and intended-file diff before and after editing so pre-existing state is
   distinguishable. The handoff supplies the exact patch, independent operator
   commit/deployment instructions, and an explicit attestation that this cycle
   did not commit, push, or deploy that checkout.
6. Jaunder's deployment and Emacs documentation explain the `no-transform`,
   proxy exclusion, identity-request, and canonical `If-Match` contract; no
   secret or real host detail is published.

## Boundaries

- No server-side interpretation of arbitrary intermediary ETags,
  conditional-write bypass, or automatic repair of historical Emacs files.
- No compression-policy change for public Syndication Feeds, web pages, assets,
  or the independent legacy site.
- No external-repository commit, push, deployment, or alteration of pre-existing
  local changes.
