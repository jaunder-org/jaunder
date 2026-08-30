# Issue #1147 — Narrow `RenderedHtml` reconstruction

## Outcome

`RenderedHtml` can be minted in production only by the common-owned sanitizer or
by reconstruction paths confined within its owning crate. Application crates can
no longer turn a raw string into trusted markup through a generic constructor.

The change preserves rendered output, persisted-post decoding, server-produced
seed DTO round-trips, and exact test fixtures while replacing source-scan policy
with compiler-enforced ownership.

## Load-bearing decisions

- `common::render` exclusively owns the `RenderedHtml` safety invariant and the
  operation that establishes it.
- `common::render::sanitize` is the only public production inbound API. It
  scrubs outside, authored, and ingested HTML before returning `RenderedHtml`.
- Sanitization is an optional host-only capability. The CSR/wasm dependency
  graph must not acquire `ammonia` or another sanitizer dependency.
- The existing host sanitizer is removed rather than retained as an alias or
  re-export; every caller moves to the common-owned API.
- The generic production `RenderedHtml::from_trusted` constructor is removed. No
  public replacement may accept an already-trusted raw `String` or `&str`.
- SQLx decoding remains a trusted reconstruction path inside `common`. Persisted
  rendered HTML is not re-sanitized or silently rewritten while decoding.
- Seed DTO deserialization remains a field-specific, common-private trusted
  reconstruction path. `RenderedHtml` does not gain blanket `Deserialize`.
- Tests that need byte-exact rendered markup use a raw fixture constructor
  available only through `common::test_support`, under `cfg(test)` or the
  existing `test-support` feature. Production builds expose no equivalent door.
- Sanitizer behavior tests move with sanitizer ownership. Host-owned media
  extraction remains in `host`, including coverage that every
  sanitizer-permitted attribute is classified as media-bearing or inert.
- The `rendered-html-from-trusted` spelling gate and its allowance markers are
  deleted. Compile-time privacy and feature boundaries own the invariant
  instead.
- `common/src/render.rs`, ADR-0079, ADR-0123, and `docs/ARCHITECTURE.md` are
  amended to describe the new ownership and reconstruction model. This refines
  existing decisions and does not require a new ADR or glossary term.

## Acceptance

- A normal production crate cannot construct `RenderedHtml` from `String`,
  `&str`, tuple syntax, a generic trust constructor, or the test fixture API.
- The public sanitizer removes active markup and preserves the currently allowed
  safe markup, including permitted `language-*` classes on code blocks.
- Markdown, Org, and HTML post rendering still pass through the same sanitizer
  policy and produce the same observable rendered output.
- Existing posts and revisions decode from both supported storage backends
  without re-sanitization, content changes, or new fallible behavior.
- Server-produced seed DTOs retain their current serialization and
  deserialization behavior without adding blanket `RenderedHtml: Deserialize`.
- Downstream tests can create exact `RenderedHtml` fixtures through
  `common::test_support`; a production compile-fail check proves the helper and
  raw construction paths are unavailable there.
- The CSR/wasm build remains free of the sanitizer dependency.
- Host media-reference extraction retains its sanitizer-surface classification
  regression coverage.
- No production use, definition, marker, or xtask registration for
  `rendered-html-from-trusted` remains.
- The relevant focused tests and the repository validation gate pass.

## Boundaries

- Do not change the sanitizer allowlist, URL policy, or rendered HTML bytes.
- Do not change post rendering semantics, media-reference extraction semantics,
  storage schemas, wire formats, or persisted data.
- Do not redesign `SqlxBridge` or the general newtype derive system.
- Do not move media extraction out of `host`.
- Do not add runtime provenance tags, capabilities, re-sanitization,
  compatibility constructors, deprecated aliases, or new trust-marker types.
- Do not broaden this work into the typed SQLx bind seam tracked by #1146.
