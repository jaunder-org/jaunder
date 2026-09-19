# Preserve Org publication migrations

## Outcome

A `jaunder-reconcile` push can migrate an existing Org publication without
corrupting source or wire bytes, flooding Emacs with internal warnings, or
leaving buffers open. Imported Posts retain their paragraph structure and legacy
category/tag metadata, and canonical filename changes are reported clearly
enough not to look like deletion.

## Load-bearing decisions

- AtomPub request bodies are transmitted byte-for-byte. Text source and media
  bytes must not pass through curl's form-style `--data` normalization.
- Inventory reads remain side-effect-free: they parse temporary-buffer contents
  without activating Org mode or user major-mode hooks.
- Reconcile may continue the established canonical `<slug>.org` rename contract
  from ADR-0047. A successful result must expose the old and new paths when they
  differ so the operation cannot masquerade as deletion.
- Reconcile owns buffers it opens solely for a push and kills those buffers
  after each terminal result. It never kills a buffer that was already visiting
  the source file.
- The three create saves remain durability checkpoints: durable create intent
  before POST, identity plus validator after the response, then intent cleanup.
  This is the non-destructive recovery-metadata exception to ADR-0047
  established by the proposed
  [durable AtomPub create intent decision](../../adr/drafts/durable-atompub-create-intent.md):
  authored content and identity remain untouched on a pre-response failure,
  while the same key remains available for an exact retry. Their ordinary
  `Saving file…` chatter is suppressed; durability is not traded for fewer
  writes.
- Removing temporary bookkeeping properties removes their complete lines,
  including line endings, so the leading metadata block remains contiguous.
- The Org adapter maps legacy `#+CATEGORY` and repeated `#+TAGS` values into
  Atom categories alongside `#+KEYWORDS`, preserving source order and omitting
  blank terms. Those migration headers remain client-side metadata rather than
  body content.
- Publication-specific root-relative image and obsolete `.html` link corrections
  are data repair in the source publication, not a generic Jaunder rewrite
  policy: a leading slash is ambiguous outside that known legacy site.
- No server storage, public URL, authentication, or deletion semantics change.

## Acceptance

- A deterministic inventory command over many Org files emits zero
  `Making change-major-mode-hook buffer-local while locally let-bound!` messages
  and executes no user mode hooks.
- A reconcile batch opens no lasting source buffers of its own; a source buffer
  open before the batch remains live and follows any canonical rename.
- A normal create retains all durability checkpoints while emitting only batch
  progress and the final publication/rename message, not repeated save chatter.
- Create-intent cleanup leaves exactly one separator blank line between the
  contiguous header block and body.
- A multiline Org body sent through the real transport reaches AtomPub with
  every interior newline intact; binary request bodies are likewise
  byte-preserving.
- A legacy Org fixture containing `CATEGORY`, repeated `TAGS`, and `KEYWORDS`
  produces the complete ordered category list without leaking those headers into
  the body.
- Reconcile reports a canonical rename using both the original and destination
  paths.
- The repaired `radios-appear` source tree has no accidental content loss and
  contiguous Jaunder metadata. Its reviewed migration inventory rewrites exactly
  the 39 root-relative raster-image targets whose sibling files exist to
  `./<filename>`, and exactly the 16 dated legacy intra-site targets whose slug
  resolves to a local Org file to `/~operator/YYYY/MM/DD/<slug>` using that
  target file's `#+DATE`; all other root-relative targets remain unchanged.
- On `/~operator/2025/02/10/22-years-wtf`, the Post body renders as four
  paragraphs, preserves the `+years+` strike-through and the source's literal
  `~ 2013` text, and exposes `Meta`, `anniversary`, and `nattering` as
  categories/tags. On each of the three `2014-01-0{1,2,3}` Posts, the body
  renders an image backed by uploaded Media. Theme typography, spacing,
  navigation, and Hugo-only shortcodes are intentionally outside the comparison.

## Boundaries

- Do not remove, combine, or defer the create durability checkpoints.
- Do not add a general root-relative link rewriter or Hugo shortcode
  interpreter.
- Do not promise pixel-identical rendering with tendentious.org.
- Do not rewrite unrelated author prose or guess unresolved link destinations.
- Do not merge the resulting PR without explicit approval.
