# #1820 Seed bridge writer ownership

**Status:** Approved implementation scope; planning baseline only.

## Approval provenance

The Direct User approved this exact scope with **“1820: approved”**, answering
the reviewed `.xtask/etxtbsy-1820/repair-proposal.md`. The approval was relayed
by the Manager and is recorded as qualified direct-user approval, not Manager
approval. Scope is limited to bridge construction and focused helper regression
tests described below. It does not authorize #1821 or the proposed alternatives.
The approved input is the reviewed proposal; the prior working branch baseline
was `ecb80ed9fdee424959832060c381a03a6eb800eb`.

## Problem and evidence boundary

The unchanged production-baseline fixture reproduced `ETXTBSY`; the no-spawner
control and qualified failure-time trace make same-process sibling inheritance
of a bridge writer the leading explanation for that diagnostic schedule. They do
not prove the precise descriptor-copy/retention instant or identify a historical
incident caller. No further stress campaign is required for this bounded repair.

## Construction contract

At
`xtask/src/production_baseline_lifecycle.rs::BaselineLifecycle::seed_process`,
change bridge construction only:

- Preserve admitted canonical Nix-store Node validation, private-path
  validation, regular-file/symlink checks, and final `0700` restriction.
- Send the unchanged bridge source to that exact admitted Node child over piped
  stdin, not arguments. No shell, PATH lookup, or launcher/API redesign;
  preserve the shebang and same single executable path returned to current
  callers.
- The child exclusively creates the allocated path with restrictive permissions,
  writes the complete input and closes it. The Rust parent never owns a writable
  bridge descriptor.
- The parent sends all source and closes stdin before waiting, including on
  write failure. Writer completion requires EOF, a successful full write, and
  close. Do not leave an undrained output pipe while feeding stdin, and reap the
  child on every post-spawn path.
- Return the path only after successful input transfer, successful child
  completion, and existing protection checks. On any failure, do not
  return/publish the path. After child termination, remove only a file this
  child successfully created. An exclusive-create failure must never remove a
  pre-existing path. Do not add content readback.

These exclusive creation, writer lifecycle, and failure cleanup requirements are
new behavior, not claims about existing guarantees.

## Compatibility and explicit exclusions

Keep existing admitted-Node/private-path/regular-file/symlink/final-`0700`
safeguards, bridge source and shebang, TypeScript `execFileSync` use of
`JAUNDER_E2E_SEED_PROCESS`, no-PATH-Node direct invocation intent, and argument,
environment, guest-identity, output, status, and error behavior. Current Rust
callers continue receiving the same single executable path.

Do not introduce errno-specific retry, sleep, serialization, error suppression,
launcher framework, explicit Node-plus-script caller/API redesign, or the guest
accept/join cleanup bundle. #1821 remains unapproved and out of scope.

## Focused regression proof

At the actual helper seam, coordinate a sibling plain-`Command` spawn during the
writer lifetime and before publication. Demonstrate that the old
parent-owned-writer arrangement permits the sibling inheritance schedule to make
an unchanged direct launch fail with `ETXTBSY`, whereas child-owned writing
prevents the parent's sibling from inheriting the writer and the helper returns
only after writer-child exit, after which the same direct launch succeeds. Do
not keep an arbitrary writer open on the published script. Retain the
no-PATH-Node invocation and existing argument/environment/guest
identity/output/status/error/private-file assertions; add focused failure-path
checks for no path return and safe cleanup as needed. This proves prevention for
the reproduced schedule, not every historical incident.

## Outline assessment

No separate outline is required: this is a focused construction/test change with
a specific helper seam, lifecycle ordering, failure contract, compatibility
constraints, and an explicit regression oracle. The reviewed proposal and this
spec provide the proportionate implementation plan; no new interview or approval
is needed while these inputs and scope remain unchanged.

## Delivery lifecycle

Implement against this spec. Keep it current through conformance review. Before
implementation, commit this approved spec as a normal tracked planning commit
after Lead approval of the exact staged diff/tree. At final cleanup, remove the
completed spec in a cleanup commit as required by the repository artifact
lifecycle; promote durable rationale only if implementation establishes an
enduring contract not already captured elsewhere.
