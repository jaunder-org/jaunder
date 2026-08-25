# Issue #800: Stable mount-shift observation

## Outcome

The authenticated-owner and timeline mount-shift scenarios compare projector
paint with a settled post-mount layout, so Firefox paint scheduling cannot turn
an unchanged layout into a flaky failure.

## Load-bearing decisions

- The shared mount-shift probe owns post-mount geometry stability for every
  caller; individual scenarios do not add browser-specific waits.
- Post-mount top-left geometry is settled by equal samples on consecutive
  browser animation frames after mount and caller-supplied readiness complete.
- Settling is condition-based and bounded. No fixed-duration sleep is
  introduced.
- Settled post-mount geometry is still compared with the original
  projector-paint geometry using each caller's existing tolerance; issue #800
  does not loosen an exact assertion.
- This remains an end-state comparison. Detecting every transient layout
  movement would be a stronger, different cross-browser observation contract.
- Existing CI retry policy and flaky-reporting policy remain unchanged.

## Acceptance

- The authenticated-owner scenario passes without retry while retaining exact
  `post-head` and `post-body` position checks.
- All timeline mount-shift scenarios use the same stability boundary and retain
  their existing exact target checks.
- A focused run contains no fixed wait and reports a named failure if post-mount
  target geometry never stabilizes within its bounded wait.
- The authoritative Firefox CI artifact reports the authenticated-owner scenario
  as passed on its first attempt and `flaky-scan` as `0 flaky test(s)`.

## Boundaries

- Do not change product rendering, CSS, projector output, mount signaling, or
  timeout/retry policy to make the assertion pass.
- Do not expand into issue #357's audience refetch/remount assertions; they use
  a different held-response observation boundary.
- Do not add a general browser layout-instability monitor or transient CLS
  policy.
