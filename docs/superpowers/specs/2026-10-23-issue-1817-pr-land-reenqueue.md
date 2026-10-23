# Proposal: simple explicit `pr land --reenqueue`

## Decision requested

Approve a narrowly scoped extension to existing `cargo xtask pr land <PR>`: add
`--reenqueue` to permit one enqueue after the observer detects a prior
merge-queue ejection. This is intentional operator opt-in, not a
security/provenance framework. The default refusal remains. This new tooling
capability is not authorized by prior PR1813 land approval; no implementation or
operational recovery is authorized by this proposal.

## Contract

Without the flag, preserve current behavior. With it, preserve existing PR
head-divergence guard, current required-check validation, GitHub queue rules,
and fail-closed behavior. Refuse if already queued or auto-merge armed. Make
exactly one existing armer call in this invocation; do not auto-repeat or fall
back to ordinary rearming on uncertainty. A newly failed attempt is terminal.

The only observer accommodation: retain the identity of the already-observed
ejection in invocation-local state after arming, so normal land observation does
not mistake that same historical run for the new attempt. Ignore only that run
for this invocation. Fresh-run visibility may use the existing observation
deadline; a distinct fresh failure terminates. Reuse the existing
observer/state/reporting. No operator-supplied run/head fields, custom proof
attestation, provenance framework, persistent registry, or global cap across
separate deliberate CLI invocations.

This intentionally refines ADR-0087 decision 4's refusal of re-enqueue after
ejection; `pr watch` remains observational. Future implementation scope:
existing land/CLI seam, CLI help, relevant `CONTRIBUTING.md` land guidance,
focused ADR-0087 update, and regression coverage for default refusal, one
enqueue, old-versus-fresh run handling, queued/armed refusal, terminal fresh
failure, no rearm on uncertainty, and unchanged guards. If existing run identity
cannot distinguish old from fresh, return that narrow seam blocker; do not
expand this contract into a framework.

**Approval ask:** approve this exact `--reenqueue` scope for implementation on a
dedicated project branch arranged by Manager after approval. PR1813 approved
head/tree remain untouched. No implementation, tests, commands, network, CI, PR
mutation, or source/index changes were performed for this proposal.

## Approval provenance

Current Manager assignment relays direct User: “approved scope for 1817”. The
User reviewed and approved this simple spec v1. No renewed outline or design
gate is required; this copy records the approved scope for planning and
implementation.
