# ADR-DRAFT: Own Collection evidence within Emacs reconciliation operations

- Status: proposed
- Date: 2026-10-09
- Issue: [#1716](https://github.com/jaunder-org/jaunder/issues/1716)

## Context

Complete Collection discovery is repeated inside row preflights and nested Local
Post Link resolution. Matched pulls have one such traversal per Post; keep-local
and keep-remote have two preflights, with publishing potentially adding another
traversal for links. Network work therefore grows with both Collection size and
selection length. The same responsibilities also appear in single-row actions
and interactive merge preparation/completion.

[ADR-0200](../0200-revalidated-matched-post-pull.md) and
[ADR-0211](../0211-emacs-reconciliation-conflict-resolution.md) require fresh
selected-Post replacement/conflict evidence, not indefinite authorization from a
report preview. [ADR-0201](../0201-emacs-local-post-link-round-trip.md) requires
unique remote/local target proof and server-owned canonical URLs. Discovery,
authorization and outcomes are distinct responsibilities; a cache binding that
only bypasses repeated fetches leaves their ordering distributed across callers.

Post IDs are database primary keys on both backends. Supported writes cannot
create two Posts sharing an ID. A Collection response may nevertheless contain
malformed duplicate Entries, which the client rejects. Repeating that global
fault check differs from checking a selected Member's identity and strong ETag.
Pagination is not an atomic snapshot under
[ADR-0209](../0209-atompub-collection-member-etags.md).

## Decision

Use one deep operation-level module for confirmed reconciliation push, pull,
keep-local, keep-remote and delete. A single selected Post is batch size one.
The module owns root/active-origin/User scope, discovery lifetime, fresh local
proof and remote evidence invalidation. Dependencies remain exact under
[ADR-0016](../0016-dependency-injection-and-appstate.md), not a heterogeneous
service bundle. Blog selection continues using
[ADR-0047](../0047-emacs-publish-orchestration.md)'s directory resolution.

Acquire at most one complete Collection traversal when operation discovery is
needed, independently of the reviewed preview. Preserve pagination validation
and duplicate remote-ID rejection. Complete-empty and failed acquisition are
explicit outcomes; a failed/partial walk never supplies valid proof or retries
independently for every row. Delete, link-free push and immediate ineligibility
need not acquire unused discovery merely to meet a fixed traversal count.

Share remote discovery, not mutation permission or a frozen local inventory.
Retain action-specific reviewed identities/strong ETags, fresh targeted Member
checks, conditional writes, local authority and identity/uniqueness, byte digest
where required, clean visiting buffers, destination exclusivity, Media integrity
and recoverable local installation. Matched pull and keep-remote repeat local
uniqueness after Media finalization immediately before their final replacement
guard. Collection ETags remain preview evidence rather than authorization.

After a known create/update/delete, update or invalidate that identity's remote
and link evidence before another consumer, even when local completion fails. An
unknown write outcome invalidates assumptions rather than proving unchanged
state. Valid authoritative response data or fresh targeted Member reads may
restore usable discovery; another per-row Collection walk may not. Never invent
an unknown created Post ID, guess a permalink, automatically retry an uncertain
update/delete, or hide a remote commit behind a local failure. Durable keyed
create recovery continues under
[ADR-0199](../0199-durable-atompub-create-intent.md).

Both publish-time and pull-time Local Post Link processing participate in the
owned scope while retaining current local target proof and their existing
failure policies. Publish aborts visibly without required proof; pull retains
canonical URLs without localization proof. Earlier renames do not authorize
repairing authored links or searching for a target by basename.

End operation evidence before the separate authoritative final report refresh.
Cancellation and independent failures retain ordered terminal outcomes; refresh
failure preserves their recovery evidence. Later, nested and other-root/blog
operations cannot inherit the caller's discovery. Interactive merge uses
separate short-lived preparation and completion scopes, never freshness spanning
human editing; its scratch and terminal-refresh contracts remain unchanged.

## Consequences

Collection work is at most one operation traversal plus one final traversal, not
proportional to selected Post count. The two traversals may have different page
counts after writes. Unneeded discovery is skipped; targeted Member/link checks
still cost work appropriate to selected Posts and referenced targets.

Fresh local checks and invalidation remain necessary after earlier rows create,
rename, change or delete Posts. Partial and unknown results remain honest; the
module does not claim rollback or a transaction across HTTP and the filesystem.

After successful discovery, a newly faulty global duplicate Collection response
is not observed until another traversal. This accepted reduction in repeated
fault detection does not authorize accepting a changed selected Member or
changed local bytes. No operation claims a paginated remote snapshot.
