# Issue #80 — Durable local media copies for pulled Posts

## Outcome

A Post pulled through the Emacs Protocol Client is previewable offline when its
native source links to media served by the configured Jaunder instance. The
client downloads verified bytes as durable **Local Media Copies** under
`local-media/` and rewrites only those link destinations before exposing the
pulled Post.

## Load-bearing decisions

- Localization applies only to a server-only Post pull. It does not change
  matched-Post reconciliation, divergence handling, publishing, or ordinary
  local file deletion. Markdown and HTML republish remain outside this issue's
  existing Org-only publish behavior.
- Org, Markdown, and HTML Posts receive equivalent behavior through format-aware
  link handling:
  - Org link targets;
  - Markdown link and image destinations;
  - HTML `src`, `href`, `poster`, and individual `srcset` destinations.
- Bare URL text, CSS URLs, scripts, embedded data, malformed links, and non-link
  occurrences are never rewritten. Link labels, descriptions, alt text,
  ordering, and URL fragments are preserved.
- A candidate must be an absolute HTTP(S) URL on the configured Jaunder origin,
  contain no user information or query, and match Jaunder's canonical public
  media route. External origins, scheme-relative URLs, AtomPub Member URLs,
  credential-bearing URLs, and other same-origin resources remain unchanged and
  are not fetched.
- The authenticated Member response and every media response must carry exactly
  one syntactically canonical `X-Jaunder-Instance` UUID, and every value must
  match. Missing, duplicate, malformed, or mismatched identity fails closed.
- Media binary GETs are anonymous and never use the App Password. Redirects are
  disabled; only a direct `200` response is accepted. AtomPub authentication
  remains confined to the Member request.
- The client preserves response bytes exactly. It computes SHA-256 and requires
  it to equal both the canonical media URL's content hash and the response's
  strong `"sha256-<64-lowercase-hex>"` ETag.
- Decode the server's canonical percent-encoded filename once and validate the
  result as a safe local leaf. The Local Media Copy is stored at
  `local-media/<sha256>/<decoded-filename>` under the configured root.
- `local-media/` contains durable managed content, not an evictable cache. No
  automatic expiry, deletion, or repair scan is introduced.
- Org rewrites to a relative `file:` target; Markdown and HTML rewrite to a
  relative URL path. Each target uses the server's canonical percent-encoded
  filename exactly once, so the consumer resolves the decoded on-disk leaf;
  literal percent signs, spaces, and non-ASCII bytes remain unambiguous.
  Original fragments are preserved in the native syntax.
- Duplicate references download once. Repeated Posts and retries reuse an
  existing local target only after hashing its bytes and confirming the same
  digest. A mismatching entry is corruption: fail loudly and never overwrite.
- The configured blog root is trusted, author-owned local state. Path creation
  and each immediate mutation reject symlinks and non-directory components under
  `local-media/`; staging files are created exclusively and files are never
  overwritten. A malicious concurrent replacement after Emacs's final path check
  is out of scope because Emacs Lisp has no dirfd-anchored mutation.
- One Post pull is fail-closed. Download all candidate media to temporary files,
  validate every response and digest, install verified Local Media Copies, then
  atomically install the Post file last. Any failure leaves the Post destination
  absent and cleans temporary files.
- Verified Local Media Copies installed before an ordinary late failure are
  retained, just as after a process crash. They are safe and reusable; no
  rollback, cross-file transaction, or orphan collector is added.
- Retry is the existing user action: rerun `jaunder-reconcile`. Because the Post
  file remains absent, it is still server-only; verified media already present
  is reused. No retry queue or second pull command is introduced.
- An occupied `<slug>.org` Post destination remains blocked before the Member
  request or media work, preserving the existing no-clobber contract.
- Reconciliation keeps its current deterministic, sequential, fail-fast batch
  behavior. A later invocation retries the failed and not-yet-attempted Posts.

## Acceptance

- Pure fixtures for Org, Markdown, and HTML rewrite every supported canonical
  media link destination to the exact relative `local-media/` target while
  preserving labels, fragments, order, and unrelated source bytes. Encoded
  filename fixtures cover spaces, literal percent signs, and non-ASCII names,
  proving each native target resolves to the decoded local leaf.
- External, scheme-relative, queried, credential-bearing, non-media same-origin,
  AtomPub Member, bare-text, CSS, script, data, and malformed URLs remain
  byte-identical and cause no download.
- Duplicate references and separate Posts with the same hash and filename use
  one verified local file; a retry reuses it without another binary GET.
- A real-server pull downloads the uploaded media anonymously, verifies the
  instance identity, `"sha256-<hash>"` response ETag, URL hash, and bytes,
  installs the native Post last, and remains previewable after the server
  becomes unavailable.
- Republish of a pulled Org Post uploads its Local Media Copies through the
  existing publish path, server-deduplicates them to the authoritative URLs, and
  leaves the relative native body links and Local Media Copy bytes/path
  unchanged. Ordinary publish metadata writeback remains allowed. Markdown and
  HTML republish behavior is unchanged and is not acceptance for this issue.
- Redirect, non-`200`, transport failure, missing, duplicate, malformed, or
  mismatched instance identity, malformed canonical filename, URL/ETag/body-hash
  mismatch, unwritable path, symlink path, and existing-byte mismatch each
  surface a specific error, leave the Post absent, preserve existing entries,
  and remove temporary files. A failure after the first media install retains
  that verified Local Media Copy, cleans remaining temporaries, and reuses it
  without another GET on retry.
- An initially failed pull succeeds when reconciliation is rerun after the fault
  is removed. An occupied Post destination performs no Member or media request
  and stays byte-identical.
- Pure ERT covers candidate extraction, format-preserving substitution, URL and
  path validation, identity-header cardinality and syntax, deduplication, digest
  verification, install ordering, retained verified entries, and every failure
  branch. Live ERT covers anonymous download, offline preview, retry, reuse, Org
  republish deduplication, and no-clobber behavior.

## Boundaries

- No arbitrary external-media downloader, cross-origin redirects, credentials on
  media GETs, best-effort partial localization, fallback to remote links, or
  rewriting of author source outside the newly pulled file.
- No media cache eviction, matched-Post repair, orphan collection, reconcile
  report redesign, background synchronization, bulk retry state, or new server
  endpoint.
- No change to server media identity, canonical filename encoding, Post body
  canonicalization, AtomPub authentication, or Deleted Post semantics.
