# #991 — split E2E fixture infrastructure by responsibility

Issue: [#991](https://github.com/jaunder-org/jaunder/issues/991). Milestone:
Code quality: test fixture and lifecycle consolidation.

## Outcome

The E2E fixture infrastructure is separated into modules with one named reason
to change, while every test-facing interface and observable behavior remains
unchanged. `fixtures.ts` remains the single composition and composed-test import
surface for the suite.

## Load-bearing decisions

- `fixtures.ts` owns one explicit, ordered `base.extend({ ... })`, the composed
  `test` export, `expect`, and compatibility re-exports. It does not own fixture
  behavior.
- `performance.ts` owns Performance/OTel lifecycle behavior, trace propagation,
  capture handoff, and navigation telemetry.
- `timeout-policy.ts` owns timeout scaling, the default test budget, and
  `setTestBudget`.
- `provisioning.ts` owns identity, mailbox, seeded authentication, and one-shot
  page provisioning.
- Fixture implementations are imported by name and registered explicitly in
  `fixtures.ts`; object spreads and chained `extend()` calls do not hide
  dependency order.
- The existing fixture registration and teardown order is preserved, including
  `_lifecycleStart`, traced-context capture handoff, automatic timeout fixtures,
  provisioning dependencies, and `_autoPerfSpan`.
- All existing imports from `fixtures.ts` remain valid through explicit
  re-exports. Specs continue to import the composed fixture surface there;
  responsibility-focused contract tests import the module whose implementation
  or contract they prove.
- The static checker replaces its whole-file exemption with violation-specific
  ownership: only `fixtures.ts` may import Playwright’s upstream `test`, and
  only `performance.ts` may create a raw browser context. Its contract tests
  prove both restrictions independently.
- Existing ADR-0039 identity isolation, ADR-0098 seeded authentication, ADR-0111
  one-boot enforcement, #887 telemetry semantics, and #828 timeout scaling
  remain unchanged.
- This reversible TypeScript organization change introduces no domain-language
  or architectural decision requiring a new ADR.

## Acceptance

- Each resulting module has only its named responsibility; `fixtures.ts`
  contains composition, exports, and no fixture behavior.
- The composed fixture dependency order is explicit and unchanged.
- Every existing `fixtures.ts` import remains valid; responsibility-focused
  tests move their imports to the owning module while composed fixture consumers
  require no caller changes.
- Existing telemetry fields, span relationships, timeout formulas, identity
  isolation, authentication artifacts, mailbox behavior, and page-load behavior
  are unchanged.
- Telemetry, duration-budget, boot-budget, and traced-context checker contracts
  continue to pass; tests reside with the implementation or contract they prove.
- Checker coverage proves that only `fixtures.ts` may import Playwright’s
  upstream `test` and only `performance.ts` may create a raw browser context.
- The repository gate passes.

## Boundaries

- No product behavior, E2E scenario behavior, retry policy, worker count,
  browser/backend matrix, timeout value, OTel schema, or fixture public API
  changes.
- No cleanup or redesign of the imported action, capture, mail, polling, seed,
  duration-manifest, or boot-budget subsystems.
- No new fixture framework, compatibility alias, transitional import path, ADR,
  or glossary entry.
