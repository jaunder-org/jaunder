# Lint Suppression Guardrail

**Issue:** [#294](https://github.com/jaunder-org/jaunder/issues/294)

## Context

`CONTRIBUTING.md` requires explicit user approval before a Rust `#[allow(...)]`
or `#[expect(...)]` lands. Issue #94 reconciled the prior inventory but did not
make an addition mechanically visible. The repository now contains thirteen
attributes: nine existing `#[expect]` attributes (including the integration-test
crate root) and four legacy test-scoped `#[allow]` attributes. The latter
contradict #294's intended expect-only ratchet, so this issue deliberately
expands from its original out-of-scope reconciliation boundary only enough to
eliminate those four sites without changing test behavior.

## Decision

Add a host-side `lint-suppression` step to `cargo xtask check` and
`cargo xtask validate`.

- It is a **hard fail**, not a review-only annotation.
- It parses every Rust file below the fixed workspace/tool source and test
  roots: `client/src`, `common/src`, `csr/src`, `host/src`, `macros/src`,
  `macros/tests`, `server/src`, `server/tests`, `storage/src`,
  `test-support/src`, `test-support/tests`, `web/src`, `tools/coverage/src`,
  `tools/devtool/src`, `tools/devtool/tests`, `tools/doctests/src`, and
  `xtask/src`; it also parses the standalone first-party build script
  `server/build.rs`.
- A missing root, unreadable source file, or `syn` parse failure fails the step;
  the guard must not silently shrink its population.
- It recognizes Rust lint attributes whose structural path is exactly `allow` or
  `expect`, including lint attributes nested inside `cfg_attr`; comments,
  strings, and unrelated attributes are not members of the population.
- Each discovered site is identified by repository-relative file path, attribute
  start line, attribute kind, and normalized argument tokens. A permitted
  `#[expect]` must carry a non-empty `// lint-suppression:allow <reason>` marker
  on the immediately preceding source line.
- An `#[allow]` always fails. An `#[expect]` fails unless its source-site marker
  is present and non-empty. Bare markers, orphan markers, and marker lines that
  point at multiple lint attributes fail, so removing or relocating a
  suppression cannot leave dead approval behind.
- The adjacent marker is the reviewable record of explicit user approval. The
  implementation documentation and `CONTRIBUTING.md` state that it may be added
  only after that approval; the guard cannot infer human approval.

Eliminate all four current test-scoped
`#[allow(clippy::unwrap_used, clippy::expect_used)]` attributes. Retain only
fulfilled self-removing expectations: `web/src/test_support.rs` keeps both lint
expectations and `web/src/posts/api.rs` keeps `clippy::unwrap_used`; remove the
unfulfilled attributes from `web/src/subscriptions/server.rs` and
`web/src/timeline/server.rs`. Preserve test behavior.

No ADR is needed: ADR-0085's enumerate/fail-closed discipline and ADR-0094's
source-gate conventions already decide the relevant architecture.

## Acceptance criteria

1. `cargo xtask check` and `cargo xtask validate` both report the new
   `lint-suppression` step.
2. Every current first-party Rust source, test, and build-script path is scanned
   recursively where applicable; a missing root, unreadable, or syntactically
   invalid source file fails the step.
3. All four legacy test-scoped `#[allow]` attributes are eliminated. Only
   fulfilled `#[expect]` lint members remain; unfulfilled expectations are
   removed without changing test behavior.
4. The committed source-site marker inventory is exactly the post-cutover
   expect-only inventory; the normal tree passes without `#[allow]` attributes.
5. A synthetic unapproved `#[expect]`, any `#[allow]` (including one nested in
   `cfg_attr`), a bare marker, an orphan marker, a marker that points at
   multiple lint attributes, and a parse failure each fail in focused unit
   tests. A marked `#[expect]`, a marked `cfg_attr(..., expect(...))`, and
   unrelated attributes/comments/strings pass.
6. Failure output names each offending repository-relative site, includes the
   derived approved-expectation census, and tells the developer that a new
   suppression needs explicit approval in a reviewed source-site marker.
7. `CONTRIBUTING.md` documents the hard guardrail and marker approval protocol.

## Non-goals

- Re-auditing or removing the nine existing `#[expect]` sites.
- Changing the underlying panic calls or test logic.
- Guarding runtime test ignores, coverage markers, non-Rust sources,
  command-line `-A` flags, or third-party/generated files.
- Replacing the existing per-gate source-marker mechanism from ADR-0094; this
  gate uses that mechanism.
