# Production baseline harness

Four root Just commands expose the opt-in, host-side production baseline and
styling workflows: `production-baseline-discover <revision>`,
`production-baseline-accept <source> <target>`,
`production-baseline-qualify-styling`, and `production-baseline-smoke`.
Qualification deploys immutable `packages.jaunder` revisions through the
supported NixOS module; smoke instead uses the mutable current checkout. These
commands are not part of `just check`, pre-push, CI, or `nix flake check`.

## Prerequisites

Run Just from the repository's Nix development environment. Ordinary
discovery/acceptance require a clean checkout whose canonical `origin` is
`jaunder-org/jaunder`, with the current harness `HEAD` and requested product
commits available upstream. Just captures the full harness `HEAD`, resolves
product revision expressions with Git, and rejects equal resolved acceptance
commits and an empty acceptance target. It checks origin and performs one
`git fetch --quiet --no-write-fetch-head --refetch origin <commits>` over the
distinct harness/source/target set. `--refetch` prevents Git's locally-complete
fast path from accepting a SHA without asking the server for it. This proves
origin serves the objects, not named-branch reachability, at the cost of larger
transfers and normal Git maintenance. A locally committed but unpushed harness
`HEAD` must refuse before lifecycle work.

The host binds Just's expected harness SHA to its independently admitted clean
`SourcePin` before Nix harness admission or lifecycle mutation; it does not
repeat product expression resolution, origin validation or upstream fetch.
Styling also requires a clean checkout and expected-HEAD binding, but uses its
local immutable Git source with no origin, upstream or Git-network requirement.
Smoke has no clean-source qualification admission. Nix realization may still
need ordinary dependency downloads; local-only styling is not a promise that
cold dependencies are already cached.

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
traced-run -- just production-baseline-discover <revision>
```

Run acceptance with source and target expressions resolving to distinct commits
available upstream (full SHAs are also accepted):

```bash
traced-run -- just production-baseline-accept <source> <target>
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
PNG, authentication and all three Syndication Feed formats. Only its canonical
`baseline-seeded.txt` requires HTTP `application/octet-stream`: the pinned
helper stores that detected type and the pinned product serves it unchanged.
Modern seeded text remains strictly `text/plain`, all four modern SVGs remain
`image/svg+xml`, and the independently uploaded PNG remains `image/png`. Status,
Media identities, URLs, lengths and byte hashes remain exact; HTTP success alone
does not prove seeded Media Record ownership or direct row readback. Its exact
v1 wire and historical composer/Post-create contracts are explicit, not inferred
from replies. Ordinary App Password creation/token controls also belong to that
fixed fixture: `.j-app-passwords` and `.j-app-password-token code` for the
historical source, unchanged modern Sessions controls otherwise. The same
selection reaches primary creation, manifest-author access and fresh
mint/use/revoke. All other ordinary sources and live e2e retain strict modern
v2, five-Media and Rendered Title obligations. Incompatible upgrade targets
fail; there is no adapter, retry, selector option or ambient fixture flag.

Historical discovery evidence remains schema1. New evidence attributes the
actual selected graph and current executing harness/verifier, not historical
execution. This applicability correction alone does not establish helper Nix
output admission, backend/browser compatibility or a suitable distinct format-1
target; those require separately executed proofs before positive qualification.

## Separate styling qualification

Run the source-pinned, current-format styling release-transition proof with:

```bash
traced-run -- just production-baseline-qualify-styling
```

This local-only qualification uses three canonical fixture packages, retained
Chromium contexts and same-backend fresh-target restores on SQLite and
PostgreSQL. See [application assets](application-assets.md) for its protocol,
completed evidence and browser limits.

That operation does not change ordinary discovery/acceptance's exact format-1
admission or four-direction restore matrix. It is not actual Safari/iPhone,
production, #1419 release-candidate or milestone acceptance.

## Mutable local smoke

```bash
traced-run -- just production-baseline-smoke
```

Smoke runs the SQLite/PostgreSQL VM lifecycle against the mutable current
checkout's `path:` flake, under the shared lease. It performs no qualification
source pin or upstream admission and publishes neither ordinary nor styling
evidence. A successful smoke is a lifecycle diagnostic, not qualification.

## Topology and immutable identity

Just/Git owns public workflow selection, ordinary input normalization and
ordered fail-fast preflight. A focused host xtask leaf receives full immutable
product IDs and the expected harness SHA, invokes Nix and manages the domain
lifecycle. Nix compiles the host tool but never invokes its orchestrator.
Qualification admits and roots the selected graph before lifecycle effects.
Standard Nix GC roots protect that graph throughout execution and any retained
lifecycle workspace; there is no checkout-local executable cache. Playwright
runs from a private writable copy of the selected browser package, using that
copy's CLI and imports with pinned Node/browser binaries; the seed bridge's
interpreter is the selected store Node. Source cleanliness and revision are
rechecked around execution and publication. Each operation initializes fresh
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

For each ordinary source backend, lifecycle work observes service identity after
browser/Media creation and after restart plus actual read-only Atom/RSS/JSON
Syndication Feed verification. Both observations require canonical service
User/Group, resolved non-root IDs, the running service's MainPID, verified
executable and all four `/proc` UID/GID fields to agree. The early observation
omits lock probing and lock fields; only the later observation additionally
requires publisher-lock ownership to match the service account. Preserve the
CREATE → RESTART → read-only verification sequence without Feed warmup. MainPID
must stay stable within each observation; a restart may change it. Numeric-only
summaries are emitted to stderr; structured observations stay restricted in the
owned run workspace without changing the published evidence schema. These
observations prove the running service/lock boundary, not the identity of
already-exited seed/CLI children. Host command/parser stand-ins are not actual
VM proof.

## Evidence

A completed run attempts one atomic publication at:

```
.xtask/production-baseline-reports/YYYY-MM-DD-<operation>-<source12>-<target12-or-none>-<harness12>/
```

The identity-derived dated name is collision-safe: an existing destination is
never overwritten. A retained destination contains exactly `summary.json` and
generated `summary.md`. JSON validates against `production-baseline.schema.json`
and is authoritative; Markdown is mechanically derived and checked for parity.
The workflows return ordinary process status and focused diagnostics, including
the safe destination and identity/check counts. They do not support `--json` or
produce the generic xtask report envelope, `.xtask/last-result.json`, or
`xtask-done:` sentinel. Unrelated xtask commands retain their own reporting
contracts. `traced-run` capture is separate from workflow evidence. These are
ignored run artifacts, not source deliverables. Link the tested revisions,
result and sanitized reports from the issue/PR when review needs them. Explicit
release evidence belongs in a designated external artifact store with an owner
and retention period.

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
evidence collection, rendering, schema/Markdown parity, sanitization, a scan,
required cleanup, or source recheck fails, nothing is published. An otherwise
valid, sanitized product/protocol-failed outcome may be published with its
failed outcome and failure classes; the command still exits nonzero. This
reportable failure is distinct from publication-inadmissibility. Styling instead
publishes only after its own validation, canary scans, cleanup and source
recheck succeed; it has no ordinary schema/parity or failed-report contract.
Smoke publishes neither form.

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
