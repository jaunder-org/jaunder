# Issue 1847 — readable publication of the E2E capture archive

## Outcome

The completed outer `capture-<lane>.tar.gz` artifact is published with mode
`0644`, so the host process can read it from either a successful output or a
retained failed-build output. The archived capture contents and private
tar-member metadata remain unchanged.

## Load-bearing decisions

- Approval: human decision `ask_user 2591f131-9473-482f-b47a-33e00b428b9f`,
  “Approve small publication fix.”
- Scope is only the outer archive's final publication mode and the existing
  deterministic `archive_capture` regression test.
- Keep exclusive creation at mode `0600` while the archive is assembled. After
  the writer closes successfully, explicitly set the completed file to `0644`;
  do not rely on `os.open` mode or process umask to establish the final mode. An
  assembly failure must not expose a partial archive as world-readable.
- Preserve archive bytes, member modes (`0700` directories, `0600` files),
  normalized member ownership headers, integrity verification, and filtering
  behavior.
- This repairs the source-backed publication-contract mismatch; it does not
  prove which side caused run 40777996562's historical `PermissionDenied`.

## Acceptance

- Existing focused Python test for `archive_capture` asserts final outer mode
  `0644` under the ordinary test environment and continues checking the exact
  member modes/ownership and payload.
- Focused `nix/test_e2e_capture.py` unit tests pass. No VM, browser, E2E, CI, or
  broad gate is run.
- Review confirms no changes beyond `nix/e2e_capture.py` and its existing test,
  plus this temporary spec; private member metadata and filters are unchanged.

## Boundaries

- No generic permission/chown framework, chmod of other artifacts,
  instrumentation, retry/re-enqueue, snapshot changes, push/head update, land,
  or queue action.
- Preserve the pre-change queue failure evidence and PR1847 visual failure as
  separate unresolved history. This follow-up requires independent review and a
  new review-delta packet before any PR update; prior PR1847 merge approval does
  not authorize landing this behavior.
