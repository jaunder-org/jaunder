# Replacing current image references without claiming retirement

This is an operator preparation and disposable-rehearsal procedure, not
authority to inspect or change a production instance. Obtain explicit
instance/owner/scope approval before any real inventory or write.
Historical-original retirement and its script are separate work in
[#1714](https://github.com/jaunder-org/jaunder/issues/1714). Do not delete
hash-addressed files, rewrite Post Revisions, purge caches or change retention
policy as part of replacement.

## Preconditions and inventory

Use a deployment whose mandatory image preparation and required format/fidelity
checks have been accepted. A failed upload is not a replacement. In particular,
the pinned native editor's ordinary PNG timestamp/textual-XMP gap must not be
worked around by accepting an original, stripping a test witness, rejecting the
whole family as delivered, or adding an unapproved editor. General #1702 release
acceptance and licensing are separate prerequisites; this procedure grants
neither.

Prepare one private inventory row for **each of at least two images**, A and B.
Use actual authorized observations, not example production URLs or guessed
hashes. For each row record:

- Instance identity and owner; exact `source`, SHA-256 and canonical filename;
  current public URL, successful GET hash, strong ETag, byte size and MIME.
- Every authorized current Post reference: Post ID, Member URL, current strong
  Member ETag, native authoring format and exact reference to replace.
- Owner-retained Post/Revision references and independently owned Media Records
  that the authorized owners can establish. Keep an unknown/legacy/unproven
  reference as uncertainty, not as proof that no other owner depends on the
  bytes.
- The untouched author-original source and any durable Local Media Copies, plus
  the intended new identity and conditional-write results once observed.

Use the owner's Media library, Post/history views and ordinary authenticated
Member reads. Cross-owner records require the respective authorized evidence; an
empty owner view is not a global inventory. Never bypass a nondisclosing refusal
to discover another owner's IDs. Keep this inventory private instance data, not
committed source or a public issue attachment.

## Create and verify a replacement for A, then B

1. Preserve the source image unchanged. Upload a copy through the normal Media
   library or AtomPub Media Collection (`POST /atompub/{username}/media`). Do
   not write directly into the public Media tree. Upload permission, positive
   size limits, available preparation capacity and quota still apply; busy/error
   responses do not authorize original-byte fallback.
2. Take the authoritative binary URL from the returned Entry's `<content src>`;
   `Location` identifies its Media Member, not the binary. A new filename alone
   is not evidence of cleaned content. Require a new sanitized hash when the old
   original contains metadata that preparation removes. Identical input and
   filename, or its already-clean reupload with that filename, may deduplicate.
3. Retrieve the returned binary URL. Verify successful bytes against its URL
   hash and strong ETag, detected MIME and stored size/accounting. Independently
   inspect ordinary metadata removal and applicable fidelity using the accepted
   procedure for that format. Do not treat an editor's success or edit count as
   proof. Record the new identity in the private inventory.
4. Recheck the old identity: its bytes, URL and ETag have not been rewritten. A
   legitimately preexisting original can still return 200. Do **not** apply a
   new-upload test's “original hash was never published” assertion to old Media.

## Update only current references

For each inventoried current Post, fetch the current Member and strong ETag,
edit the exact intended references in its native source, and send the ordinary
Member `PUT` with that ETag in `If-Match`. Preserve unrelated title, source,
audience, publication and other metadata. A stale precondition means refetch and
review; do not force an unconditional overwrite.

Read the Member back and verify the new authoritative references and resulting
ETag. Verify the previous state in owner history: a meaningful edit captures an
immutable Post Revision containing the old reference. Updating current Posts,
unpublishing or deleting them does not erase that history. Repeat the verified
sequence for the second image and each of its current Posts.

## Refusals, record removal and observed old-byte availability

Normal AtomPub Media `DELETE` has **no force option**. Owner-retained history
normally yields 409 with only that owner's Post IDs.
Legacy/unknown/global-safety refusal yields 409 without another owner's IDs.
Neither response means the old image is gone.

The web Media library's explicit owner-only force action may knowingly remove
that owner's Media Record despite owner-retained history. Require the owner's
specific acknowledgment of possible broken historical references before using
it. It is not an AtomPub parameter and never overrides global safety. A
qualifying independent owner's record remains independent, with that owner's
quota/accounting intact. Do not remove it on the first owner's authority.

After every refusal or accepted record removal, perform an ordinary fresh public
GET of the **old exact URL**, without a conditional 304 shortcut. Record actual
status and, on 200, its SHA-256, size and strong ETag. Also recheck the
replacement and relevant record/accounting state through supported owner reads.
Record removal is not a promise of filesystem unlink or a 404: retained
references or another record can keep old bytes available, and public serving
can still open a retained file after the last owner's record is removed.
Conversely, a 404 alone does not prove retirement from caches, copies or
backups. Do not manually unlink files to obtain a preferred result.

If global safety refuses, stop the removal step. Preserve the disposition and
uncertainty for separately authorized analysis; do not invent a global force,
raw SQL cleanup, retention waiver or filesystem workaround.

## Local and remote copies

- Preserve author originals. Update current authored links deliberately; stale
  sources can republish old references even after the current server Post
  changes.
- A Local Media Copy is durable managed content, not an evictable cache. Its
  identity is bound to serving-instance proof, URL hash and strong ETag. Never
  put new bytes under an old hash/path or weaken client checks. Verified pull
  and same-filename republish use the sanitized identity without altering the
  author's original.
- Existing immutable URLs/ETags and already-downloaded Local Media Copies do not
  change. Carry these copies with Post backups/synchronization and handle stale
  sources deliberately.
- Immutable proxy/browser caches, instance backups, retained history and remote
  downloads may retain original bytes. Remote copies cannot be recalled. Any
  actual historical-byte retirement requires the separately approved #1714
  procedure and authority, not this replacement operation.

## Disposable rehearsal and maintenance

Before operational use, rehearse A and B with owned non-personal inputs and
isolated SQLite/PostgreSQL state. The maintained
[`media_replacement` integration tests](../server/tests/atompub/media_replacement.rs)
seed exact preexisting objects only as test fixture state, then use real native
uploads and supported HTTP writes. They check conditional current-reference
replacement, retained old Revision references, owner-history refusal and web
record override, independent sharing/accounting, non-overridable legacy global
safety, old public bytes/ETags and private-spool cleanup. Their execution
receipts belong in ignored/session evidence, not in this runbook. No sanitizer
bypass is part of an operational interface.

Maintainers must update this procedure and its rehearsal when Media identity,
conditional Member writes, retention/reclamation, client-copy trust or supported
preparation changes. The controlling policies are
[Media Record independence](adr/0183-per-user-media-records-from-local-post-references.md),
[retained Post history](adr/0136-local-post-lifecycle.md) and
[live reference ownership](adr/0154-media-reference-live-ownership.md).
