# Plan — issue #412: correct ADR-0062's coverage claim

Single-task, docs-only. Spec:
`docs/superpowers/specs/2026-07-15-issue-412-adr0062-coverage-correction.md`.

## Task 1 — correct ADR-0062 and document the covering technique

- [x] Add `- Amended: 2026-07-15 (#412) — ...` to the ADR-0062 metadata header
      (ADR-0050 convention).
- [x] Replace the "No new coverage surface" Consequences bullet with the
      corrected wording (keeps build-time/no-runtime-footprint; corrects "no
      gate-measured lines"; cites #403).
- [x] Fold the covering technique into that bullet (`syn::parse_quote!` in-crate
      unit tests + `// cov:ignore` the `?`-fall-through brace; cite
      `macros/src/str_newtype.rs:304` / `storage/src/backup.rs:515`).
- [x] `prettier -w docs/adr/0062-macros-crate-proc-macro-home.md`.
- [x] Gate: `cargo xtask check --no-test` (static/link/README-table sanity).
- [x] Commit referencing #412.

No separable follow-ups anticipated.
