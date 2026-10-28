# Approved contract: RUSTSEC-2026-0414 remediation

Approval: human via Manager, “Approve the dependency fix”, ask_user
`a74f366a-414e-4953-8a1e-79a3349beec1`. 0.13.5 is an upstream release, not a
local patch. Governing intake packet:
`/tmp/jaunder-rustsec-2026-0414-approval-packet.md`.

## Scope

In `/home/mdorman/src/jaunder/herdsman-work-6f713041`, change the root workspace
`pulldown-cmark` requirement from 0.13.4 to 0.13.5 and only the necessary root,
tools and xtask lock entries. No unrelated dependency upgrades, #1816 changes,
advisory ignores, bypasses, or policy changes. Deliver one standalone
dependency-only implementation commit after Lead accepts the tree; share its
full SHA with the #1816 owner. No tracker mutation or PR is authorized at this
implementation boundary.

## Acceptance

Inspect the full diff; prove all three graphs resolve the upstream 0.13.5
release with locked Cargo commands. Run focused host advisory-policy and
Markdown consumer checks plus auxiliary consumer compilation. Lead dispatches
broad gates after the writer settles. Ordinary hooks remain mandatory; inspect
any hook mutation and reapprove/revalidate the intended tree before retry. Stop
before staging or committing for independent review and Lead acceptance.

## History prerequisite

CONTRIBUTING and dev-cycle require the approved spec to be committed separately
before implementation and removed in a final cleanup commit after conformance.
The assigned no-commit/no-spec-in-fix boundary withheld that preservation
commit. This session copy is not committed history and must not be represented
as such. Lead should resolve preservation sequencing before the implementation
commit: preserve the approved contract in a separate spec-only history commit,
keep the four-file implementation commit separate, and delete the spec in a
later cleanup commit. Do not mix this document into the dependency fix or
silently claim the before-implementation requirement was met.
