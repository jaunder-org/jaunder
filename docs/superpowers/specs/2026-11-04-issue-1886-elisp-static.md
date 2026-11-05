# #1886 — Isolate Elisp static verification

## Outcome

Move the existing Elisp formatter verification and warnings-as-errors
byte-compilation out of the broad `static-code` Nix derivation into one
independent hermetic verification leaf. This is one incremental #1864 carve-off;
it neither completes nor closes #1864.

## Approach and tradeoffs

Keep `devtool check elisp-fmt` and `devtool check byte-compile` as the single
command definitions shared by the existing host verify ladder and Nix. The Nix
leaf requests each command once in verify mode. Formatting remains a check,
never the host `check` auto-fix operation; byte-compilation continues to fail on
compiler warnings and writes no `.elc` files into the source tree.

Preserve the commands' exact non-Markdown Elisp file populations and actual
inputs: the Elisp files and loaded scripts/configuration, pinned `emacsForCi`,
timezone data/environment, and other dependencies actually read by these
commands. Declare sufficient source/tool inputs so relevant source or
configuration changes alter the leaf identity while valid output reuse remains
available. Existing host checks and populations are unchanged.

Pure ERT remains the independent `static-ert` verdict, and the separate ERT
coverage producer remains unchanged. ADR-0031's separately-tested Elisp boundary
and ADR-0052/0146's shared `devtool` command ownership remain in force; this
changes only Nix leaf responsibility. Update maintained verification and
invalidation-boundary documentation/assertions so the new leaf is visible and
correctly attributed.

## Acceptance

- The new leaf runs the existing formatter in verify mode and warnings-as-errors
  byte-compile command, each exactly once, over their current populations; it
  does not run ERT or introduce duplicate command implementations.
- Nix inputs capture the complete actual source, pinned Emacs, timezone and
  command dependency closure; relevant inputs invalidate the leaf and valid
  outputs remain reusable.
- A controlled failure of either command fails the leaf and propagates as a
  failure through the hermetic validation consumer, which requests the leaf
  exactly once.
- The existing host formatting Fix/verify behavior, host byte-compilation check,
  ERT leaves/coverage, required-check selectors, hooks and unrelated leaves
  remain unchanged. Maintained verification and source/invalidation
  documentation reflect the new boundary.

## Boundaries

No host check, hook, CI selector, or required-check routing changes; no new
framework, devtool command, or ADR decision. Do not fold ERT into this leaf or
alter its coverage producer. #1885's end-to-end census is independent; any later
shared Nix/catalog integration is rebase-only and must not import unlanded peer
commits. #1880, #1882 and #1884 remain independently owned and out of scope.
Full validation remains authoritative after implementation; this discovery/spec
task runs no builds or implementation proofs.
