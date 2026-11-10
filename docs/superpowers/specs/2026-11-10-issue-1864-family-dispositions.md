# #1864 finite verification-family dispositions — draft

## Outcome

Document a finite share / retain distinct / defer decision for every current
verification family, so contributors know what each verdict establishes without
adding execution machinery. Preserve delivered sharing and required assurance;
universal host-to-Nix sharing is not the outcome.

## Approach and tradeoffs

Change only `CONTRIBUTING.md` (parity and current Nix inventory) and
`docs/ARCHITECTURE.md` (current workflow/static-family ownership), using concise
maintained guidance rather than a dated audit ledger. Accept repeated
host/hermetic checks: replacing them with a universal verdict would erase
mutable-tree, offline or population guarantees or add unjustified
admission/orchestration machinery.

- Markdown/Prettier: retain the x86_64-linux shared `static-docs` verdict; host
  Fix, raw/index admission and one reconciliation remain separate safety
  responsibilities.
- Other Prettier/Rust formatting: retain shared definitions but distinct host
  Fix/Verify and hermetic verdicts; scoped Rust formatting and raw xtask
  formatting have different populations.
- TypeScript/census: retain distinct compiler and E2E-population checks; their
  independent leaves do not establish shared hook verdicts.
- Elisp static/ERT/coverage: retain formatting/byte-compilation, pure behavior
  and combined pure/live coverage authorities; none replaces the others.
- Root/auxiliary Rust tests and Clippy: retain fresh host tests, instrumented
  dual-backend coverage, auxiliary suites and target-specific lint units;
  offline lint/dependency policy is not host advisory assurance.
- Doctests: retain host/sandbox execution and auxiliary populations; preserve
  shared bidirectional fence reconciliation and required status gates.
- Rust coverage: retain its producer, hard status verdict and host line/CRAP
  policy; host test success cannot replace instrumented SQLite/PostgreSQL
  evidence.
- Wasm/browser/E2E: retain separate primitive, size-budget and diagnostic
  checks; retain actual full-local/CI catalog, producer and reconciliation
  sharing, not equivalence to focused host E2E. Defer further family migration,
  dependency pruning and verdict unification unless a concrete maintenance
  defect, consumer need or attributable workflow cost warrants a separately
  scoped decision.

## Acceptance

Current guidance accounts for every family’s selected consumers, inputs, modes,
platform limits, Git/index needs, populations and producer/verdict owner, with
the reasons above. Correct obsolete xtask-Clippy host-only/leaf-inventory
wording and pending E2E-migration ownership prose; describe current behavior
without changing it or claiming speedups. For every actually selected sharing
boundary, including retained Markdown and full-local/CI E2E sharing, establish
the same consumer contract and verdict, compatible-platform derivation identity
or explicit proven equivalence, legitimate reuse, relevant-input invalidation
and meaningful failure propagation through both consumers. Distinguish E2E’s
shared producer/reconciliation contracts from its deliberately different local
and distributed scheduling; never substitute a producer-output join for the
complete verdict. Status-as-data requires its verdict gate; missing, malformed,
incomplete or wrong-output evidence must not pass. Preserve all required
aggregates, populations and backend/browser parity. Reuse adequate historical
proof only after checking its source/tool/consumer applicability; source wiring,
merged history, a test’s presence or stored-output availability alone does not
fulfill executable acceptance. The ignored
`.xtask/issue-1864-intake/proof-index.md` records reusable candidates and
unresolved gaps; acceptance remains unfulfilled wherever adequate proof is
absent, until justified focused proof is assigned under approved scope. Keep
receipts in ignored evidence, not committed audit inventories; retain only
enduring current guidance in the two implementation documents.

## Boundaries

This draft is not spec approval, implementation authority, a proof waiver or
issue closure. Cold soundness review and explicit human spec approval remain
required; no outline is expected because no execution or architectural boundary
changes. Exclude `nix/checks.nix` even comment edits, Justfile, CI, tools,
instrumentation, new routing/sharing, framework work and policy exceptions; no
gates, builds or probes are authorized by drafting. Leave #1933’s
retention/export guidance untouched; coordinate only concrete overlapping
regions through Manager. #1898 remains human-PAUSED with no contact, resumption
or other-checkout inspection. Preserve host/index safety, honest failures,
exact-head merge approval and the managed semantic branch/checkout; no commits,
tracker changes or cleanup follow from this draft.
