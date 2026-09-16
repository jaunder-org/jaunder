# Preserve Post Actions focus around accessibility scanning

## Outcome

The Post Actions end-to-end flow reliably proves both the native popover
keyboard contract and the accessibility of its open state without one proof
invalidating the other's focus preconditions.

## Load-bearing decisions

- Treat keyboard behavior and accessibility scanning as separate phases within
  the existing owner Post Actions flow.
- Prove the keyboard phase first from an explicitly asserted focused invoker:
  Tab enters the open popover, and Escape closes it and restores focus to that
  invoker.
- Reopen the popover before the accessibility phase and assert that its intended
  open state is visible before scanning the complete mounted document.
- Do not rely on an accessibility scan preserving browser focus or native
  popover interaction state.
- Retain the existing native-popover behavior and accessibility policy; this is
  test stabilization, not a product behavior change.

## Acceptance

- Immediately before Tab, the test proves the Post Actions invoker is focused
  and its controlled popover is open.
- Tab focuses the Edit action inside the popover.
- Escape closes the popover and returns focus to the same invoker.
- The popover is deliberately reopened and confirmed open before the axe scan.
- Ten consecutive focused `cargo xtask e2e-local post-actions.spec.ts:7`
  invocations pass with the local runner's default zero retries, without added
  sleeps, assertion timeout increases, or whole-test budget increases.
- The authoritative `cargo xtask e2e sqlite chromium` lane that exposed the
  reported failure passes.

## Boundaries

- Do not weaken, exclude, or relocate the existing complete-document WCAG scan.
- Do not alter Post Actions product markup, styling, or interaction behavior
  unless implementation reveals a reproducible product defect outside the
  reported test race.
- Do not broaden this work into unrelated Post Actions scenarios or general axe
  helper changes.
