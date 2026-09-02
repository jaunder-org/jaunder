# Reader-Aware Elisp Census Implementation Outline

> Execute with `jaunder-iterate`, using `jaunder-dispatch` when delegation is
> useful. This outline exists because the fix introduces a shared reader
> boundary consumed by two census collectors.

## Scope

In:

- One shared owner for Emacs reader protocols and process classification.
- Clean migration of Elisp clone-shape collection.
- Reader-aware top-level dependency extraction and its regression coverage.
- Focused collector checks and the original full-census reproduction.

Out:

- Census report/schema changes, non-Elisp collectors, production Emacs code,
  generic process redesign, or new dependencies.

## Task outline

- [x] Task 1: Establish the shared Elisp reader owner and migrate clone shapes
  - Contract: `census::elisp` exposes the shared reader error plus the
    function-shape operation; its embedded program stays private, and process
    drainage/cleanup remains owned by `census::process`.
  - Contract: `clone` keeps its collector-local injection seam but delegates the
    real reader implementation to `census::elisp`; its private process
    implementation is deleted.
  - Verification: focused xtask clone collector tests prove actual-reader
    normalized shapes and injected unavailable behavior still pass.

- [x] Task 2: Replace dependency prechecks and line scanning with the shared
      reader
  - Contract: Task 2 adds the specialized top-level-dependency operation to
    `census::elisp`; `dependency` has a collector-local injected reader seam
    whose real implementation calls that operation once per source.
  - Contract: only literal top-level quoted `require` forms are returned.
    Reader-valid string/comment parentheses, multiline forms, require-shaped
    non-code, nested forms, malformed source, and unavailable-reader states each
    have focused coverage.
  - Verification: focused xtask dependency collector tests pass, then
    `cargo xtask census --json` completes without the false
    `jaunder-pull-media-test.el` failure.

## Risk checks

- Preserve `census-elisp-structural` identities, dependency candidate
  identities, limitations, path attribution, and failed/unavailable state
  distinctions.
- Keep one source per Emacs invocation; do not introduce multi-file framing or a
  fallback parser.
- Preserve every existing early-error drain, termination, and reap path while
  moving reader ownership.
- Delete `balanced_elisp`, `elisp_require`, and the clone-local process
  implementation only after all callers migrate.
- Keep `mod.rs` assembly-only and use repository Rust ownership-path
  conventions.
- Run the commit gate only after both tasks and the original census reproduction
  are green; at the ship boundary, require the normal PR CI gate before merge.
