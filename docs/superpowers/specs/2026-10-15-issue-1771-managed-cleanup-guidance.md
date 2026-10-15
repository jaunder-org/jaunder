# #1771 — Assignment-aware post-merge cleanup guidance

## Outcome

Repository guidance and emitted CLI cleanup help distinguish Manager-assigned
preservation and accepted handoff from ordinary unmanaged branch cleanup.
Contributors can follow the appropriate path without losing retirement
selectors.

## Load-bearing decisions

- For active Manager-assigned work, implementors and assigned Leads leave the
  semantic branch checked out and the checkout intact after verified merge; they
  skip `cargo xtask pr cleanup` and equivalent detach/delete operations.
- Lead accepts merge, validation, and preservation evidence and hands it to
  Manager. Delivered work and pending Manager-owned lifecycle cleanup are
  separate states; receipt alone is not acceptance or assignment release.
- Manager retains required branch selectors, reconciles exact current assignment
  and runtime identity, and owns authorized retirement through supported runtime
  routes. Merely moving the branch-deleting helper to Manager is not sufficient
  while the active assignment still needs that branch handle.
- Physical checkout removal remains separate: it requires an explicit removal
  request and interactive confirmation, never force, recreation, or a bypass.
- Unmanaged `pr cleanup` remains supported with its exact merged-head,
  `main`-target, clean-tree, ancestry, and safe-delete safeguards unchanged.
- Manual Status **Done** still requires verified eligible cleanup. Managed work
  hands remaining cleanup/tracker responsibility to Manager; already Closed/Done
  readback is acknowledged without undoing or reapplying it and is not cleanup
  proof.

## Acceptance

- `CONTRIBUTING.md` explains both ownership paths, accepted evidence handoff,
  selector preservation, supported retirement/removal boundaries, and Done
  rules.
- Emitted `pr cleanup --help` identifies unmanaged applicability and the managed
  preservation exception; parent `pr --help` does not imply universal cleanup.
- Source review and focused help-output verification establish consistency with
  the maintained communication/ship contract and unchanged cleanup safeguards.

## Boundaries

Only repository contribution guidance, cleanup help text, and focused proof are
in scope. No runtime behavior, role detection, cleanup admission protocol, live
cleanup, external skills/configuration edits, or hypothetical-runtime-fix
dependency.
