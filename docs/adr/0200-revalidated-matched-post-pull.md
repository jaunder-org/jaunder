# ADR-0200: Permit revalidated replacement when pulling a matched Post

- Status: accepted
- Date: 2026-09-17
- Issue: [#1565](https://github.com/jaunder-org/jaunder/issues/1565)

## Context

The Emacs Protocol Client currently pulls only a server-only AtomPub Member.
[ADR-0160](0160-emacs-pulled-media-local-copies.md) deliberately requires an
unoccupied destination and installs the Post last after every Local Media Copy
is verified. That rule makes creating a missing local Post safe, but it prevents
an explicit pull of a matched Post that reconciliation has classified as
server-ahead.

Replacing a matched local file can discard an edit made after the reconciliation
report, overwrite a modified Emacs buffer, or leave an open clean buffer showing
stale bytes that it can later save over the pull. A server change after the
report can likewise make the reviewed Member stale. If the server-assigned slug
changes, replacing bytes and moving the local path cannot be one filesystem
transaction; the sequence must remain recoverable between those operations.

## Decision

An explicit pull may replace a uniquely matched `server-ahead` local Post. The
report captures the local path and a SHA-256 digest of its bytes plus the remote
Member's strong ETag. Pull fetches and stages the complete Member representation
and every Local Media Copy under the existing ADR-0160 trust chain, then
revalidates immediately before installation:

- the selected row still has one local Post and one remote Member with the same
  Post ID;
- the remote strong ETag still equals the reviewed ETag;
- the local path still identifies a regular file whose byte digest equals the
  reviewed digest;
- no destination implied by the server's canonical slug is occupied by another
  filesystem entry; and
- any Emacs buffer visiting the local path is unmodified.

Failure of any condition leaves the local Post untouched and returns a blocked
result. Selection never weakens these checks.

After revalidation, the client atomically replaces the existing file at its
current path with the staged bytes. If the canonical slug changes the filename,
it then atomically renames that updated file to the unoccupied destination. A
crash between replacement and rename leaves one valid, updated Post carrying the
same Post ID at the old path; the next inventory can identify it and safely
complete the rename. Local Media Copies installed before a later failure remain
durable and reusable under ADR-0160.

When the destination is visited in a clean buffer, successful installation
refreshes that same buffer to the installed bytes, updates its visited filename
when needed, keeps it unmodified, and preserves its displayed windows and point
where possible. A modified visited buffer is always blocked rather than reverted
or saved.

This decision supersedes only ADR-0160's server-only reconciliation boundary and
occupied-destination rule for a uniquely matched, explicitly selected,
server-ahead Post. Server-only pull and all Local Media Copy validation,
no-overwrite, staging, and trust-chain rules remain unchanged.

## Consequences

A User can explicitly accept a reviewed remote change without manually deleting
and recreating the local file. Concurrent remote edits, local disk edits,
modified buffers, duplicate identities, and destination collisions fail closed.
The local byte digest is a safety snapshot, not a second Post identity; Post ID
remains the join key.

Matched pull gains a recoverable two-step path when the canonical slug changes.
The temporary old-filename state after a crash is intentionally recognizable and
retryable rather than hidden behind a multi-file transaction Emacs cannot
provide. Clean open buffers follow the installed file instead of retaining stale
content; modified buffers retain user authority.

Tests must cover remote and local changes after report generation, modified and
clean visited buffers, canonical-slug rename, destination collision, and
recovery between replacement and rename, in addition to ADR-0160's existing
Media and exclusive-install cases.
