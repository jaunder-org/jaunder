# #1864 Rust-format implementation outline

**Status:** Approved outline and execution. The human approved this outline
under Manager request `9b7971b8-088c-4d0a-bbf4-8dd60bf14d40` / issue
`issue1864-rust-format-outline-v1`, with the exact answer “Approve outline and
begin.” This is a separate-checkout execution; merge remains separately gated.
The governing scoped-format spec is approved under request
`1b68e607-ca9a-460b-b005-179532a44a9d` / issue `issue1864-rust-format-v3`, with
the exact answer “Approve scoped-format spec.”

### Task 1 — Scope `leptosfmt` to the approved roots and fail closed

Update the existing `devtool` command so host Fix/Verify and Nix Verify invoke
the pinned `leptosfmt` only on `web/src` and `csr/src`. Preserve its pinned
defaults, including the recognized `view` and `leptos::view` macro paths and
their full-path semantics. Do not add repository-wide syntax selection, an AST
parser, a placement guard, or a generic discovery framework.

Within those two roots, add checked input enumeration using the pinned
formatter’s glob and exclusion semantics. Treat enumeration, unreadable-input,
parse, and per-file formatter diagnostics as failures even where the formatter
itself would otherwise exit successfully. Handle empty selections and literal
file paths safely. Add focused fixtures for malformed/unreadable inputs and
formatter output/errors, and prove that files outside the two roots are
excluded.

Keep existing named check identity and pinned tool defaults unchanged. Retain
the existing host Fix/index admission behavior; do not broaden the
implementation into hook or CI routing changes.

### Task 2 — Add the coherent Rust-format Nix leaf

Add one Rust-format leaf/catalog verdict consuming the existing `fmt`,
`tools-fmt`, and scoped `leptosfmt` checks. Remove exactly those three
invocations from residual `static-code`, with no duplicates. Preserve named
identities and `--all` order, and consume the new leaf exactly once in both
`nix flake check` and hermetic validation.

Declare and prove the real derivation closure, including the compiled
`devtoolBin`, product/tools offline Cargo homes, pinned formatter/toolchain, and
transitive source/vendor inputs. Product and tools Cargo formatting populations
overlap; keep them together in this leaf. Distinguish direct source/config/tool
identity from transitive closure effects, relevant from unrelated inputs, and
request timing from actual current execution and reuse. Exercise the real leaf
and hermetic consumer for success, controlled failure propagation,
relevant-input invalidation, unrelated-input retention where the full closure
permits, and valid local-store reuse. Do not add a timing protocol, force a
rebuild, purge caches, or claim guaranteed execution on every request.

Reconcile maintained documentation and narrow applicable ADR/source-boundary
references if implementation requires it; default to no new ADR. Keep raw xtask
formatting separate.

### Acceptance and delivery

The Lead reviews and accepts the integrated diff and focused proof. Follow
applicable repository gates and hook-backed staging/commit/push boundaries;
inspect and resolve hook mutations before re-staging or retrying. Do not impose
a blanket full-E2E worker gate. Commit the approved spec, outline,
implementation, and maintained documentation, then follow the normal PR/CI
handoff. Keep proof artifacts ignored; record exact proof references, delivered
revision, and a fresh checkout preservation inventory in the handoff.

An independent admitted checkout need not wait for TypeScript/ERT work:
coordinate exact conflicting edits and rebase actual conflicts only. Do not
duplicate the separately approved xtask-format assignment or expand into Clippy,
hook/CI routing, or other family work.

Proposed canonical spec path:
`docs/superpowers/specs/2026-11-03-issue-1864-rust-format-syntax-selection.md`.
