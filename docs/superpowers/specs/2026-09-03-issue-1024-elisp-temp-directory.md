# Issue #1024 — centralize integration temp-directory lifecycle

## Outcome

The four media integration tests use one shared macro for temporary-directory
creation and cleanup. Fixture construction, assertions, and live-server lifetime
remain local to their current owners.

## Load-bearing decisions

- Add `jaunder-test--with-temp-directory` to
  `elisp/test/jaunder-integration-helper.el`; the helper owns only directory
  lifetime.
- The macro accepts an explicit caller binding and prefix, conceptually
  `(jaunder-test--with-temp-directory (directory prefix) body...)`, matching the
  established test-macro convention while preserving each caller's local name
  and existing `"jaunder-media-"` or `"jaunder-att-"` prefix.
- Create the directory with `make-temp-file` in directory mode, evaluate the
  body under `unwind-protect`, and recursively remove it with the existing
  unguarded `delete-directory` call.
- Keep deletion unguarded and unconditional. Cleanup still runs on errors,
  signals, and non-local exits, and a cleanup error retains its current ability
  to supersede a body error.
- Give the macro the repository-standard docstring plus indentation and debug
  declarations for its binding form.
- Migrate exactly the four audited tests in
  `elisp/test/jaunder-media-integration.el`. Their fixture paths and contents,
  request setup, assertions, and enclosing `jaunder-test--with-live-server`
  calls remain local and unchanged.
- Do not absorb the integration harness's server root or suite-wide server
  lifecycle. ADR-0035 continues to govern those resources.
- Other integration and pure-test directory helpers are excluded because they
  own richer fixtures, buffers, servers, or distinct cleanup semantics.
- This mechanical test refactor adds no domain terminology or architectural
  decision; `CONTEXT.md`, ADRs, and `docs/ARCHITECTURE.md` remain unchanged.

## Acceptance

- All four audited media integration tests use the shared directory-lifetime
  macro, with no duplicated local `make-temp-file` / `unwind-protect` /
  `delete-directory` lifecycle remaining in those tests.
- The macro preserves each test's directory prefix, recursive cleanup on normal
  and non-local exit, and unguarded cleanup-error propagation.
- Fixture construction, assertions, and live-server ownership remain at their
  existing call sites.
- The affected media integration tests pass.
- `cargo xtask check` passes.

## Boundaries

- No test-suite split or runner change from coordination issue #992.
- No generalized fixture, buffer, server, or cleanup-policy abstraction.
- No public, production, or unrelated test interface changes.
