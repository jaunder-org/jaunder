# ADR-DRAFT: Pin Org special-string export to a focused v0.10 orgize fork

- Status: proposed
- Date: 2026-09-24
- Issue: [#1656](https://github.com/jaunder-org/jaunder/issues/1656)

## Context

Jaunder stores rendered Post bodies and Rendered Titles at write time. Its
`orgize` 0.10.0-alpha.10 exporter leaves Org's default `---`, `--`, and `...`
special strings literal, unlike Emacs Org HTML export. A Jaunder-only text
replacement would lose the distinction between prose and code, verbatim, and
link destinations. Moving away from upstream's `v0.10` branch would combine a
typography fix with unrelated parser changes.

## Decision

Fork `PoiScript/orgize` from `v0.10` at
`5f26c94dcec2a33b37b1c880ace053b29b5d021e` to `jaunder-org/orgize`. Change only
HTML export of prose text events to emit `&mdash;`, `&ndash;`, and `&hellip;`,
with focused exporter tests. Literal code and verbatim, link destinations, and
source text retain their original bytes. Neither the parser nor Org export
option support changes here.

Pin the fork by full commit revision in Cargo's `[patch.crates-io]`, the
separate tools workspace patch, and a flake input. Feed that exact Nix checkout
to both Crane git-source vendor steps; retain the existing `jaunder-org`
cargo-deny source allowance with this additional rationale. Review the fork's
full diff against its recorded upstream base before moving the revision. Host
rendering and title sanitization continue to own Jaunder's existing storage and
security boundaries.

## Consequences

Prose typography becomes Emacs-compatible for new writes; current Posts require
an explicit offline rebuild of their stored derivatives, not a source edit.
Historical Post Revisions retain their historical rendered bytes. Git and Nix
locks must move together when advancing the fork; a manifest-only revision
change is insufficient. Remove the patch, flake input, vendor overrides, and
deny rationale for this fork together when an audited upstream release supplies
the same exporter behavior.
