# Issue #445 — Sanitize rendered post HTML (stored XSS)

**Status:** approved **Issue:**
[#445](https://github.com/jaunder-org/jaunder/issues/445) (P1, milestone
"Correctness & data integrity") **Depends on:** #398 (closed) — the
`RenderedHtml` newtype and its xtask gate **Coordinates with:** #282 (in flight)
— the first inbound producer

## Goal

Protect against malicious HTML **from outside**, enforced through the type
system so it cannot be missed by accident, with as little boilerplate as
possible.

Derived requirements:

- **MUST** sanitize raw HTML arriving as input from outside.
- **SHOULD** sanitize HTML generated from outside input.
- **SHOULD** support effortless reading from the DB.

## Problem

`common::render::render()` performs no sanitization, yet its output is emitted
unescaped via Leptos `inner_html`. Any authenticated author can store markup
that executes in every viewer's browser — stored XSS, no special privileges
needed.

- `PostFormat::Html` is a raw passthrough (`common/src/render.rs:230`).
- Markdown and Org run `pulldown_cmark::html::push_html` and
  `orgize::Org::parse(..).to_html()`, which pass embedded raw HTML through
  untouched.
- `storage/src/posts.rs` calls the column "Sanitized HTML rendering".
  Aspirational.

#398 made `RenderedHtml` a **provenance** marker ("came from `render()`"). This
issue makes it an **invariant** ("contains no active markup").

## Findings that shaped the design

1. **`render()` is not the only inbound door — and the second is being built
   now.** #282 is "the first real inbound producer": it ingests remote RSS/Atom
   entries, whose HTML arrives already-rendered from a stranger's server and
   never passes through `render()`. #6 (remote channels, same milestone) adds
   another. **A fix shaped around `render()` is structurally incomplete.** The
   guarantee must live on the type, not in one function.
2. **All outside input enters host-side.** The client never receives foreign
   HTML — only our own server's output round-tripping through a page we
   authored. So a host-only sanitizer enforces the invariant _completely_, not
   partially.
3. **`render()` is server-only.** Callers are `storage::post_service`
   (create/update/publish) and test fixtures. Nothing in `web/` or `client/`.
4. **AtomPub is not a separate door.** `server/src/atompub/posts.rs:379` calls
   `storage::perform_post_creation` with the raw body, funnelling through
   `render()`.
5. **The client rebuilds `RenderedHtml` in wasm.** `csr/src/lib.rs:32`
   deserializes `PageSeed` via `deserialize_rendered_html`.
6. **There is no production data anywhere.** No deployed instance holds posts.
7. **The #398 gate is already the enforcement mechanism.**
   `rendered_html_from_trusted_check` is a syn-AST scan pinning every non-test
   `from_trusted` to an allowlist of enclosing functions, with `#[cfg(test)]`
   exemption, nested-fn-shadow detection, and negative tests proving it bites.
8. **`ammonia` is not vendored** — a new dependency, so the first gate run pays
   a full cold Nix vendor rebuild.

## Decisions

### D1 — Two doors on the type, meaning different things

The guarantee lives on `RenderedHtml`, not inside `render()`:

