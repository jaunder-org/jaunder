# Issue #1747: guest state uses the service identity

## Outcome

Ordinary production-baseline guest seeding and Jaunder CLI state mutations run
as the canonical `jaunder.service` identity. The service can create Media and
refresh a Syndication Feed after restart without a root-owned publisher lock.

## Load-bearing decisions

- Correct the qualification harness, not product file permissions or the
  publisher-lock contract. Do not repair ownership after creating root-owned
  application state; create that state under the service identity.
- Preserve the selected environment, shell argument quoting, command exit
  framing, backend storage selectors, and specialized fresh-restore behavior.
- Retain SQLite and PostgreSQL parity, the existing fixture population, ordinary
  format-1 admission, source selection, and Nix executable/helper provenance
  checks. Styling qualification is not ordinary qualification proof.
- Reuse supported guest command execution; no new helper framework or security
  bypass. Host execution of xtask remains outside Nix derivations.

## Acceptance

- Focused host regressions drive the actual seed and ordinary CLI command
  constructors and reject execution as root. They verify preserved environment,
  difficult argument quoting, storage selection and nonzero exit framing.
- Regression coverage distinguishes ordinary commands from specialized
  fresh-restore commands and verifies the latter's existing semantics.
- An admitted, Nix-provenanced ordinary discovery executes on both SQLite and
  PostgreSQL past Media creation, service restart, and the read-only Syndication
  Feed checks. Confirm guest identity and publisher-lock ownership match the
  service account using sanitized existing guest-control observations. Report
  tested harness/product/helper revisions, command, outcome and report paths.
  Incompatible fixture admission is a blocker, not a successful identity proof.
- Run the applicable verification ladder from `CONTRIBUTING.md`: focused
  regressions first, enforced precommit and prepush, and required CI proof. Do
  not waive hooks or copy the unmerged #1760 security correction.
- The spec, scoped harness correction and regression tests are source
  deliverables. Logs, guest disks and browser captures stay in ignored run
  storage; do not commit credentials or execution reports. Delete the completed
  spec after conformance review, retaining its history.

## Boundaries

No changes to application authorization, service sandboxing, file modes, backup
compatibility policy, fixture population, or backend behavior. Historical
prototypes are evidence to inspect, not commits to cherry-pick. This issue does
not select a #1419 candidate, run #1419 acceptance, or operate production. Spec
soundness review and approval precede implementation. Real VM execution requires
coordinated ownership of the actual harness/ports and sufficient CPU/RAM;
unrelated workloads need not be globally serialized or quiescent.
