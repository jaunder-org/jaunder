# ADR-DRAFT: Reuse verified original Media destinations on matched Org pull

- Status: proposed
- Date: 2026-10-07
- Issue: [#1703](https://github.com/jaunder-org/jaunder/issues/1703)

## Context

[ADR-0045](../0045-emacs-media-content-src.md) keeps authored local Media links
unchanged while publishing harvested server URLs. Pull's
[ADR-0160](../0160-emacs-pulled-media-local-copies.md) trust chain verifies
remote bytes before creating durable Local Media Copies. Applied to
[matched pull](../0200-revalidated-matched-post-pull.md) and
[conflict staging](../0211-emacs-reconciliation-conflict-resolution.md), that
mandatory-copy rule replaces a still-usable original link with a second file.
Server regeneration can therefore disrupt local authoring without changing
Media.

Filename matching cannot prove identity. Persisted upload provenance would add
state whose validity still depends on current file bytes, while searching the
root by hash would select files the author never referenced in that Post.

## Decision

For matched Org server-ahead pull, keep-remote, and merge's staged remote
representation, reuse an original body-level relative file destination only when
its actual bytes equal verified remote Media bytes. The eligible destinations
come exclusively from that matched local Post, resolve inside its configured
root, and have only non-symlink directory components and a non-symlink regular
file target. Exclude Local Post Link candidates, attachment links, absolute
paths, queries, and Org search targets. The relative destination must resolve to
the verified file from the final Post location, including a slug rename.

Exactly one distinct eligible authored file destination must match a remote
Media hash. Distinctness uses exact authored spelling excluding fragments:
`file:images/a.png`, `images/a.png`, and `./images/a.png` are distinct even when
resolving to one file. Repeated uses of one spelling are allowed; multiple
matching spellings are ambiguous and fall back. Filename equality is neither
proof nor a requirement. Preserve the file destination's authored spelling and
encoding, but use the remote link's fragment and description and retain all
other remote authored edits. No positional pairing of old and remote links
establishes identity.

Reuse does not bypass remote verification: canonical active-origin URL,
Member/media instance UUID agreement, anonymous direct 200 response, strong hash
ETag, and downloaded byte hash must all pass ADR-0160. For server-ahead and
keep-remote, recheck eligible original files at the final local-install
boundary. Missing, changed, unreadable, unsafe, or ambiguous originals use
normal verified Local Media Copies before the Post is installed; unexpected
infrastructure/I/O failures remain visible failures. Successfully reused
originals create no redundant Local Media Copy and are never modified. Remote
bytes may still be fetched for verification.

Retain matched Post identity, digest, clean-buffer, unique-match, reviewed
remote ETag, destination-collision, and recoverable replacement/rename
safeguards. Merge verifies reuse during initial remote staging. Thereafter links
are ordinary authored merge content, retaining the existing conditional-publish
and conflict checks. No automatic stale-reuse fallback rewrites the completed
merge result or substitutes bytes after publication. The final-check filesystem
race accepted by ADR-0160 remains out of scope, as do rollback and orphan
collection.

This supplements ADR-0160's mandatory-copy rule only for verified originals.
Server-only pulls and Markdown/HTML localization remain unchanged. No upload
provenance ledger, directory search, or server behavior is introduced.

## Consequences

Regenerated remote Posts can retain usable authored local Media destinations
without discarding real remote edits or weakening the Media trust chain. Reuse
must carry enough transient evidence to revalidate file bytes before local
installation and to produce fallback copies if that evidence becomes stale.
Ambiguity deliberately favors a durable copy over guessing author intent.

Pure and live Emacs tests must cover all matched-pull consumers, source-span
preservation, fallback and final revalidation, slug renames, and unchanged
fail-closed remote evidence and Post replacement behavior.
