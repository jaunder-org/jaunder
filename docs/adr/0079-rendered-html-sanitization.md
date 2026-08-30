# ADR-0079: `RenderedHtml` carries a common-owned sanitization invariant

- Status: accepted
- Date: 2026-07-29
- Issue: [#445](https://github.com/jaunder-org/jaunder/issues/445)

## Context

`common::render::render()` performed no HTML sanitization, yet its output is
emitted **unescaped** into the DOM via Leptos `inner_html`. Any authenticated
author could store markup that executed in every viewer's browser — stored XSS,
no special privileges needed to plant it. All three `PostFormat` variants were
affected: `Html` was a verbatim passthrough, and both `pulldown-cmark` and
`orgize` pass embedded raw HTML through untouched, so `<script>` in a Markdown
body reached the sink just as readily.

ADR for #398 introduced `RenderedHtml` as a provenance marker. Its static
spelling policy was later retired: compiler visibility now prevents downstream
raw construction, which is the ownership boundary the type needs.

Three constraints shaped the fix:

1. **`render()` is not the only inbound door.** #282 (RSS/Atom ingestion) is the
   first real inbound producer: remote entries arrive _already rendered_ from a
   stranger's server and never pass through `render()`. #6 adds remote channels.
   A guarantee attached to `render()` would have been structurally incomplete on
   arrival.
2. **All outside input enters host-side.** The client never receives foreign
   HTML — only our own server's output, round-tripping through a page we
   authored. So a host-only sanitizer enforces the invariant _completely_, not
   partially.
3. **`common` is target-agnostic and compiled into the wasm bundle.** An
   unconditional sanitizer dependency would ship `ammonia` + `html5ever` to
   every browser for code the client never calls.

## Decision

**The guarantee lives on the `RenderedHtml` type, not inside a renderer.** Its
invariant is "contains no active markup". `common::render::sanitize(raw)` is the
only public production API that establishes it by scrubbing untrusted input.

Supporting choices:

- **`ammonia` sits behind a `sanitize` feature on `common`** — off by default
  and never enabled for wasm. The public sanitizer exists only when that
  host-only feature is enabled.
- **One allowlist, defined once**, as a module-level `SANITIZER` builder:
  ammonia's audited default, widened only to keep `class` on `<pre>`/`<code>` so
  a fenced block retains its language marker, with an attribute filter narrowing
  the surviving values to `language-*` tokens.
- **The raw field is crate-private.** No public constructor, `From<String>`,
  `TryFrom`, `FromStr`, or blanket `Deserialize` admits already-trusted markup.
  Common-private SQLx decode and field-specific seed/revision DTO
  deserialization reconstruct Jaunder-owned values directly; neither
  re-sanitizes nor rewrites rendered bytes.
- **Exact fixtures are test-only.** `common::test_support::rendered_html` is
  compiled only for `cfg(test)` or the `test-support` feature.
- **The boundary is compiler-checked in an isolated dependency.** The
  `rendered-html-compiler-boundary` xtask step resolves a standalone production
  crate with default features disabled, proves ordinary dependency use, and
  proves raw construction and the fixture helper are inaccessible. It does not
  rely on workspace feature unification or source spelling.
- **No sanitizing on read, and no backfill.** Decode preserves persisted
  rendered HTML exactly.

## Consequences

- **`RenderedHtml` means what its name says.** The provenance framing of #398's
  ADR is amended: the type now guarantees safety, not merely origin. Code and
  docs that described it as "a provenance marker, not a safety guarantee" were
  corrected.
- **The invariant is enforced host-side only.** That is complete rather than
  partial, per context (2) — but it depends on that premise holding. If the
  client ever receives HTML from anywhere but our own server, this must be
  revisited.
- **A `Decode` blesses any text column decoded into `RenderedHtml`.** This risk
  is real and accepted on one ground: typing a column as `RenderedHtml` is a
  deliberate, reviewable act. There is no marker or spelling gate for this
  semantic correspondence; reviewers must ensure a decode reads the
  rendered-HTML column. The bridge does not sanitize on decode.
- **No inherited-trust public constructor survives.** Private reconstruction and
  test-only fixtures cover their respective representations without exposing a
  downstream raw-string door.
- **#282 must use `common::render::sanitize`** for ingested entry HTML.
- **`pulldown-cmark` and `orgize` remain in the wasm bundle.** With `render()`
  now host-only they are dead weight there and could become optional under the
  same feature — a genuine improvement, deliberately left out of a security fix.
- **A no-op cannot be substituted.** An earlier design injected an
  `HtmlSanitizer` trait to avoid the carve-out; it was rejected because a caller
  could supply a do-nothing implementation, which would be a hole in exactly the
  guarantee the type exists to provide.
