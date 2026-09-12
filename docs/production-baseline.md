# Production baseline harness

`cargo xtask production-baseline` is an opt-in, host-side qualification harness.
It deploys immutable `packages.jaunder` revisions through the supported NixOS
module; it is not part of `cargo xtask check`, pre-push, CI, or
`nix flake check`.

## Prerequisites and commands

Run from a clean checkout whose `origin` is `jaunder-org/jaunder`; the executing
xtask, shared Playwright flow, test-support binary, and both manifests must all
identify the same clean harness commit. The host needs the repository's Nix
development environment, Nix virtualization support, Chromium/Playwright
dependencies, and free local ports `8080` and `8443`. Do not run alongside
another baseline command: the command holds one host lease. Ordinary host
quiescence is not required; enough resources for reliable VM execution are.

First commit the clean harness revision, enter the repository devShell with
`nix develop`, then run the following from that exact clean checkout before
invoking the harness:

```bash
devtool run -- cargo build -p test-support
```

This builds the matching `test-support` binary that provenance verification
requires. A stale binary or a binary built from a dirty or different revision
fails closed; do not bypass that failure or treat it as qualification evidence.

The required pre-milestone source is `520034ed6f778a854cd5f8708419c1b2b700ad9f`.
Run its discovery with:

```bash
devtool run -- cargo xtask production-baseline discover --revision 520034ed6f778a854cd5f8708419c1b2b700ad9f
```

The earlier `f6b26c2f9e68c82504b4758778ab2bb706e7cca7` evidence is historical
harness-development evidence, not the pre-milestone baseline: it already
contains Milestone 22 issue #1422's merge.

Final acceptance is a later source-to-clean-release-candidate run. Use distinct,
resolved full commits only, with the selected clean Milestone 22
release-candidate commit as target:

```bash
devtool run -- cargo xtask production-baseline accept --source 520034ed6f778a854cd5f8708419c1b2b700ad9f --target <clean-release-candidate-commit>
```

Acceptance qualification is explicitly **non-release** evidence. Issue #1419
selects and qualifies a release candidate later.

## Supported production deployment and operator handoff

The preferred supported deployment is the exported `nixosModules.jaunder` module
behind an externally managed TLS-terminating reverse proxy. Deploy
`packages.jaunder`, set the module's production, bind, and database options
explicitly, and let the proxy own TLS. The module owns the `jaunder` account,
durable state directory, initialization, service start, and on-failure restart;
the operator owns the reverse proxy, TLS certificates, process configuration,
and service-manager secret injection.

Manual single-binary deployment is also supported: the operator supplies the
same process configuration and manages initialization, `jaunder serve`, restart,
and replacement of the binary through the service manager. PostgreSQL bootstrap
is a supported one-time administrative alternative: an experienced administrator
runs `jaunder create-pg-db` with explicit `--bootstrap-db`, `--app-db`, and
application-role password inputs before `jaunder init`; steady-state PostgreSQL
uses `JAUNDER_DB` plus `JAUNDER_DB_PASSWORD_FILE` (preferred) or
`JAUNDER_DB_PASSWORD`. Do not put the password in a database URL, command line,
persisted configuration, or this runbook. CLI flags override matching
`JAUNDER_*` variables, which override documented defaults; production operators
set `JAUNDER_ENV=prod` explicitly.

For upgrades, the operator replaces the deployed `packages.jaunder` binary or
updates the NixOS configuration, then lets the supported service lifecycle start
the new `jaunder serve` process against the existing database. The NixOS module
continues to own its declared init/start/restart lifecycle; manual deployments
continue to own theirs. This handoff adds no alternate provisioning or
configuration model.

Jaunder owns scheduled backup configuration through its persisted
`BackupConfig`. Configuring a destination enables its scheduled backup worker;
configure its six-field cron schedule, retention count of at least one (default
seven), and directory or archive mode there. For an immediate backup, use
`jaunder backup --path <local-backup-destination>`. The operator owns local
destination permissions and capacity, external copying, and encryption. Recover
only onto a fresh, isolated, empty target: initialize that target with its
isolated `--storage-path` and `--db`, then run
`jaunder restore --storage-path <isolated-state> --db <isolated-db> <backup-path>`.
Never restore over a live deployment or production data. Backup format and exact
schema version are compatibility authorities, and a rejected format or schema
combination is not recovery success.

