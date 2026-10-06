# Issue #1692 — Home composer collapse regression

## Outcome

Home's eligible empty, unfocused composer collapses when the User drives the
intended downward-scroll transition. Expanding a manually compacted composer
preserves editing state. The browser proof reliably drives those behaviors in
local debug and hermetic release execution.

## Load-bearing decisions

- Diagnose before choosing a fix: the owning cause may be browser-test scroll
  driving or product collapse behavior. Neither is presumed defective.
- Preserve the existing collapse eligibility policy; this is not a redesign of
  Home, its scrolling model, or its composer.
- Focused or dirty editing state must continue to prevent automatic collapse.
  Manual collapse must retain draft body and format-menu state, and explicit
  expansion/collapse must retain the tested focus handoff.
- Keep a real browser proof capable of detecting the original symptom, not only
  a host-unit approximation of browser event delivery.
- Synchronize on observable state or browser lifecycle evidence rather than
  arbitrary sleeps. Do not weaken assertions, inflate timeout budgets, or skip
  the failing scenario to obtain green results.
- Compare local debug and hermetic release execution before claiming behavior
  across those environments. A passing run in one does not establish parity.
- Respect ADR-0070's host/wasm boundary, ADR-0083's host-tested state decisions,
  ADR-0012's timeout policy, and ADR-0111's one document boot per Page.

## Acceptance

- Record the exact focused command and observed failure for the original
  `Home preserves editing state while its composer is compact` scenario, or
  report inability to reproduce without inventing a root cause.
- Establish evidence distinguishing scroll-transition driving from collapse
  eligibility/state behavior; describe the demonstrated cause in the PR.
- The retained browser scenario proves automatic collapse after clearing the
  body and moving focus out of the composer, followed by downward scrolling.
- The same scenario continues to prove focused/dirty protection, draft-body and
  format-menu preservation through manual compaction, focus handoff, reduced
  motion, and compact behavior at the existing mobile viewport.
- Run the focused local debug proof after the fix and report its result. Compare
  against hermetic release browser evidence for the same scenario; identify
  backend/browser coverage and any unresolved differences explicitly.
- If product state logic changes, add red-capable host tests where the behavior
  is host-testable. Browser wiring remains covered by the real browser proof.
- If user-visible presentation changes, provide comparable before/after visual
  proof of Home's expanded and compact states at affected viewports.
- Remove temporary diagnostic instrumentation and report applicable verification
  results without hiding failures.

## Boundaries

No new persistence, protocol, authentication, or domain-language changes. No
unrelated Post Actions styling fixes or broad browser-harness redesign. Adjacent
composer tests may change only when the same demonstrated cause applies. A new
architectural decision or materially changed eligibility policy requires renewed
design discussion rather than silently broadening this bug fix.
