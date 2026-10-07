# Native Orgize ownership correction (#1697)

This supplements the [approved verse/quote spec](2026-10-07-issue-1697-org-verse-quote-spec.md).
The user clarified before merge that the rendering correction belongs in the
pinned `jaunder-org/orgize` fork, not a Jaunder-owned syntax-tree adapter.

## Corrected implementation contract

- Implement verse parsing and layout export natively in the existing focused
  `v0.10` fork. Preserve lossless authored source, document-owned footnote
  identity, cross-line inline objects, literal code/verbatim, and the existing
  multi-paragraph quote semantics.
- Preserve comma-escaped verse lines, mixed-case verse/quote begin/end
  delimiters, and inert shortcode-looking text inside recursive inline objects.
  Unrelated block names retain their previous exact closing-name matching.
  Exercise both the default grammar and the optional Org-fc grammar directly
  in fork tests.
- Remove `host/src/org_verse.rs` and traverse the original parsed document
  directly through the existing Jaunder exporter and sanitizer.
- Advance the root/tools Cargo patches and locks and the Nix input/lock together
  to the reviewed immutable revision
  `17311ba02d4317571fab04a752a479e09bef2397`.
- Retain paired migration `0050`, backend-parametric rebuild preservation tests,
  and the focused Local/permalink browser proof. The original storage,
  sanitization, historical-revision, and no-production-access boundaries remain
  unchanged.

The fork revision is published on `issue-1697-verse-export`; publishing the
revision does not merge Jaunder PR #1700. Jaunder still requires the normal
review, hook-backed gates, required CI checks, and explicit human merge approval.
