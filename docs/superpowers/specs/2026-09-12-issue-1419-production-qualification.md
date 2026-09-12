# Production Deployment and Recovery Qualification

Issue: #1419

## Outcome

Jaunder has durable evidence showing what the repository state immediately
before Milestone 22 could do, and whether the eventual Milestone 22 release
candidate can be deployed, upgraded, backed up, and recovered through the
supported production topology without losing representative application
behavior. The result is qualification evidence and an operator handoff, not a
performance, capacity, soak, availability, or release claim.

## Load-bearing decisions

- The pre-milestone source revision is
  `520034ed6f778a854cd5f8708419c1b2b700ad9f`, the first parent of the first
  currently recorded Milestone 22 merge, PR #1423 for issue #1422.
- The target is the eventual clean Milestone 22 release-candidate commit.
  Source, target, and executing harness identities are resolved to immutable
  full commits; source and target must differ.
- Qualification uses the existing opt-in `cargo xtask production-baseline`
  harness. It does not create a second provisioning, deployment, backup, or
  recovery system.
- Discovery qualifies the source revision by itself. Final acceptance upgrades
  fresh persisted source instances to the target and qualifies the target's
  backup and recovery behavior.
- The representative topology is the exported NixOS module behind an external
  local TLS proxy, with fresh isolated SQLite and PostgreSQL instances. It
  exercises both backends and all four source-to-target restore directions.
- Representative behavior covers browser login, browser-session continuity, App
  Password authentication, web and AtomPub Post operations, Media, public pages,
  and RSS, Atom, and JSON Syndication Feeds.
- Restart, reboot, upgrade, and restore checks prove that representative
  content, visibility, Media hashes, browser sessions, and App Passwords remain
  usable. Every restored environment also proves fresh login and App Password
  mint/use/revoke behavior.
- Backup format and exact schema version are compatibility authorities. A
  rejected format or schema combination is not reported as successful recovery.
- Check durations are diagnostic metadata only. No duration is interpreted as a
  performance, capacity, availability, or soak threshold. The host need not be
  otherwise quiescent: only the harness's exclusive lease, required free ports,
  and sufficient resources for reliable VM execution are required. Contention
  may produce an infrastructure failure and rerun, never a performance finding.
- Findings are cross-checked against every applicable existing Milestone 22
  issue, including the operator's manually filed aesthetic and behavior bugs.
  Existing matches are linked rather than duplicated; only a materially distinct
  defect or operational blocker receives a new issue.
- Each new finding uses the repository's native issue type and includes
  reproduction evidence. Behavior bugs use the repository's P1 convention.
  Findings are classified separately as release-blocking, accepted limitations,
  or deferred improvements.
- Source state is synthetic and isolated. Qualification never accesses live
  credentials or production data, interrupts a live instance, overwrites an
  existing target, or restores over production.
- Durable evidence contains only the schema-validated `summary.json` and its
  generated `summary.md`. Raw databases, backups, Media, credentials, browser
  state, proxy keys, and logs remain in the restricted gitignored workspace.
- `docs/production-baseline.md` is the operator handoff. It presents the NixOS
  module plus external TLS proxy as the preferred deployment path and identifies
  manual single-binary deployment and PostgreSQL bootstrap as supported
  alternatives without expanding them into a new provisioning system.

## Acceptance

- A discovery report against `520034ed6f778a854cd5f8708419c1b2b700ad9f` records
  every attempted check, outcome, gap, finding, failure class, immutable
  identity, and observed backup format/schema identity.
- Discovery either completes deployment, representative application checks,
  restart, reboot, same-revision backup, and all four restore directions, or
  records the exact incomplete or failed checks without claiming them green.
- A final acceptance report uses the same source and the selected clean
  Milestone 22 release-candidate target, proves the target binary owns the
  upgraded services, and repeats the representative read-only and recovery
  checks against the target.
- The final handoff compares source and target outcomes, states which observed
  pre-milestone failures were eliminated, and lists every remaining limitation
  without inferring causation from unrelated changes.
- Each product defect or operational blocker is linked to exactly one applicable
  Milestone 22 issue after an explicit duplicate search; reused issues receive
  the new evidence rather than a parallel report.
- Restore evidence demonstrates usable canonical Posts, visibility, Media,
  Syndication Feeds, retained authentication, and fresh authentication on
  isolated targets—not merely successful archive creation.
- Both passing and failed durable reports pass the evidence schema, two-file
  allowlist, forbidden-field checks, symlink/file-type rejection, and run-canary
  scans before publication.
- `docs/production-baseline.md` agrees with the selected pre-milestone source,
  explains the earlier `f6b26c2…` evidence as historical harness-development
  evidence, gives actionable setup, TLS, service lifecycle, upgrade, backup, and
  isolated recovery steps, and names unsupported remote backup transport,
  encryption, public DNS/CA, internet availability, performance, capacity, and
  soak behavior as limits rather than successes.

## Boundaries

- Qualification does not authorize a release or close Milestone 22; those remain
  separate decisions after reviewing the final evidence and linked findings.
- It does not run against a live deployment, real user data, production secrets,
  public infrastructure, or destructive recovery targets.
- It does not add performance/load testing, a fixed soak duration, remote backup
  storage, encryption, provisioning automation, or speculative hardening.
- It does not fix findings inline. Distinct defects and blockers are handled by
  their own issues and development cycles so their scope and evidence remain
  reviewable.
- The earlier `f6b26c2f9e68c82504b4758778ab2bb706e7cca7` harness-development
  evidence remains historical, but it is not presented as the pre-milestone
  source because it already contains Milestone 22 issue #1422's merge.