| Door                                      | Meaning                                                   | Callers                                                        |
| ----------------------------------------- | --------------------------------------------------------- | -------------------------------------------------------------- |
| `RenderedHtml::sanitize(raw)` — host-only | **establishes** safety                                    | `render()`, feed ingestion (#282), any future inbound producer |
| `RenderedHtml::from_trusted(s)`           | **inherits** safety — our own prior output round-tripping | client seed deserialize only                                   |

Any future inbound path has exactly one usable public door: `sanitize`. Reaching
for `from_trusted` instead fails the #398 gate. That is the "can't accidentally
miss it" mechanism, and it already exists.

### D2 — `ammonia` in `common` behind a `sanitize` feature

A second deliberate carve-out, alongside the existing `sqlx` one, and its own
feature rather than riding `sqlx` (rendering is not persistence; conflating them
would make `common`'s feature list misleading). Off by default, never enabled
for wasm.

`render()` moves behind the same feature: with it off `render()` does not
_exist_, rather than existing and silently not sanitizing. Absence, not
weakening.

_Rejected: injecting an `HtmlSanitizer` trait implemented in `host`._ Its only
purpose was avoiding this carve-out — but with the sanitizer gated host-only
anyway, injection buys nothing except the ability to hand in a no-op, which is a
hole in the type guarantee. The goal of type-enforced safety outranks carve-out
minimalism.

_Rejected: moving `render()` to `host`/`storage`._ Neither is necessary once the
guarantee lives on the type, and both split the doors across crates.

### D3 — `Decode` uses the private constructor; no sanitizing on read

`RenderedHtml` gains a `sqlx::Decode` impl under the existing `sqlx` feature.
Because it lives in `common/src/render.rs` it constructs the private tuple field
directly — it needs neither `sanitize` nor `from_trusted`.
`PostRow.rendered_html` becomes `RenderedHtml` (joining every other domain
column, which already decode into their newtypes via #438/#572) and
`build_post_record` drops its rebuild. That satisfies "effortless reading from
the DB".

Reads do **not** re-sanitize. Safety comes from every write door sanitizing,
enforced by the gate — write-side enforcement instead of read-side
recomputation. Sanitizing on read was considered and rejected: with no
production data (finding 6) it guards only against a write path that forgot,
which is precisely what the gate catches, at the price of an html5ever parse per
post per read forever.

**Accepted residual risk:** a `Decode` blesses _any_ text column decoded into
`RenderedHtml`, not just `rendered_html` — the objection recorded at
`common/src/render.rs:185-191`. Decoding a column into this type is a deliberate
typing act, and the same class of mistake the gate exists to catch. The comment
must be rewritten to record that the trade was reconsidered and why, not
deleted.

### D4 — Gate extended to point at the new door

`ALLOWED_FNS` shrinks: `build_post_record` no longer calls `from_trusted` (D3),
so the allowlist reduces to `deserialize_rendered_html` — one door. The gate's
recovery message must name `RenderedHtml::sanitize` as the correct alternative
for inbound data. A new negative test asserts an inbound-shaped function
reaching for `from_trusted` is still flagged.

### D5 — `PostFormat::Html` survives, sanitized

The variant stays; its output goes through the same allowlist, so the
passthrough stops being raw. Removing it would touch the DB enum, wire
serialization, and AtomPub mapping.

### D6 — Ammonia's default allowlist, verified against our renderers

Start from ammonia's audited baseline rather than hand-rolling. Verify it strips
nothing `pulldown-cmark` or `orgize` legitimately emit — code blocks, tables,
heading anchors, links — and widen only where a test demonstrates a real gap.
One allowlist, defined once, shared by every caller of `sanitize`.

### D7 — No backfill

Nothing exists to backfill (finding 6). Not a deferral; there is nothing to
remediate.

### D8 — Record an ADR

Numberless draft in `docs/adr/drafts/`, numbered at ship by
`cargo xtask adr promote`. Records the sanitization policy, the two-door model
and what each door means, and the shift in `RenderedHtml` from provenance to
invariant — which amends #398's framing.

### D9 — Coordinate with #282 (done)

#282's ingestion path becomes a consumer of `RenderedHtml::sanitize`: remote
feed entries carry already-rendered third-party HTML that never passes through
`render()`. That worktree is dormant, so coordination is by issue addendum
rather than by talking to an active owner — recorded at
[#282 (comment)](https://github.com/jaunder-org/jaunder/issues/282#issuecomment-5120081185),
which names the correct door and notes that the gate will fail the build if
ingestion reaches for `from_trusted` instead.

This issue does not implement ingestion sanitization and takes no dependency on
#282.

## Acceptance criteria

- **AC1 — all three formats neutralized.** A body containing `<script>`,
  `onerror=`, and a `javascript:` URL is sanitized in `Markdown`, `Org`, and
  `Html`.
- **AC2 — regression test end-to-end.** A malicious body is driven through the
  real path and the emitted HTML asserted free of dangerous markup.
- **AC3 — one establishing door.** `RenderedHtml::sanitize` is the only public
  way to build the type from outside data; the field stays private.
- **AC4 — the gate bites.** An inbound-shaped function using `from_trusted`
  fails `cargo xtask check`, proven by a new negative test. `ALLOWED_FNS`
  contains exactly the round-trip doors.
- **AC5 — effortless DB reads.** `PostRow.rendered_html` is `RenderedHtml`;
  `build_post_record` no longer calls `from_trusted`.
- **AC6 — legitimate markup survives.** Code blocks, tables, headings, emphasis,
  and safe links from both renderers pass through intact.
- **AC7 — wasm build carries no sanitizer.** `ammonia` is absent from the wasm
  dependency graph; the CSR build compiles unchanged.
- **AC8 — stale doc comments corrected.** `from_trusted`'s doc, the `Decode`
  rationale at `common/src/render.rs:185-191`, and the "Sanitized HTML
  rendering" comments in `storage/src/posts.rs` all describe reality.
- **AC9 — ADR drafted**, numberless, in `docs/adr/drafts/`.
- **AC10 — gate green.** `cargo xtask validate` passes.

## Out of scope

- **Sanitizing #282's ingestion path** — that issue's work; this one provides
  the door (D9).
- **Backfilling existing rows** — nothing exists to backfill (D7).
- **Sanitizing on read** (D3). Revisit only if an instance ever accumulates data
  written by a pre-fix build.
- **Removing `pulldown-cmark`/`orgize` from the wasm bundle.** With `render()`
  behind a host-only feature they become dead weight in the CSR build and could
  be made optional under the same feature. A genuine win, but bundle-size work
  rather than security work — file as a follow-up.
- **Other HTML-bearing surfaces** (feed output, AtomPub output) beyond what
  flows through the two doors.

## Risks

- **Cold rebuild.** New `ammonia` dependency forces a full Nix vendor rebuild on
  the first gate run.
- **Over-aggressive stripping.** If the default allowlist drops markup our
  renderers emit, posts render degraded. AC6 is the guard.
- **Feature-gated `render()` churn.** Every caller and test fixture must build
  with the `sanitize` feature enabled. Confined to `storage` and `server`, but
  it touches `storage::test_support` broadly.
- **`Decode` blessing.** See D3's accepted residual risk.
