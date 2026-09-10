# Issue #1450: Repeatable production deployment baseline harness

## Outcome

Jaunder has a repeatable, opt-in harness that issue #1419 can use to demonstrate
a named release candidate completing the supported deployment, publishing,
upgrade, and recovery workflows under explicit conditions.

The harness lands with a reusable runbook, an initial dated discovery report,
and a dated non-release qualification report for its two-revision acceptance
path. Final release-candidate evidence and milestone finding disposition remain
owned by #1419 after this prerequisite merges.

## Load-bearing decisions

- The baseline is behavioral release-readiness evidence, not a milestone-status
  snapshot, soak test, performance benchmark, or assertion that an unobserved
  defect exists.
- The exercise runs in isolated NixOS virtual machines and deploys the supported
  `packages.jaunder` artifact through the Jaunder NixOS module.
- An external reverse proxy terminates local test TLS. The baseline proves HTTPS
  routing, HTTP-to-HTTPS handling, and secure-cookie behavior; it does not claim
  public DNS or ACME certificate validation.
- SQLite and PostgreSQL receive the complete workflow independently. Neither
  backend is a reduced smoke-test path.
- Discovery deploys one revision on both source backends, creates the canonical
  data, verifies it, survives a Jaunder systemd service restart and whole-VM
  reboot, backs up that same revision/schema, and restores those backups into
  fresh SQLite and PostgreSQL targets running the same revision/schema.
- Acceptance repeats deterministic source creation and restart/reboot checks,
  upgrades each source deployment to the target package, verifies the upgraded
  state, backs up the target revision/schema, and restores those backups into
  fresh SQLite and PostgreSQL targets running the target revision/schema.
- Recovery therefore covers all four backend directions at one compatible
  package/schema level per operation: SQLite to SQLite, SQLite to PostgreSQL,
  PostgreSQL to SQLite, and PostgreSQL to PostgreSQL.
- A pre-upgrade backup is never silently restored into a different schema
  version. If such a compatibility probe is retained, exact-schema rejection is
  an expected compatibility result rather than a failed recovery workflow.
- Restore always targets a separate, freshly initialized environment. The
  exercise never restores over the source deployment or any live data.
- The discovery product source is commit
  `f6b26c2f9e68c82504b4758778ab2bb706e7cca7`.
- Before #1450 completes, acceptance is qualified from that source to the first
  clean upstream commit on the `issue-1450-production-baseline-harness` branch
  that contains the complete harness implementation. The report is explicitly
  non-release evidence.
- Activating a distinct revision proves a package upgrade and preservation of
  data and behavior. The evidence claims a binary change only when source and
  target executable SHA-256 values differ.
- The evidence claims a schema migration only when the recorded schema version
  changes and target startup actually runs the migration path.
- A permanent opt-in host-side `cargo xtask production-baseline` module owns
  revision resolution, Nix builds, VM lifecycle, workflow ordering, evidence
  collection, failure classification, and cleanup.
- The external harness interface has two operations: discovery for one exact
  revision, and acceptance from one exact source revision to one exact target
  revision.
- Nix derivations build packages and VM profiles but never invoke xtask. The
  harness is not added to the mandatory local or CI gate.
- Product revisions must resolve to full commits reachable from
  `jaunder-org/jaunder`; package builds use immutable flake sources pinned to
  those commits, never the xtask process's live checkout.
- The executing xtask, shared Playwright flow, test-support binary, and both
  manifest definitions must come from one clean full harness commit reachable
  from `jaunder-org/jaunder`. Dirty, mixed-revision, or uncommitted harness
  execution is rejected before lifecycle mutation.
- Every report records the harness commit plus the seeded-manifest and
  operation-manifest versions and hashes.
- Package identity records the installable URI, derivation path, realized output
  store path, NAR hash, and executable SHA-256. Runtime identity is the running
  service's executable and systemd `ExecStart` resolving inside that recorded
  output store path. Cargo package version is provenance only because current
  artifacts all report `0.1.0`.
- The existing typed-storage `demo` profile remains the fixture foundation and
  is extended rather than replaced. It supports both storage backends and adds a
  versioned seeded-record manifest for the missing HTML, scheduled, non-public,
  and Media-reference states while retaining its existing Users and rich
  Markdown/Org corpus.
