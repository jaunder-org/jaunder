# Production baseline harness

`cargo xtask production-baseline` is an opt-in, host-side qualification harness.
It deploys immutable `packages.jaunder` revisions through the supported NixOS
module; it is not part of `cargo xtask check`, pre-push, CI, or
`nix flake check`.

## Prerequisites

Run `cargo xtask` from the repository's Nix development environment and a clean
checkout whose `origin` is `jaunder-org/jaunder`. Ordinary discovery/acceptance
also require the harness and product commits to be available upstream.
Qualification selects its fixture-owned harness graph from the admitted source:
the actual executing xtask must match its Nix output, and browser inputs, Node,
test-support and VM construction are bound through the same graph. Identical
outputs may be reused across unrelated Git commits; native Cargo builds and
copied executables cannot claim Nix provenance. A `JAUNDER_TEST_SUPPORT_BIN`
override is accepted only if it resolves to the graph's selected helper.

The host needs Nix virtualization support and free local ports `8080` and
`8443`. The graph supplies pinned Chromium/Playwright dependencies. Do not run
alongside another baseline command: the command holds one host lease.

Run discovery against the source revision selected for the qualification:

```bash
cargo xtask production-baseline discover --revision <source-commit>
```

Run acceptance only with distinct, resolved full commits available upstream:

```bash
cargo xtask production-baseline accept --source <source-commit> --target <target-commit>
```

Acceptance qualification is explicitly **non-release** evidence. Issue #1419
selects and qualifies a release candidate later.

## Ordinary fixture applicability

Only the resolved source `520034ed6f778a854cd5f8708419c1b2b700ad9f` selects
`qualification-harness-legacy-1419`. Acceptance never selects from its target,
and the selection remains fixed through upgrade and every restore. This graph
uses current host/browser/VM tooling with the immutable historical helper
`78d201e6969ed17b8a54b87d3de24be6afa1f5ee` and its own dependency locks. Its
executable, seed authority and both guest helper bindings agree; the complete
historical authority-file digest is checked before lifecycle work.

The historical fixture retains all 73 native Posts, seeded text Media, uploaded
PNG, authentication and all three Syndication Feed formats. Its exact v1 wire
and historical composer/Post-create contracts are explicit, not inferred from
replies. All other ordinary sources and live e2e retain strict modern v2,
five-Media and Rendered Title obligations. Incompatible upgrade targets fail;
there is no adapter, retry, selector option or ambient fixture flag.

Historical discovery evidence remains schema1. New evidence attributes the
actual selected graph and current executing harness/verifier, not historical
execution. This applicability correction alone does not establish helper Nix
output admission, backend/browser compatibility or a suitable distinct format-1
target; those require separately executed proofs before positive qualification.

## Separate styling qualification

`cargo xtask production-baseline qualify-styling` is the source-pinned,
current-format styling release-transition proof. It uses three canonical fixture
packages, retained Chromium contexts and same-backend fresh-target restores on
SQLite and PostgreSQL. See [application assets](application-assets.md) for its
protocol, completed evidence and browser limits.

That operation does not change ordinary discovery/acceptance's exact format-1
admission or four-direction restore matrix. It is not actual Safari/iPhone,
production, #1419 release-candidate or milestone acceptance.

## Topology and immutable identity

The host xtask resolves commits to immutable flake references and alone invokes
Nix and manages lifecycle. Nix compiles the host tool but never invokes its
orchestrator. Qualification admits and roots the selected graph before lifecycle
effects. Standard Nix GC roots protect that graph throughout execution and any
retained lifecycle workspace; there is no checkout-local executable cache.
Playwright runs from a private writable copy of the selected browser package,
using that copy's CLI and imports with pinned Node/browser binaries; the seed
bridge's interpreter is the selected store Node. Source cleanliness and revision
are rechecked around execution and publication. Each operation initializes fresh
persistent SQLite and PostgreSQL source VMs. The harness supplies each VM's
immutable package through the qualification-only internal `productionBaselineVm`
construction seam; ADR-0142's supported module options remain unchanged. Source
state is never live operator data. An external Caddy proxy terminates local TLS
and keeps one stable `https://localhost:8443` browser/protocol origin while it
switches among source and fresh restore targets. HTTP redirects to that origin.

Discovery restarts and reboots each source, then makes one same-schema backup
and restores every SQLite/PostgreSQL source-to-target direction. Acceptance
repeats that work, declaratively activates its target package, proves the target
runtime identity, and restores target-schema backups in all four directions.
Backup format and exact schema version are compatibility authorities; a
rejection is not recovery success.

## Evidence

A completed run attempts one atomic publication at:

```
.xtask/production-baseline-reports/YYYY-MM-DD-<operation>-<source12>-<target12-or-none>-<harness12>/
```

The identity-derived dated name is collision-safe: an existing destination is
never overwritten. A retained destination contains exactly `summary.json` and
generated `summary.md`. JSON validates against `production-baseline.schema.json`
and is authoritative; Markdown is mechanically derived and checked for parity.
Command output reports only the safe destination and identity/check counts.
These are ignored run artifacts, not source deliverables. Link the tested
revisions, result and sanitized reports from the issue/PR when review needs
them. Explicit release evidence belongs in a designated external artifact store
with an owner and retention period.

Schema-v2 reports record the harness source commit, selected Nix harness
`derivation`, `output_path` and `nar_hash`, manifest identities, source/target,
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
broad glob or any other run's paths. Publication never overwrites a report, but
local reports may expire after review. Once no live review needs a report (and
any explicitly required external retention is satisfied), remove only its exact
dated directory under `.xtask/production-baseline-reports/`.

This harness demonstrates only the named revisions, local VM topology, storage
backends, stable HTTPS routing, browser/protocol flows, and representative data.
It makes no public CA, public DNS, internet availability, performance, soak,
capacity, release, or release-candidate claim.