Remote backup transport, backup encryption, public DNS, public CA issuance,
internet availability, performance, capacity, and soak behavior are unsupported
by this handoff and are not qualification successes.

## Topology and immutable identity

The host xtask resolves upstream commits to immutable flake references and alone
invokes Nix and manages lifecycle. Nix never invokes xtask. Each operation
initializes fresh persistent SQLite and PostgreSQL source VMs. The harness
supplies each VM's immutable package through the qualification-only internal
`productionBaselineVm` construction seam; ADR-0142's supported module options
remain unchanged. Source state is never live operator data. An external Caddy
proxy terminates local TLS and keeps one stable `https://localhost:8443`
browser/protocol origin while it switches among source and fresh restore
targets. HTTP redirects to that origin.

Discovery restarts and reboots each source, then makes one same-schema backup
and restores every SQLite/PostgreSQL source-to-target direction. Acceptance
repeats that work, declaratively activates its target package, proves the target
runtime identity, and restores target-schema backups in all four directions.
Backup format and exact schema version are compatibility authorities; a
rejection is not recovery success.

Check durations are diagnostic metadata, not performance measurements. Resource
contention is an infrastructure failure requiring a rerun, never performance,
capacity, availability, or soak evidence.

## Evidence

A completed run attempts one atomic publication at:

```
docs/evidence/production-baseline/YYYY-MM-DD-<operation>-<source12>-<target12-or-none>-<harness12>/
```

The identity-derived dated name is collision-safe: an existing destination is
never overwritten. A retained destination contains exactly `summary.json` and
generated `summary.md`. JSON validates against `production-baseline.schema.json`
and is authoritative; Markdown is mechanically derived and checked for parity.
Command output reports only the safe destination and identity/check counts.

Reports record harness and manifest identities, source/target,
package/runtime/backup identities, lifecycle and fixed check durations,
outcomes, gaps, findings, and failure classes. A binary change is claimed only
when executable hashes differ; a schema migration only when observed schema
versions differ and startup migrates.

The successful pre-milestone discovery report is
[`2026-09-12-discover-520034ed6f77-none-78d201e6969e`](evidence/production-baseline/2026-09-12-discover-520034ed6f77-none-78d201e6969e/summary.md).
It records source `520034ed6f778a854cd5f8708419c1b2b700ad9f` and harness
`78d201e6969ed17b8a54b87d3de24be6afa1f5ee`: all 11 applicable checks passed; the
upgrade check was skipped by discovery definition; there were no gaps, findings,
or failure classes. It demonstrates only that source's discovery topology and
checks, not upgrade behavior or any untested limit.

Every generated browser/session value, App Password, fixed password, sensitive
seed value, and Caddy private-key bytes is a run canary. Both passing and failed
durable outcomes undergo schema validation, forbidden-field and canary scans,
prohibited-file-type/symlink rejection, and the exact two-file allowlist. If
evidence collection, rendering, sanitization, or a scan fails, nothing is
published.

Failures are classified as **product**, **harness**, or **infrastructure**. An
interrupted or incomplete graph is non-passing and cannot become passing
evidence. Raw databases, backups, Media, proxy keys, credentials, Playwright
state, and logs remain only under the restricted gitignored
`.xtask/production-baseline/` workspace.

## Reruns and cleanup

Every invocation creates fresh source disks and a fresh workspace. Never resume
an interrupted workspace or reuse source disks; rerun the exact command from
clean state. Preserve the restricted workspace only long enough to diagnose a
failed run, then remove only the exact `run-<pid>-<nonce>` directory printed or
identified beneath `.xtask/production-baseline/`; do not delete its parent, a
broad glob, an evidence destination, or any other `.xtask` path. Published
evidence is immutable; no cleanup command may remove or replace it.

This harness demonstrates only the named revisions, local VM topology, storage
backends, stable HTTPS routing, browser/protocol flows, and representative data.
It makes no public CA, public DNS, internet availability, performance, soak,
capacity, release, or release-candidate claim.
