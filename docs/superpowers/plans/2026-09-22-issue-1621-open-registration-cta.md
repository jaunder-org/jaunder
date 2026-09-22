# Issue #1621 — Open Registration CTA Outline

## Trigger

The visible change is small, but Registration Policy must join the serialized
Local projector/CSR presentation seed. That cross-runtime contract warrants a
short outline.

## Scope

**In:** Local-only Register CTA policy, Local presentation/seed propagation,
four-policy host/browser coverage, and transient visual proof.

**Out:** changes to `/register` or invitation behavior, authorization, other
public mastheads, and styling. Regression proof for unchanged direct invitation
links remains in scope.

## Execution

Use `jaunder-iterate`; use `jaunder-dispatch` only if an isolated task benefits
from delegation.

- [x] **1. Deliver policy-complete Local presentation end to end**
  - Extend the shared Local presentation and `PageSeed::SiteTimeline` contract
    with typed current Registration Policy.
  - Resolve and propagate that policy through both the initial projector and
    `list_local_timeline`, then carry it through `LocalDestination` and commit
    it with identity/page; seeded cold mounts adopt it synchronously.
  - Make the shared pure Local masthead render Register only for `Open`; retain
    Sign in for every resolved policy and for an unresolved policy.
  - Ensure unseeded loading/failure never manufactures `Open` or displays
    Register, while existing Local failure handling and server authority remain
    unchanged.
  - Pin the four-policy matrix, projector/seeded coincidence, seed
    serialization/adoption, and unresolved/failure-safe omission in host and
    HTTP tests.
  - Focused proof: relevant `common`, `web`, and projector unit/integration
    tests.
  - Commit through `jaunder-commit`.

- [x] **2. Prove browser behavior and retained invitation entry**
  - Add a focused Local browser flow proving the four policy outcomes and
    observing the DOM throughout unseeded loading/failure so Register never
    flashes.
  - Preserve existing invitation behavior and extend browser verification so a
    direct invitation URL reaches invited registration under both
    `OperatorInvites` and `MemberInvites`.
  - Do not add a generic invitation-request path or alter direct `/register`
    behavior.
  - Focused proof: the narrow Local CTA and invitation Playwright cases.
  - Commit through `jaunder-commit`.

- [ ] **3. Capture and verify presentation evidence**
  - Capture transient before/after Closed Local screenshots with Studio, signed
    out, fixed Site Identity/Post, `1440×900` and `390×844`, after the specified
    readiness barriers.
  - Compare for only the Register CTA removal; do not create or repurpose a
    committed baseline unless one already governs this exact Local state.
  - Run the focused Local browser flow and applicable final gate, then commit
    any test/document updates through `jaunder-commit`.

## Key contracts

- `RegistrationPolicy::Open` is the sole positive condition; every other or
  unresolved state omits Register.
- Local projector HTML and seeded CSR consume the same typed policy value.
- Policy is part of route presentation, not browser-only convenience state.
- A failed policy read follows existing Local destination failure handling and
  never falls back to `Open`.
- Other public mastheads and `/register` remain untouched.

## Risk checks

- Seed serde fixtures and projector response assertions catch wire-shape drift.
- Shared masthead tests prevent projector/CSR markup divergence.
- Browser coverage observes the DOM throughout loading/failure rather than
  checking only the final frame.
- Visual proof verifies no incidental masthead layout or theme changes.
