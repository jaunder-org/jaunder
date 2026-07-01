# ADR-0024: Server-Side Org Canonicalization and the Local-vs-Served Representation Principle

- Status: accepted
- Deciders: mdorman, Claude
- Date: 2026-06-26

## Context and Problem Statement

Posts can be authored three ways against one blog: the web/mobile compose form,
the Emacs front-end (raw Org with `#+`-header metadata), and third-party
clients. The original Emacs spec said metadata is mapped **client-side only**
and "no server-side org-header parsing is added." That was already untrue — the
server parses `#+TITLE:` today (`extract_org_title`,
`common/src/render.rs:149-194`) but **discards the stripped body**
(`render.rs:96`), so the title line stays in storage and `orgize` renders it.
The real question: how do web-authored and Emacs-authored Org posts reach
**one** stored representation, without metadata living in two places (Atom
elements _and_ embedded header lines) where it can diverge?

## Decision Drivers

- One canonical stored body regardless of authoring client (web/mobile/emacs).
- No metadata duplication between structured fields and body text.
- Preserve author content faithfully, including Org constructs the mapping
  doesn't know.
- Don't break local Org authoring (inline image preview must keep working).
- Round-trip stability — reconcile must not see a post as perpetually
  "diverged."

## Decision Outcome

**The local representation differs from the served form; the client maps between
them. Neither side may corrupt the other.** Concretely:

**1. The server normalizes every ingested Org body to one canonical,
metadata-free form** at the existing `extract_org_title`/`derive_post_metadata`
seam — by _keeping_ the stripped body it currently discards. Only headers the
server stores structurally are stripped (today `#+TITLE:`). **Unrecognized
`#+FOO:` header lines stay in the body verbatim** and round-trip. No
`#+KEYWORDS:`/`#+DESCRIPTION:` parsing is added — those arrive structurally on
both paths (web form fields; Atom elements from Emacs).

**2. Clients synthesize their own header block on the way out** from clean Atom;
the server never emits client-specific (`JAUNDER_*`) markup. `atom:content` is
the **body only**, not the whole buffer.

**3. Media links follow the same principle.** The on-disk Org file always keeps
local, previewable links; publish substitutes the content-addressed media URL
(`/media/{sha256}/{filename}`) **only in the body sent to the server** and never
rewrites the buffer. Because the mapping is content-derived, divergence
detection normalizes a local link to its sha-URL before comparing — no stored
map, no false divergence.

**4. The canonical form must be byte-deterministic** (fixed header order/format
on synthesis) so the strip-and-resynthesize round-trip is stable for reconcile.

This **reverses** the spec's original "client-side only / no server org parsing"
decision: org-header normalization is a server responsibility, applied uniformly
below both ingestion paths.

## Consequences

- Good: web, mobile, and Emacs converge on one canonical post; no metadata
  duplication; authored body content (incl. unknown keywords) round-trips; local
  preview is intact.
- Good: a single normalization point serves every client and every future
  client.
- Bad: stripping the `#+TITLE:` line changes web rendering (the inline org-title
  no longer renders; the title column shows) — needs a regression test and a
  backfill decision for legacy stored bodies.
- Bad: `JAUNDER_DATE_TZ` has no Atom home and is recaptured (not preserved) on a
  fresh pull — acceptable, it is a local convenience.
- Open: teaching the server the _full_ Org header block (so raw-Org web
  authoring sets tags/summary) is deferred (would duplicate the web form fields
  → precedence rules); tracked as follow-on #77 ("β").
