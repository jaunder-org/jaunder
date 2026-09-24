# ADR-0211: Resolve Emacs Post Conflicts with Explicit, Revalidated Choices

- Status: accepted
- Date: 2026-09-23
- Issue: [#1640](https://github.com/jaunder-org/jaunder/issues/1640)

## Context

A matched Emacs Post becomes a reconciliation `conflict` when both its local
source and remote Member have changed since synchronization. Ordinary push and
pull intentionally block it. The User currently has to move the file aside,
fetch the remote Post, decide which authored content to keep, and manually
preserve identity and synchronization metadata. This is error-prone even though
the Protocol Client holds the reviewed local digest and remote strong ETag.

[ADR-0200](0200-revalidated-matched-post-pull.md) authorizes staged, revalidated
local replacement only for a `server-ahead` Post;
[ADR-0047](0047-emacs-publish-orchestration.md) makes a conditional publish safe
to retry after server success but local write-back failure. Neither provides a
way for the User to explicitly resolve a true conflict. A last sync ETag is not
a saved content ancestor, and network loss after a PUT may conceal a remote
commit.

## Decision

Only a uniquely matched, reviewed `conflict` row admits explicit keep-local,
keep-remote, or merge. All three require a fresh remote Member identity and
strong ETag matching the reviewed evidence, and an unchanged local identity,
path, byte digest, and clean visiting buffer. No action silently accepts newer
remote content as the new precondition. Keep-local and keep-remote support
ordered confirmed batches; merge is one interactive Post at a time. Successful
resolutions refresh the report while retaining ordered terminal results.

Keep-local sends the reviewed local authored representation under a fresh
`If-Match` without first rewriting local synchronization metadata. Keep-remote
stages the Member and uses ADR-0200's final checks and recoverable atomic local
replacement/rename. Merge uses two-way Ediff over actual local and staged remote
authored representations, with a separately editable result and no invented
ancestor. Identity, slug, and synchronization markers are client-managed; the
User explicitly completes the merge before its result can be conditionally
published. Initial staging failure preserves both Posts and produces no editable
result. Once a result exists, cancellation or blocked completion preserves both
Posts and keeps edited scratch work available until the User discards it.

The safety guarantee concerns the **Posts**, not rollback of verified Local
Media Copies or uploaded Media. A rejected conditional write changes neither
Post. A lost PUT response is an **unknown remote outcome**, so no automatic
retry or unchanged claim is allowed; the local file and merge scratch remain for
fresh reconciliation. After a confirmed remote commit, failed local write-back
or rename is **partial success** with explicit recovery guidance, not a
fictitious two-sided rollback. This extends ADR-0047's ID-first write-back and
ADR-0200's recoverable rename, without claiming a transaction across HTTP and
the filesystem.

## Consequences

The User can resolve the motivating local-authoritative title loss without
hand-editing `JAUNDER_SYNCED`, and can inspect both versions before deciding
what to merge. Remote ETag changes, local edits, duplicate identities, and
modified buffers block rather than permitting an unintended overwrite.

Post-commit and lost-response outcomes remain visible, recoverable, and
non-atomic. Media side effects may persist after an otherwise blocked action;
reconciliation must report the committed/unknown side honestly. Pure and live
Emacs tests must cover the eligibility, revalidation, Ediff, batch, and
partial/unknown-outcome boundaries.
