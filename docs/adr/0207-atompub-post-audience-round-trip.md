# ADR-0207: AtomPub Post Audience Round-Trip

- Status: accepted
- Date: 2026-09-22
- Issue: [#1637](https://github.com/jaunder-org/jaunder/issues/1637)

## Context

Jaunder's AtomPub interface carries native Post source and structured Post
metadata between editing clients and the server. The Emacs Protocol Client maps
its local Org metadata block into a format-neutral Entry representation, while
the server canonicalizes recognized Org metadata out of the stored source.

The server already accepts repeated `JAUNDER_AUDIENCE` Org properties, but the
Emacs client strips the local metadata block before serialization. An explicit
local audience therefore disappears: create applies the instance Default
Audience, and update preserves the Post's prior audience. Re-inserting the
property into `atom:content` would make audience the sole exception to the
body-only content contract established by
[ADR-0024](0024-server-side-org-canonicalization.md) and would still leave
responses unable to report the current audience.

Audience is mutable Post state and can contain multiple targets. ADR-0020 makes
those targets a union: Public admits everyone without erasing narrower targets,
Subscribers and every Named audience admit their respective members, and Private
is the empty target set. Retaining dominated targets is intentional — an author
can temporarily add Public and later remove it to restore the prior narrower
union. Audience omission also has deliberate create/update meaning, and silently
ignoring an unsupported extension can publish a Post with unintended visibility.
The protocol needs a structured, discoverable, bidirectional representation that
preserves those semantics without replacing the existing raw-Org metadata
contract.

## Decision

The existing Jaunder Atom namespace, `https://jaunder.org/ns/atompub`, gains a
repeated text-valued `j:audience` Entry element. Each occurrence contains
exactly one canonical audience token: `public`, `subscribers`, `private`, or
`named:<id>`. A Named ID is spelled `[1-9][0-9]*` and must fit in signed 64-bit
range; zero, signs, negatives, and leading zeros are rejected rather than
normalized.

The wire projects [ADR-0020](0020-content-visibility-and-subscription-model.md)
directly. Any deduplicated combination of Public, Subscribers, and Named
audiences is valid and retained even when Public currently dominates effective
visibility. Private represents the empty target set and is valid only by itself.
Canonical output orders Public, then Subscribers, then Named audiences by
ascending numeric ID.

An incoming nonempty `j:audience` set is structured audience presence. It wins
as a complete set over audience values in an Org metadata block, following the
structured-input precedence established by
[ADR-0155](0155-server-side-org-metadata-block.md). Absence preserves the
existing contract: an Org header may supply the field; otherwise create applies
the Default Audience and update preserves the current audience. Empty,
duplicate, malformed, unauthorized, and Private-plus-other representations
reject the complete write.

Authenticated AtomPub Member and Collection Entries always emit the complete
current target set. Private is explicit rather than encoded as omission, and a
broader target never causes narrower retained targets to disappear from the
response.

The Service Document keeps Jaunder extension version `1` and adds the additive
feature token `audience`. Support is advertised only by a direct
Jaunder-namespace `j:extension` carrying supported version `1` and that feature
token; a foreign namespace, absent or unsupported version, or absent token does
not qualify. A client with an explicit audience must require this exact
advertisement before mutation. Clients without explicit audience remain
compatible with servers that do not advertise it.

The Emacs Protocol Client maps repeated local `JAUNDER_AUDIENCE` properties to
and from repeated `j:audience` elements. Audience becomes part of pull and
reconciliation state. The canonical target projection also contributes to the
strong AtomPub Member ETag, so an audience-only change becomes server-ahead or
conflicting state and a stale conditional update cannot overwrite it. Named
audiences remain authorable only through canonical numeric IDs; discovery and
friendly selection are separate work.

This decision extends, rather than replaces, the format and slug extension
contract in [ADR-0023](0023-atompub-jaunder-wire-extensions.md). Raw Org clients
may continue using `JAUNDER_AUDIENCE`; the Atom element is the canonical
structured representation for extension-aware clients.

## Consequences

Audience state round-trips without contaminating canonical native source or
creating a special exception to body-only Atom content. The wire preserves
ADR-0020's union, including temporarily dominated targets, rather than
projecting only effective visibility. A remote audience change is visible to
reconciliation, and an older server cannot silently accept an explicit local
audience with different visibility semantics.

The AtomPub mapping, Member serialization, Service Document capability list,
Emacs Entry representation, pull mapping, and reconciliation comparison all
carry audience state. Both server and client require pure mapping tests and live
integration proof because visibility correctness depends on the complete wire
path.

The public extension surface grows beyond read-only slug metadata. Other AtomPub
clients ignore the foreign element safely, but only clients that understand the
advertised `audience` feature can intentionally author audience state. Friendly
Named audience discovery remains absent, so authors must use stable numeric IDs
until a separately designed discovery surface exists.

Adding audience to the Member representation and strong ETag invalidates every
previously stored Emacs synchronization ETag once after a server upgrade.
Unchanged local Posts become server-ahead and can be fetched to install
canonical audience metadata and the new validator. A simultaneously edited local
Post remains a conflict: documentation gives a preserve-local, fetch-remote,
reapply-and-conditionally-publish recovery procedure rather than introducing an
automatic merge that could guess wrong about content or visibility intent.
