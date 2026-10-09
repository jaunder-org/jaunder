# ADR-DRAFT: Share remote Collection evidence within Emacs pull batches

- Status: proposed
- Date: 2026-10-09
- Issue: [#1716](https://github.com/jaunder-org/jaunder/issues/1716)

## Context

Matched Post pulls and keep-remote conflict resolutions require fresh identity
and replacement evidence under
[ADR-0200](../0200-revalidated-matched-post-pull.md) and
[ADR-0211](../0211-emacs-reconciliation-conflict-resolution.md). Enumerating the
complete Collection at every row revalidation makes network work grow with both
Collection size and batch length. Keep-remote has two such boundaries per Post.

Post IDs are database primary keys on both backends. Supported writes cannot
create two Posts sharing one ID. A Collection enumeration can nevertheless
observe malformed duplicate Entries, and the client rejects them. Repeating that
global defensive check for every row is distinct from checking the selected
Member's current identity and strong ETag. Collection pagination is not an
atomic snapshot, as [ADR-0209](../0209-atompub-collection-member-etags.md)
already specifies.

## Decision

One confirmed Emacs reconciliation pull or keep-remote batch may share a single
complete remote Collection enumeration across its rows. Acquire it when needed,
scoped to the active blog and root, independently of the reviewed preview.
Preserve pagination validation and duplicate remote-ID rejection. Failed or
partial acquisition is not valid proof and is not retried independently for
every selected Post. Discard the evidence when the operation ends; final report
refresh and later batches acquire their own fresh evidence.

Share remote Members only. At each matched-row revalidation boundary, re-read
local identity and uniqueness, including a last local scan after Media
finalization immediately before replacement. Retain fresh selected-Member
checks, reviewed strong ETags, staged identity checks, local path and digest
checks, clean visiting buffers, destination exclusivity, Media integrity, and
the recoverable replacement/rename sequence. A Collection ETag never replaces an
operation-time Member check.

Pull-time Local Post Link localization may use the batch remote evidence without
another complete enumeration per Post, while retaining unique target proof and
current local target validation under
[ADR-0201](../0201-emacs-local-post-link-round-trip.md). Missing or invalid
localization proof preserves the canonical URL.

This refines the remote evidence acquisition used by batch pull and keep-remote;
it does not relax ADR-0200 or ADR-0211's selected-Post replacement
preconditions. Push, keep-local, interactive merge, standalone pull, and
publish-time link resolution are unchanged.

## Consequences

Operation-time Collection request count grows with Collection pages, not
selected Post count. The final fresh report remains a separate Collection walk.
Local checks stay current after earlier rows create or rename files.

After successful acquisition, a newly faulty Collection response that would
contain duplicate Entries is not observed until another enumeration. This is an
accepted reduction in repeated global fault detection, not permission to accept
a changed selected Member or overwrite changed local bytes. The batch does not
claim a remote snapshot or a transaction across HTTP and the filesystem.
