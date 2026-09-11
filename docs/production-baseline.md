# Production baseline harness

`cargo xtask production-baseline` is an opt-in, host-side qualification harness.
It deploys immutable `packages.jaunder` revisions through the supported NixOS
module; it is not part of `cargo xtask check`, pre-push, CI, or
`nix flake check`.

## Prerequisites

Run from a clean checkout whose `origin` is `jaunder-org/jaunder`; the executing
xtask, shared Playwright flow, test-support binary, and both manifests must all
identify the same clean harness commit. The host needs the repository's Nix
development environment, Nix virtualization support, Chromium/Playwright
dependencies, and free local ports `8080` and `8443`. Do not run alongside
another baseline command: the command holds one host lease.

Run discovery against the required product source:

```bash
cargo xtask production-baseline discover --revision f6b26c2f9e68c82504b4758778ab2bb706e7cca7
```

Run acceptance only with distinct, resolved full commits. Before issue #1450
completes, its target is the first pushed clean complete harness commit on
`issue-1450-production-baseline-harness`:

```bash
cargo xtask production-baseline accept --source f6b26c2f9e68c82504b4758778ab2bb706e7cca7 --target <pushed-clean-harness-commit>
```

Acceptance qualification is explicitly **non-release** evidence. Issue #1419
selects and qualifies a release candidate later.

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
