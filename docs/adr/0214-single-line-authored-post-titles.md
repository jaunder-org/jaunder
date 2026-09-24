# ADR-0214: Authored Post Titles Are One Logical Line

- Status: accepted
- Date: 2026-09-24
- Issue: [#1662](https://github.com/jaunder-org/jaunder/issues/1662)

## Context

`PostTitle` is the canonical authored Post title. It currently trims surrounding
whitespace without restricting internal line breaks. The server's Org metadata
parser and the Emacs Protocol Client join repeated `#+TITLE:` values with a
newline, while AtomPub converts an invalid `<title>` to an absent title. These
paths admit or silently discard multiline source even though no product need for
an authored multiline title has been identified. A line break within a title
cannot be expressed losslessly by all supported authoring and edit paths.

A browser may visually wrap long text; that is not an authored line break. The
Rendered Title contract deliberately retains an inline `<br>` from HTML source
([ADR-0204](0204-persist-inline-rendered-post-titles.md)); deciding whether to
allow that markup is a separate presentation question. The owner confirms there
are no affected production titles to migrate.

## Decision

An authored Post title is optional; when present it is non-blank and contains no
line-separator character anywhere in the source, including before or after its
visible text. Reject CR, LF, vertical tab, form feed, Unicode next-line, line
separator, and paragraph separator. Preserve the existing trim of surrounding
non-line-breaking whitespace and preserve case and internal non-line-breaking
whitespace. Do not impose a length bound or strip, replace, or collapse a line
break to manufacture a valid title. An absent AtomPub title, or one containing
only non-line-breaking whitespace, still means no title; a line separator by
itself is invalid.

The shared domain-value boundary must reject a multiline `PostTitle`. Every
creation or update surface, including web and AtomPub writes and raw Org
metadata, rejects such input before persisting a Post. A Markdown or Org heading
that supplies a candidate title must reject rather than become untitled when
that candidate is invalid. An explicit multiline AtomPub `<title>` is a bad
request, not an absent field; repeated Org `#+TITLE:` headers that compose a
multiline title are invalid even when another structured title is supplied. The
Emacs Protocol Client rejects a multiline title before any network request or
Media upload. A failed write retains the prior Post and local authoring source.
This narrows [ADR-0155](0155-server-side-org-metadata-block.md)'s repeated-title
composition to the valid one-line case without changing its structured-presence
or atomic-acceptance rules.

This is a source-value invariant, not a ban on line wrapping or rendered `<br>`.
No production-data backfill or compatibility read path is introduced.

## Consequences

- A consistent validation error replaces silent truncation/absence or
  format-dependent acceptance. Repeated Org title headers and literal newlines
  in AtomPub titles that previously worked no longer do; ordinary titleless
  Posts and single-line source continue unchanged.
- Editor and protocol tests must cover rejection before side effects and
  unchanged data on failure across the supported authoring paths.
- The inline-only Rendered Title grammar from ADR-0204 remains unchanged: source
  such as `Hello<br>World` contains no literal line separator and may still
  render with an intentional break.
