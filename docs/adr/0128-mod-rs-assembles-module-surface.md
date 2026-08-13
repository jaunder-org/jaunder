# ADR-0128: `mod.rs` assembles the module surface and holds no items

- Status: accepted
- Date: 2026-08-12
- Issue: [#942](https://github.com/jaunder-org/jaunder/issues/942)

## Context

A `mod.rs` answers one question: what does this module contain and export? It is
the first file a reader opens to orient in an unfamiliar directory, and for an
agent it is the cheapest possible map of a subtree.

That only works if the answer is the whole file. In 17 of this repository's 53
`mod.rs` files it was not: implementation had accumulated alongside the wiring,
so the surface was buried in the body and the file had no single reason to
change. `server/src/atompub/mod.rs` carried a router, an axum metrics
middleware, three handler guards, an error enum, ten `From` impls and a test
module on top of the five `pub mod` lines it is named for.
`server/tests/storage/mod.rs` reached 6090 lines with no wiring in it at all.

[ADR-0070](0070-web-vertical-wasm-only-component-files.md) already stated the
rule — "module wiring only", no items of its own — but bound it to `web/`
**verticals**. The two `web/` directories that held items, `web/src/error/` and
`web/src/reactive/`, are the non-vertical support directories, so they sat
outside that scope and broke nothing. That is the shape of the gap: the rule was
right and its scope was an accident of the problem it was written for. Fifteen
further directories elsewhere in the workspace had never been told.

The forces pulling the other way are real and shaped the decision. Rust permits
items in a `mod.rs`, so nothing in the language objects. Relocating code churns
paths, and churn in a module surface is exactly the churn most likely to break
distant call sites. And a rule that is easy to state syntactically — "`mod.rs`
contains no items" — is not obviously one a machine should enforce, because the
interesting question is not whether an item is present but whether it earns a
file of its own.

## Decision

**A `mod.rs` may contain only `mod`/`pub mod` declarations, `use`/`pub use`/
`pub(crate) use` re-exports, `//!` module documentation, and attributes** —
inner attributes, and outer attributes on those `mod` and `use` items.

Everything else is excluded: `fn`, `struct`, `enum`, `trait`, `impl`, `const`,
`static`, `type` aliases, `macro_rules!`, and inline
`#[cfg(test)] mod tests { … }` bodies. Code lives in a sibling file that
`mod.rs` declares and re-exports.

**The rule is workspace-wide with no exemptions** — production crates, test
trees, `xtask/` and `tools/` alike. An exemption list is the part that rots, and
the two largest offenders were both under `server/tests/`, so exempting tests
would have exempted the problem. ADR-0070's vertical-scoped statement stands
unchanged and is not superseded; this decision widens the same rule to the rest
of the workspace.

**Relocating code does not change any public path.** The `mod.rs` re-exports
what it moved out, so existing call sites are untouched. Re-exports name their
items explicitly rather than globbing: `pub use thing::{A, B};` states the
surface, `pub use thing::*;` states nothing, and stating the surface is the
entire purpose of the file.

**Enforcement is by review, not by a gate.** This is the deliberate half of the
decision. A syntactic check would be easy to write and would be wrong in both
directions: it would fire on a two-line `pub type` alias that genuinely belongs
with the wiring, and it would pass a sibling file named `inner.rs` holding
everything the `mod.rs` used to hold, which satisfies the letter and defeats the
point. Whether a module's surface is honestly stated is a judgement about
cohesion, and this repository already has the machinery for judgement — the
deliverable-boundary and whole-branch reviews — whereas its gate ladder is long
enough that adding a check with a known false-positive rate would spend more
attention than the rule saves.

## Consequences

- The rule is stated for humans in `CONTRIBUTING.md` and carried into the review
  path, so every deliverable-boundary and whole-branch review checks it. Because
  review is the enforcement, the rule must be _legible_ — hence the
  explicit-re-export requirement, which makes a `mod.rs` diff readable at a
  glance.
- No new xtask check. If the rule is found to erode in practice, that is the
  evidence needed to revisit this and gate it; absent that evidence, adding a
  gate would be speculative.
- Moving items out of a `mod.rs` changes the meaning of `pub(super)` — at
  `mod.rs` it names the parent of the directory, in a sibling it names the
  directory itself. Relocations must widen such items to `pub(crate)` to
  preserve their existing reach. `storage/src/sqlite`'s database-open functions
  were the live instance.
- Two `mod.rs` files exist _because_ of `clippy::module_inception`:
  [ADR-0067](0067-server-integration-tests-one-binary.md) promoted
  `server/tests/{projector,storage}/<same-name>.rs` into `mod.rs` for that
  reason. New siblings in those directories may not retake the old names.
- Gated wiring stays put by construction: the `target-arch-placement` check
  admits a `target_arch` cfg only on a `mod` or `use` item in a `mod.rs` or
  `lib.rs`, so those lines are assembly under this rule and never migrate into a
  leaf file. This decision and that check agree rather than compete.
- A `mod.rs` that ends up as nothing but `mod` and `pub use` lines is the
  intended outcome, not a sign the split went too far.