- The shared Playwright flow owns a separate versioned operation manifest for
  the browser session, App Password, and records created through web and AtomPub
  operations. Seeded and flow-created identities use disjoint stable names.
- The read-only verifier consumes both manifests and compares canonical domain
  identities, bodies, states, visibility, and Media hashes; backend-assigned
  numeric IDs are not cross-backend identity authorities.
- Test-support tooling may exist in the isolated test environment, but it
  remains outside the production Jaunder package and service closure.
- The representative data includes multiple Users, browser and App Password
  authentication, Markdown/Org/HTML Posts, draft/published/scheduled and
  non-public states, Media, and relationships that exercise public and
  restricted visibility.
- The ordinary backend-by-browser E2E suite and the production-baseline harness
  execute the same Playwright create-and-read-only-verify flow.
- The host harness alone owns deployment mechanics: service restart, VM reboot,
  package upgrade, backup transfer, and restore. Ordinary E2E does not duplicate
  those mechanics.
- Chromium is the production-baseline browser. The ordinary E2E matrix continues
  to provide supported browser parity for the shared behavior flow.
- Protocol checks exercise AtomPub discovery, Collection and Member operations,
  Media operations, and App Password authentication through the external HTTPS
  endpoint.
- Public checks exercise Atom, RSS, and JSON Feed representations through the
  external HTTPS endpoint.
- Every active deployment and restore target is reached through one stable
  external HTTPS origin. The proxy cuts that origin over to the selected
  isolated target, so the unchanged origin-scoped browser cookie and unchanged
  App Password—not a reinjected or newly minted substitute—must authenticate
  after service restart, VM reboot, binary upgrade, and every restore direction.
- Recovery separately permits a fresh login and fresh App Password
  mint/use/revoke cycle.
- Every ordinary E2E execution covers both creation and the read-only verifier,
  so no baseline-only verification branch lacks routine behavior coverage.
- The reusable runbook lives at `docs/production-baseline.md`. Each completed
  run retains exactly `summary.json` and its generated `summary.md` under a
  dated `docs/evidence/production-baseline/` directory.
- `docs/production-baseline.schema.json` is the versioned JSON Schema for
  `summary.json`. JSON is the structured evidence authority; Markdown is
  rendered from it and mechanically checked for agreement.
- The JSON schema requires the harness commit and manifest versions/hashes;
  source/target commits; installable and immutable package identities; backup
  format and schema versions; backend and restore direction; fixed check
  identifiers; lifecycle actions; outcomes; durations; sanitized hashes;
  explicit gaps; finding dispositions; and product/harness/infrastructure
  failure classifications.
- Generated credentials, cookies, keys, and sensitive fixture values are
  registered as run-specific canaries. Every durable outcome—passing or
  failed—rejects retained paths outside the two-file allowlist, prohibited file
  types, and canaries or forbidden fields in every retained textual artifact.
- If sanitization, allowlist validation, or evidence collection fails, no
  durable report is published outside the access-restricted gitignored run
  workspace.
- Raw databases, backups, Media, private keys, credentials, and unsanitized
  journals may exist only in that workspace for execution or failure diagnosis;
  they are never copied into durable evidence.
- Failures are classified as product, harness, or infrastructure failures.
  Evidence collection failure cannot produce a passing result.
- A finding blocks release readiness when a required workflow fails, data is
  corrupted, security/privacy/data-loss risk is unacceptable, or no acceptable
  operational workaround exists. Every finding is explicitly blocking, accepted,
  or deferred with rationale.
- The final acceptance result applies only to its named source, target, VM
  topology, storage backends, proxy configuration, browser, protocol flows, and
  recorded data set.

## Acceptance

- Discovery resolves and records the clean harness identity and one requested
  product revision, creates new run workspace and source disks for both
  backends, completes deployment, shared-flow creation/verification, restart,
  reboot, same-revision backup, all four same-revision restore directions, and
  dated evidence without claiming an upgrade.
- Acceptance resolves and records the same clean harness identity plus distinct
  source and target product commits; it rejects inputs that resolve to the same
  full commit. A controlled two-revision qualification creates fresh source
  disks, repeats source deployment/creation/restart/reboot, upgrades both
  persisted source backends, proves target runtime identity, repeats read-only
  verification, creates target-schema backups, completes all four target-package
  restore directions, and emits explicitly non-release evidence.
