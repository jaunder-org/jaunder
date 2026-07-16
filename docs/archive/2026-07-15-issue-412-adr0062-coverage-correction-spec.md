# Spec — issue #412: correct ADR-0062's coverage claim

## Problem

ADR-0062 ("A `macros` crate as the workspace's proc-macro home"), in its
Consequences section, asserts:

> No new coverage surface: a proc-macro crate is a separate `.so` loaded by
> rustc at build time and is not linked into the instrumented test binaries, so
> its code contributes no gate-measured lines.

This is **false in practice**. During #403 (the `StrNewtype`/`IdNewtype`
derives), `cargo xtask check` flagged uncovered lines in `macros/src/*.rs` (the
derive error paths); they had to be covered by in-crate `#[cfg(test)]` unit
tests, plus one `// cov:ignore` on an llvm-cov gap-region brace. The coverage
gate **does** measure the `macros` crate.

The true part of the bullet — build-time-only, no runtime footprint in
dependents — stays true; only the "no gate-measured lines" conclusion is wrong.

## Scope

Docs-only. No code, no ADR draft (this corrects a factual error in an existing
accepted ADR — it is not a new architectural decision). Single file touched:
`docs/adr/0062-macros-crate-proc-macro-home.md`.

## Changes

1. **Metadata header** — add an `- Amended: 2026-07-15 (#412) — ...` line,
   matching the ADR-0050 amendment convention.

2. **Consequences bullet** — replace the "No new coverage surface" bullet with a
   corrected one that:
   - keeps the true claim (build-time `.so`; no runtime footprint / not linked
     into a dependent's _runtime_ binary);
   - corrects the false conclusion: `macros` **is** gate-measured — it is a
     workspace member, the Nix coverage source filter auto-admits it (per the
     preceding bullet), and its own instrumented `#[cfg(test)]` unit-test binary
     measures `macros/src/*.rs` like any other crate;
   - cites #403 as the concrete instance that hit this.

3. **Covering technique** (satisfies the second acceptance criterion) —
   document, in the same bullet, the technique #403 established for future macro
   authors:
   - drive `compile_error!` / `?`-error branches from in-crate `#[cfg(test)]`
     unit tests that feed `syn::parse_quote!`-built `DeriveInput` fixtures to
     the derive fn and assert on its token output (precedent: the
     `macros/src/lib.rs` tests);
   - annotate the `?`-fall-through closing brace `// cov:ignore` — llvm-cov
     leaves it unmarked as a gap region even when both arms are exercised
     (precedent `macros/src/str_newtype.rs:304`, mirroring
     `storage/src/backup.rs:515`).

## Acceptance (from the issue)

- [ ] ADR-0062 no longer claims the macros crate contributes no gate-measured
      lines.
- [ ] The covering technique is documented (in the ADR).

## Verification

Docs-only Markdown edit. Gate: `prettier -w` the file before staging (per the
pre-commit-prettier convention), then `cargo xtask check --no-test` for the ADR
README-table / link sanity. No coverage/e2e surface.
