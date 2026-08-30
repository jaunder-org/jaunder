# Issue #1148 — Centralize trusted HTML injection

## Outcome

Jaunder's ordinary Leptos HTML injection path accepts trusted `Markup` rather
than an arbitrary `String`. One crate-local adapter owns the audited raw HTML
sink while preserving the exact host elements emitted at the five current web
call sites.

## Load-bearing decisions

- `web::html::Markup` remains the web crate's trusted rendering currency and is
  the adapter's only markup input. The adapter does not accept `String` or add a
  parallel `RenderedHtml` entry point.
- `RenderedHtml` continues to enter web rendering only through the existing
  reviewed `Markup::from_rendered_html` conversion.
- The adapter accepts an already-built, childless typed Leptos element, applies
  the raw HTML attribute internally, and returns the resulting view. This must
  preserve the caller's element tag and accumulated attributes without adding a
  wrapper node.
- The five existing ordinary sinks in the home masthead, post views, permalink
  first paint, and anonymous sidebar move through the adapter. Their callers no
  longer erase `Markup` to `String` or name a raw HTML sink.
- The `html-sink` gate retains its bypass-detection role. Its approved
  production census contains the adapter's audited sink rather than one marker
  at every ordinary caller.
- Sanitization ownership and policy do not move: host rendering still produces
  `RenderedHtml`, active markup remains stripped there, and the web adapter only
  injects already-trusted `Markup`.
- This local, reversible boundary introduces no new domain term or architectural
  decision record.

## Acceptance

- The five current ordinary web sink sites render through the typed adapter and
  preserve their existing element tags, attributes, layout, and projector/CSR
  coincidence behavior.
- No migrated caller converts trusted markup into an arbitrary `String` for a
  Leptos `inner_html` or `set_inner_html` attribute.
- Unmarked direct production use of a raw HTML sink outside the adapter fails
  the `html-sink` gate; the adapter's single sink remains explicitly audited.
- Focused behavioral proof shows trusted rendered markup appears unescaped
  through the adapter without changing the existing sanitizer contract.
- Existing sanitizer coverage continues to prove that script elements, event
  handlers, and active URLs are stripped before markup reaches the web layer.

## Boundaries

- Do not change sanitization rules, trusted-markup construction doors, or the
  storage representation of `RenderedHtml`.
- Do not add wrapper DOM, change projector output, or rebuild shared pure
  renderers as reactive Leptos markup.
- Do not make `Markup` public outside the web crate or add a caller-facing raw
  string overload.
- Do not generalize Leptos' foreign raw HTML API or broaden this work into other
  type-safety milestone issues.
