# ADR-DRAFT: RenderedHtml proof without a standalone compiler gate

- Status: proposed
- Date: 2026-10-08
- Issue: [#1705](https://github.com/jaunder-org/jaunder/issues/1705)

## Context

[ADR-0079](../0079-rendered-html-sanitization.md) chose an isolated downstream
compiler check for `RenderedHtml` constructor privacy and fixture-helper
confinement. Its temporary crate, lockfile and patch-table preparation, cold
Cargo home, and diagnostic matching add a separate dependency-resolution surface
to every broad host gate. A long-lived shell can retain a Cargo-source home that
does not match the checkout, causing an otherwise unrelated gate failure.

Controlled experiments established that existing positive and compile-fail
rustdoc examples already detect exposing the tuple constructor. The standalone
check additionally detects exposing the fixture module in a bare-feature build,
but remains green when a normal production dependency enables `test-support`.
Its synthetic manifest explicitly disables default features; it does not prove
fixture absence in the dependency graphs that ship.

The opaque type prevents accidental raw-string use at HTML sinks. It is not an
unforgeable sanitization certificate: trusted DTO deserialization and SQLx
decode intentionally reconstruct Jaunder-owned bytes without sanitization.
Correctness also depends on the provenance of those bytes and sanitization of
untrusted input. Compiler fixtures do not establish that provenance.

## Decision

Remove the standalone `rendered-html-compiler-boundary` step and its
implementation without adding a replacement gate. This narrowly supersedes
ADR-0079's isolated compiler-check requirement; its type, sanitization,
fixture-gating and trusted reconstruction decisions remain in force.

Retain the opaque `RenderedHtml` type, sanitizer and typed assembly, existing
positive and compile-fail doctests, actual publishing-path sanitization tests,
and existing HTML sink checks. The publishing-path regression exercises both
SQLite and PostgreSQL and checks returned and persisted rendered HTML.

Fixture confinement remains an existing feature-declaration and review
responsibility. Do not introduce production-feature graph enforcement, a reduced
standalone compiler fixture, or a source-spelling scanner merely to compensate
for this deletion. Such machinery needs its own concrete justification.

## Consequences

The host gate has no standalone HTML fixture compilation or associated cold
Cargo-home dependency. This removes the observed failing consumer; it does not
repair Cargo-source selection generally or establish which launch layer retained
the stale setting.

Existing doctests remain compiler-backed constructor proofs, and real publishing
tests remain the sanitization proofs. The distinct bare-feature fixture-helper
absence proof is deliberately relinquished. Accidental production activation of
`test-support` is review-dependent; no equivalent automated claim is made.

Sanitizer policy, rendering output, trusted reconstruction, test fixture APIs,
offline policy, pinned tooling and test budgets are unchanged. In particular,
wrong-column SQLx decoding and trusted DTO input provenance still require review
([ADR-0123](../0123-rendered-html-storage-decode.md)).
