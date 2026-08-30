# Issue #1209: Free-function path qualification

## Outcome

Production Rust call sites consistently expose the module that owns every
nonlocal free function or constant. Confirmed candidates use one meaningful
owner qualifier without import aliases and without changing behavior or generated
code. The sole reviewed public API rename changes the former AtomPub error path
to `host::atompub::Error`, with no compatibility alias.

## Load-bearing decisions

- A nonlocal free function is called through exactly one meaningful owner
  qualifier: `module::function()` by default, or `super::function()` when the
  immediate parent intentionally owns the operation.
- Import the owner module rather than a free function or constant item, and
  qualify the use through that owner. Types, enums, and traits remain
  item-oriented; associated functions, methods, enum variants, and macros keep
  their existing semantic form.
- Parent and vertical façades remain valid ownership boundaries, but their free
  functions are still called as `super::function()` or `module::function()`;
  they are not flattened or called unqualified.
- Production imports use no aliases, including `as _`. Name collisions are
  resolved with owner-qualified paths, direct unaliased trait imports, or UFCS.
  Public re-exports remain when they define an existing API, except that the
  reviewed AtomPub error path is renamed to `host::atompub::Error`.
- A long crate-rooted path is repeated when the same source file calls free
  functions from that owner module at least twice. Import that owner module and
  use one qualifier. A single already-qualified call may retain its longer path
  when no owner-module import is otherwise needed in that file.
- An exception is reviewable only when the source or an existing repository
  decision documents the façade/collision, or the conformance evidence records
  the one-off path and why an import would obscure ownership.
- Production means Rust under `client/src`, `common/src`, `csr/src`, `host/src`,
  `macros/src`, `server/src`, `storage/src`, and `web/src`, including
  target-gated code. `cfg(test)` modules, `test-support`, standalone `xtask` and
  `tools` workspaces, focused tests, generated output, and macro expansion
  artifacts are out of scope unless an in-scope edit requires a minimal compile
  repair.
- The issue's AST inventory—340 candidate import bindings, 566 candidate call
  sites across 125 files, and 10 long `common` paths—is a discovery baseline,
  not an acceptance quota. Syntax-aware or name-resolved review decides the
  confirmed set.
- This is a complete production cleanup, not a per-crate partial migration.
  Every confirmed production candidate is migrated in the same change.
- No new lint, compatibility alias, or enforcement gate is added. Conformance is
  proved by retained review evidence plus the existing checks.

## Acceptance

- Ship evidence records the exact syntax-aware or name-resolved procedure, the
  source roots and exclusions, disposition totals, every intentional exception
  path, and zero remaining confirmed violations. The evidence may live in the
  pull request; no permanent lint or checked-in inventory is required.
- The inventory distinguishes direct nonlocal free-function and constant
  imports, repeated long paths, and import aliases from associated items,
  methods, macros, variants, generated names, tests, and intentional exceptions.
- Every confirmed production direct import of a nonlocal free function or
  constant becomes an owner-module import plus owner-qualified use, or a direct
  `super::function()`/`super::CONSTANT` use for a parent-owned item.
- No production `use` item contains `as`, including capability imports written
  as `as _`; collisions use explicit owners or UFCS instead.
- Every source file with at least two long crate-rooted free-function calls from
  the same owner module imports that module and uses one meaningful qualifier.
- Documented parent façades, recorded one-off explicit paths, public re-exports,
  and generated-name exclusions remain reviewable after the cleanup.
- Types, enums, traits, associated functions, methods, enum variants, and macros
  retain their existing semantics without import aliases.
- Target-gated production crates compile under the existing precommit coverage.
- `cargo xtask precommit` passes through the repository commit hook.

## Boundaries

- No runtime behavior, module ownership, or visibility change. The sole reviewed
  public API change renames the former AtomPub error path to
  `host::atompub::Error`, with no compatibility alias.
- No test or test-support style sweep.
- No generated-file edits or blind textual rewrite.
- No new lint policy, allowlist, compatibility layer, or import cleanup outside
  the approved free-function, constant, and alias policy.