- Source disks and run workspaces are never reused across runs. Interrupted runs
  may retain diagnostics for explicit cleanup but cannot resume into passing
  durable evidence.
- Both operations fail closed on invalid or unreachable revisions, dirty or
  mismatched harness identity, reused state, concurrent ownership, failed
  lifecycle actions, failed assertions, or incomplete evidence.
- SQLite and PostgreSQL source deployments each initialize through the
  production module, serve only through the HTTPS proxy surface, and pass the
  shared browser/protocol flow.
- The expanded demo fixture produces equivalent representative state on SQLite
  and PostgreSQL without adding test-support to the production package closure.
- The shared Playwright flow runs in the normal E2E backend/browser matrix and
  covers HTML alongside the existing Markdown and Org authoring behavior.
- The shared flow proves login, Post creation and visibility, scheduling, Media
  creation and serving, AtomPub operations, and all three Syndication Feed
  formats before lifecycle transitions.
- After systemd restart and VM reboot, pre-existing public and restricted
  content, Media, browser authentication, and App Password authentication remain
  usable on both source backends.
- The acceptance run proves that the target package, not the source package,
  owns the running service after upgrade and that all shared-flow read-only
  checks still pass.
- Every backup record includes its archive hash plus the manifest
  `format_version` and observed source and target schema versions, without
  exposing backup contents in durable evidence.
- Discovery backups are produced and restored by the discovery package at its
  schema version; qualification backups are produced after upgrade and restored
  by the target package at its schema version. Each operation restores SQLite
  and PostgreSQL source backups into fresh SQLite and PostgreSQL targets,
  yielding all four directions.
- Unsupported format or schema compatibility is rejected before target mutation
  and is never reported as successful recovery.
- After every restore, the proxy moves the stable HTTPS origin to the restored
  target and the shared read-only verifier confirms canonical representative
  identities, source bodies, rendered/public visibility, Media content hashes,
  all Syndication Feed representations, the unchanged browser session, and the
  unchanged App Password.
- Every restored environment also permits fresh browser authentication and an
  App Password mint/use/revoke cycle.
- The runbook explains prerequisites, the discovery and acceptance commands,
  source/target selection, expected artifacts, evidence interpretation, safe
  cleanup, and how to distinguish product failures from harness or
  infrastructure failures.
- Each dated `summary.json` validates against
  `docs/production-baseline.schema.json`; `summary.md` is generated from that
  JSON and mechanically agrees on identities, checks, outcomes, gaps, failures,
  and finding dispositions.
- Every passing or failed durable result retains only the two allowlisted report
  files, rejects prohibited file types and fields, and scans every retained
  textual byte for the registered run-specific canaries before evidence is
  published.
- The initial discovery run is recorded against
  `f6b26c2f9e68c82504b4758778ab2bb706e7cca7`.
- A non-release qualification report records acceptance from that source to the
  defined clean harness implementation commit before #1450 completes.
- The landed acceptance interface remains executable with any validated distinct
  source/target pair, but selecting and accepting Milestone #22's
  release-candidate commit remains #1419 work.
- Every defect or operational blocker observed by the discovery run is linked to
  an individual Milestone #22 issue with reproduction evidence and a preliminary
  blocking, accepted, or deferred disposition for #1419 to confirm.

## Boundaries

- No live instance, live credential, production database, production backup, or
  operator Media is accessed or mutated.
- No public certificate authority, public DNS, or internet availability claim is
  made.
- No fixed soak duration, throughput target, large-data performance target, or
  capacity conclusion is introduced; performance harness work remains #1434.
- No new release channel, tag policy, semantic-version compatibility rule,
  downgrade path, or cross-schema restore policy is created.
- Backup compatibility continues to follow ADR-0174: format and exact schema
  version are compatibility authorities; package version and schema checksum
  remain provenance.
- The work does not replace the existing E2E matrix, backup integration tests,
  or AtomPub integration tests, and it does not move deployment lifecycle
  mechanics into Playwright.
- The work does not introduce Crux, inbound federation, native clients,
  speculative infrastructure hardening, or unrelated UI remediation.
- Final release-candidate evidence, final finding disposition, and milestone
  closure remain outside this prerequisite and are owned by #1419.
