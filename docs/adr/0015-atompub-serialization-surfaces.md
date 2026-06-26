# ADR-0015: Separate Serialization Surfaces for Syndication and AtomPub

* Status: accepted (content-type token scheme superseded by [ADR-0023](0023-atompub-jaunder-wire-extensions.md); the separate-serializers principle stands)
* Deciders: mdorman, Claude
* Date: 2026-05-29

## Context and Problem Statement

Jaunder already emits public Atom/RSS/JSON feeds for syndication (M8). The new AtomPub interface also exposes a per-user Atom **Collection** of the same posts. It is tempting to reuse the M8 Atom serializer for both. But the two surfaces have different audiences and contracts: the syndication feed is consumed by arbitrary feed readers, while the AtomPub Collection is consumed by an editing client that must be able to `PUT` back what it `GET`s. A Jaunder Post stores a *source* `body` in a `format` (Markdown/Org/Html) and derives `rendered_html` from it — so the question is what content form each surface should carry.

## Decision Drivers

* Reader compatibility: arbitrary feed readers can only render HTML.
* Round-trip fidelity: AtomPub edits must not silently destroy a post's source format.
* Honesty: store and expose content in the form the user actually authored.

## Decision Outcome

Chosen option: **Two deliberately separate serializers.**

* The **public Syndication Feed** (M8) always emits `rendered_html` as Atom `type="html"`, because its consumers can only render HTML.
* The authenticated **AtomPub Collection** serializes each post in its **native source form** — Markdown/Org as `type="text"` (carrying the raw `body`), Html as `type="html"` — so that what a client fetches round-trips losslessly back through `PUT`.

### Implementation Details

1. The syndication serializer and the AtomPub Collection serializer are distinct code paths; neither is forced to serve the other's audience.
2. Native-source serialization, combined with the per-user `default_post_format` preference, lets a Markdown post fetched via AtomPub return as Markdown source and be edited and saved without format conversion.
3. Incoming content type mapping: `type="html"`/`xhtml"` always store as `Html`; `type="text"` is interpreted via the user's `default_post_format`.
4. The distinction is by **endpoint**, not by user-agent sniffing or per-request identity branching — the public feed is one endpoint, the AtomPub Collection another. This is robust as additional clients (e.g. the planned Emacs front-end) appear.

## Consequences

* Good: Feed readers always get renderable HTML; editing clients get lossless round-trips.
* Good: Incidental edits (e.g. a title-only change in MarsEdit) no longer risk converting an Org/Markdown post to HTML.
* Good: No fragile user-agent sniffing; behaviour is a stable property of the endpoint.
* Bad: Two serializers to maintain instead of one.
* Open: Whether MarsEdit round-trips `type="text"` losslessly depends on its per-blog editing mode and must be verified against the real app; if it converts to HTML on save regardless, the affected posts fall back to last-write-wins conversion for that client.
