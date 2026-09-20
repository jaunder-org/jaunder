# ADR-DRAFT: Persist inline-rendered Post titles

- Status: proposed
- Date: 2026-09-19
- Issue: [#1590](https://github.com/jaunder-org/jaunder/issues/1590)

## Context

A Post title is authored in the Post's `Markdown`, `Org`, or `Html` format, but
Jaunder currently presents it as escaped literal text. Consequently inline
markup such as Org emphasis appears as source syntax. Syndication title fields
also have different representation contracts: Atom supports HTML text
constructs, while RSS and JSON Feed titles should contain clean plain text.
AtomPub, slugs, and metadata need the authored title rather than a presentation
projection.

Jaunder already renders and sanitizes Post bodies at write time, persists the
rendered bytes, and preserves them in full Post Revisions. Re-rendering titles
on reads would make an old title adopt the current parser while its body and
revision snapshot retain earlier parser output. It would also amplify parsing
cost across timeline readers and duplicate policy at web and feed translation
seams.

A body-safe fragment is not automatically title-safe. Post headings require
inline-only, non-interactive content: block structure can break the heading,
links can nest inside the heading's permalink, and embedded media introduces
ownership and rendering concerns unrelated to a title.

## Decision

The authored `PostTitle` remains canonical. Host rendering derives a dedicated,
sanitized Rendered Title together with the rendered body whenever a titled Post
is created or its title, body, or format changes. Storage writes the authored
title, format, rendered body, and Rendered Title atomically. Current Posts and
full Post Revisions persist the Rendered Title; titleless records persist none.
The migration only adds nullable columns: because no production instances exist,
it does not repair legacy rows or install a render-on-missing compatibility
path.

A Rendered Title is canonical, trusted, inline-only HTML with a stricter
contract than rendered body HTML. Its closed retained-element set is `b`,
`strong`, `i`, `em`, `u`, `s`, `del`, `code`, `sub`, `sup`, `mark`, `small`, and
`br`, with no attributes. Markdown and Org use every textual inline construct
their parsers project into that policy; HTML titles are untrusted fragments
under the same policy. A dedicated, narrowly configured `ammonia` builder owns
this sanitization, including removal of active and embedded content. It does not
project image alternatives or insert spaces for removed block wrappers. A source
with no surviving visible text persists an empty fragment; web, RSS, and JSON
Feed omit its title presentation rather than leaking authored markup, while Atom
emits its required empty HTML title construct.

Web Post article headings consume the persisted HTML. Atom Syndication Feed
entry titles use an HTML text construct. RSS and JSON Syndication Feed titles
use a plain-text projection produced by stripping the persisted fragment with an
empty-tag `ammonia` builder. `html-escape` decodes its entities once, `br`
becomes a space, and whitespace collapses; they never receive Markdown, Org, or
HTML source syntax. Feed fingerprints and serializer revisions cover the chosen
projections.

Slugs, document metadata, AtomPub, editing, and source-oriented administration
continue to use the authored title. AtomPub therefore remains the native-source
surface established by [ADR-0015](../0015-atompub-serialization-surfaces.md).

## Consequences

Rendered title bytes have the same parser-version and historical-snapshot
semantics as persisted body HTML under
[ADR-0123](../0123-rendered-html-storage-decode.md) and full Post Revisions
under [ADR-0136](../0136-local-post-lifecycle.md). Parser changes affect
existing records only through an explicit rewrite or a later content update.

Both storage backends require schema parity for current Posts and Revisions,
plus atomic mutation, semantic no-op, backup, restore-validation, and fixture
coverage. New writes always persist a derivative for authored titles; an
intentionally empty fragment remains a present derivative.

The common wire/storage type reconstructs trusted persisted bytes only after a
field-specific, non-rewriting recognizer confirms the bounded canonical fragment
grammar. The recognizer is dual-target validation, not an authoring parser or
sanitizer; production construction remains host-owned, following the
sanitization boundary in [ADR-0079](../0079-rendered-html-sanitization.md).
Invalid persisted bytes fail typed reads and therefore cannot reach an unescaped
sink. Backup restore retains
[ADR-0174](../0174-backup-format-and-schema-compatibility.md)'s
restore-and-report policy: invalid bytes are restored and diagnosed, but are not
blessed as a Rendered Title by subsequent reads. The CSR and public projector
paint identical validated bytes without carrying Markdown, Org, or sanitization
machinery.

This adds storage and migration complexity, but avoids reader-amplified parsing,
keeps bodies and titles in one parser era, and gives every protocol an explicit
projection instead of leaking secondary markup into plain-text fields.
