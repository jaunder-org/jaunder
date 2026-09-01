# Issue #1017: Centralize the backend e2e check builder

## Outcome

SQLite and PostgreSQL e2e checks are produced by one backend-aware NixOS-test
builder. The refactor gives the shared VM and test lifecycle one owner without
changing the four ADR-0034 gate derivations, the single-worker diagnostic
derivations, or their runtime behavior.

## Load-bearing decisions

- `flake.nix` retains one private e2e check builder receiving `backend` and the
  existing common check inputs.
- The builder selects one backend policy record. Each policy owns its database
  package, Jaunder database URL, backend-specific NixOS module fragment, and
  setup performed before Jaunder starts.
- Only `sqlite` and `postgres` are valid backend values. Any other value fails
  during Nix evaluation rather than silently selecting PostgreSQL.
- The common builder owns the NixOS test wrapper, global timeout, VM resources,
  shared packages, OTel collector, Jaunder service configuration, e2e package
  copy, seeding, Playwright execution, and diagnostic capture.
- Common NixOS configuration is composed with the selected backend fragment.
  Callers do not supply implementation fragments.
- Runtime order remains: start the VM; wait for the OTel service and receivers;
  perform PostgreSQL-only service readiness and database creation; start and
  await Jaunder; copy the e2e package; seed the empty database and verify seed
  spans; run and capture the selected browser.
- SQLite retains its existing file database and no bootstrap step. PostgreSQL
  retains its service configuration, credentials, database creation command, and
  connection URL.
- `mkE2eCombo` continues to own derivation naming, trace identifiers,
  cache-busting salt, optional environment, and VM resource arguments; it
  delegates the backend choice to the shared builder.
- No new ADR is required: this refactor preserves ADR-0034 and existing backend
  behavior rather than introducing an architectural choice.

## Acceptance

- The separate SQLite and PostgreSQL e2e check builders are removed; every
  `e2eCombos` gate and single-worker caller reaches the one backend-aware
  builder through `mkE2eCombo`.
- Before implementation, record the evaluated `drvPath` for all four gate checks
  and all four single-worker packages. After implementation, every corresponding
  `drvPath` is byte-identical; this proves that derivation names and complete
  evaluated inputs remain unchanged.
- The unchanged derivations therefore retain the current timeout, memory, core
  count, packages, service configuration, environment, seed database,
  worker/retry settings, trace inputs, lifecycle script, and diagnostic artifact
  set.
- SQLite and PostgreSQL preserve their current boot, bootstrap, copy, seed, and
  run ordering.
- Conformance review verifies that the backend selector has explicit SQLite and
  PostgreSQL policies plus a direct backend-specific error for every unsupported
  value; all eight supported policy/browser/resource combinations are exercised
  by the `drvPath` comparison.
- One real Chromium e2e check passes for SQLite and one for PostgreSQL. Each
  successful check runs the existing seed-span assertion and produces its named
  Playwright report, duration manifest, Playwright artifact archive, system
  journal, and capture archive; the browser-independent shared path makes a
  duplicate Firefox smoke run unnecessary.
- `cargo xtask check` passes.

## Boundaries

- Do not split or relocate `flake.nix`; that belongs to #985.
- Do not change the e2e matrix, derivation names, VM sizing, worker counts,
  retries, timeout budgets, cache salt behavior, or resource conclusions from
  #828.
- Do not change OTel endpoints, collector behavior, trace identity, diagnostic
  capture, seed contents, Playwright configuration, or behavior owned by #802.
- Do not alter product code, database schemas, CI fan-out, branch-protection
  checks, or public/test interfaces.
