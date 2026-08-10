# ADR-0079: `RenderedHtml` carries a sanitization invariant, via two named doors

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

ADR for #398 introduced `RenderedHtml` as a **provenance** marker — "came out of
`render()`" — enforced structurally by a private field plus the
`rendered-html-from-trusted` static check. That stopped a raw `String` reaching
the unescaped sink, but it did not make the contents safe, because nothing
sanitized. The type's name promised more than it delivered.

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

**The guarantee lives on the `RenderedHtml` type, not inside `render()`.** Its
invariant is "contains no active markup", and two doors carry it, meaning
deliberately different things:

| Door                            | Meaning                                    | Callers                                                 |
| ------------------------------- | ------------------------------------------ | ------------------------------------------------------- |
| `RenderedHtml::sanitize(raw)`   | **establishes** the invariant by scrubbing | `render()`, feed ingestion (#282), any inbound producer |
| `RenderedHtml::from_trusted(s)` | **inherits** it from an earlier `sanitize` | the seed-DTO wire rebuild — one production call site    |

Any future inbound path has exactly one usable public door. Reaching for
`from_trusted` instead fails the `rendered-html-from-trusted` gate, which is
extended to name `sanitize` as the correct alternative. Choosing the wrong door
breaks the build rather than silently reopening the hole.

Supporting choices:

- **`ammonia` sits behind a `sanitize` feature on `common`** — off by default,
  never enabled for wasm, enabled by `storage`. This is `common`'s second
  deliberate carve-out after the `sqlx` bridge. It is a _feature_ rather than a
  `#[cfg(not(target_arch = "wasm32"))]` carve-out, which is the pattern ADR-0058
  objects to.
- **`render()` is gated on the same feature.** With it off the function does not
  _exist_, rather than existing and silently not sanitizing — absence, never a
  weaker guarantee. Its private helpers and the `PostBody` import are gated with
  it.
- **One allowlist, defined once**, as a module-level `SANITIZER` builder:
  ammonia's audited default, widened only to keep `class` on `<pre>`/`<code>` so
  a fenced block retains its language marker, with an attribute filter narrowing
  the surviving values to `language-*` tokens.
- **`sqlx::Decode` constructs the private field directly** — neither door — so
  the `rendered_html` column decodes into the newtype like every other domain
  column, and the DB read no longer rebuilds through `from_trusted`.
- **No sanitizing on read, and no backfill.** No deployed instance holds data,
  so there are no pre-fix rows to heal.

## Consequences

- **`RenderedHtml` means what its name says.** The provenance framing of #398's
  ADR is amended: the type now guarantees safety, not merely origin. Code and
  docs that described it as "a provenance marker, not a safety guarantee" were
  corrected.
- **The invariant is enforced host-side only.** That is complete rather than
  partial, per context (2) — but it depends on that premise holding. If the
  client ever receives HTML from anywhere but our own server, this must be
  revisited.
- **A `Decode` blesses any text column decoded into `RenderedHtml`.** This was
  the original argument against having one. Accepted on a single ground: typing
  a column as `RenderedHtml` is a deliberate, reviewable act. The
  `rendered-html-from-trusted` gate does **not** cover it — that gate's
  population is `from_trusted` on **this** type, the definition and every use,
  with the qualifier resolved since
  [#790](https://github.com/jaunder-org/jaunder/issues/790), whereas a `FromRow`
  field typed over the wrong column names no door at all. This is the one
  residual risk in the design that nothing mechanical enforces; widening the
  gate to flag `RenderedHtml`-typed row fields is filed as
  [#701](https://github.com/jaunder-org/jaunder/issues/701).
- **`from_trusted` survives, narrowed to inherited trust.** It is down to one
  production call site, and the gate keeps it greppable and confined.
- **#282 must use `sanitize`** for ingested entry HTML. Recorded as an addendum
  on that issue; the gate will fail the build if it reaches for the wrong door.
- **`pulldown-cmark` and `orgize` remain in the wasm bundle.** With `render()`
  now host-only they are dead weight there and could become optional under the
  same feature — a genuine improvement, deliberately left out of a security fix.
- **A no-op cannot be substituted.** An earlier design injected an
  `HtmlSanitizer` trait to avoid the carve-out; it was rejected because a caller
  could supply a do-nothing implementation, which would be a hole in exactly the
  guarantee the type exists to provide.
