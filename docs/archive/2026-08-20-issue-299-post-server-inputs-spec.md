# #299 - retire post server-function arity suppression

Issue: [#299](https://github.com/jaunder-org/jaunder/issues/299). Milestone:
Code quality ratchet.

## Summary

`create` and `update` in `web/src/posts/api.rs` already use ADR-0129-style
request aggregation: `create(post: PostInputs)` and
`update(post_id: PostId, post: PostInputs)`. The original scalar-argument
`#[allow(clippy::too_many_arguments)]` sites no longer exist.

The remaining issue #299 work is the temporary wasm-clippy allowance that was
added while those server functions still exceeded Clippy's argument threshold.
Jaunder will remove that temporary `-A clippy::too_many_arguments` from both
static-check definitions and prove the wasm lint path stays green.

## Decision

Remove the #299-specific `clippy::too_many_arguments` allowance from:

- `xtask/src/steps/static_checks.rs`, including the unit test that locks the
  wasm-clippy argv; and
- `flake.nix`'s matching `wasm-clippy` derivation.

Update the surrounding comments so they describe only the permanent wasm-clippy
purpose: linting wasm-only `web`, `client`, and `csr` code that host clippy
cannot see. Do not leave a stale reference to the retired post server-function
exception.

Do not change `PostInputs`, the `create`/`update` server-function signatures,
the generated endpoint paths, request assembly, form behavior, or the private
CSR wire shape. ADR-0129 already accepts the nested server-function argument
shape for cohesive requests, and `/api/*` remains Jaunder's private CSR protocol
under ADR-0082.

## Acceptance criteria

AC1. No production `#[allow(clippy::too_many_arguments)]` remains in the
repository.

AC2. The wasm-clippy command used by `cargo xtask check` no longer passes
`-A clippy::too_many_arguments` in either `xtask/src/steps/static_checks.rs` or
`flake.nix`, and the `static_checks` unit test expects the stricter argv.

AC3. `cargo xtask check` passes, proving the `xtask` unit test, host clippy,
wasm-clippy, formatting, static checks, and coverage accept the stricter lint
configuration.

AC4. A focused post create/edit browser proof passes with
`devtool run -- cargo xtask e2e-local posts.spec.ts`, covering the existing
`PostInputs` request shape through the Leptos-generated client.

## Out of scope

- Renaming `PostInputs` or splitting it into operation-specific request structs.
- Changing the post create/edit form state model or validation rules.
- Changing endpoint paths, authentication, storage mutation semantics, tag
  handling, summary handling, audience handling, or publication scheduling.
