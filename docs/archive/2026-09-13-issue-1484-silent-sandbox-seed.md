# Issue 1484: Keep sandbox seed manifests off startup stdout

## Outcome

Starting a Jaunder sandbox no longer prints the internal sandbox-profile seed manifest to the operator's stdout. Startup retains useful initialization output and error diagnostics.

## Load-bearing decisions

- Suppress stdout only for the sandbox orchestrator's invocation of `test-support seed-sandbox-profile`.
- Preserve the direct `test-support seed-sandbox-profile` contract: successful direct callers still receive the machine-readable manifest on stdout.
- Preserve all seed-process stderr. A failed seed remains fail-closed and its diagnostic remains visible.
- Preserve `jaunder init` stdout and stderr; this issue does not establish a general silent-startup policy.
- Standard and demo profiles use the same suppression rule. The empty profile remains unchanged because it runs no seed process.
- Do not change profile contents, workspace persistence/reset semantics, process supervision, or signal handling.

## Acceptance

- A sandbox preparation using a non-empty profile does not forward the successful seed manifest to the operator's stdout.
- Direct execution of `test-support seed-sandbox-profile` still emits its JSON manifest after a successful transaction.
- Seed-process stderr remains visible, and a non-zero seed exit still aborts sandbox preparation.
- Existing named, disposable, resume, reset, and empty-profile behavior remains unchanged.
- A focused automated or smoke-test feedback loop demonstrates the leak before the fix and silence after it.

## Boundaries

- No changes to the manifest schema or the profile seeder's direct CLI.
- No suppression of Jaunder initialization output or runtime server output.
- No broader changes to xtask's structured output, logging policy, or child-process abstraction.
- No changes to e2e seeding, production-baseline seeding, or Nix VM behavior.
