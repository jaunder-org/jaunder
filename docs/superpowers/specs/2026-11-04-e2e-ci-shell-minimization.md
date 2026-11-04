# Minimize the CI E2E host shell without reducing lane coverage

> Approved design: request `bdfeac26-ae92-4df4-85d0-e6ec03728eee` /
> `ci-e2e-shell-spec-v1`, exact answer “Approve narrow-shell design.” Approval
> is recorded; earlier pending-approval text is superseded. Separate from #1874
> and PR #1877.

## Outcome and scope

Add Linux `.#ci-e2e` and route only `.github/workflows/ci.yml` jobs
`e2e-chromium` and `e2e-firefox-lane` to it. Preserve their commands/arguments,
all catalog identities, SQLite/PostgreSQL × Chromium/Firefox coverage, NixOS VM
behavior, Firefox census/reconciliation, reports, diagnostics, and required
verdicts. Four validation producers and Firefox reconciliation remain on
unchanged `.#ci`; all other jobs, aggregates and artifact names remain
unchanged. No ENOSPC cure or capacity claim: the recorded 72G/68G pre-lane
baseline remains unattributed.

## Required host path and environment

Both jobs run `scripts/cargo-xtask` → `traced-run` → Git/Nix →
`nix build .#xtask --no-link --print-out-paths` → Nix-built host xtask. Keep the
Nixpkgs Cargo executable solely as `cargo xtask`'s external-subcommand
dispatcher, launcher, Nix/Git/coreutils/traced-run, OpenSSL/SQLite/dav1d
runtime, exact OpenSSL/dav1d `LD_LIBRARY_PATH` prepend, fail-closed checkout
`.xtask/tmp` create/use/remove probe and `TMPDIR` export. Keep `LC_ALL=C.UTF-8`,
`TZ=UTC`, and `TZDIR` until proven unnecessary. Do not rely on cached
`xtask/target`. Do not include the full host Rust toolchain:
`scripts/cargo-xtask` invokes `nix build .#xtask --no-link --print-out-paths`,
whose Nix derivation retains its pinned build toolchain and whose compiled
binary runs on the host. The host dispatcher does not compile Rust itself.

Chromium dispatch builds the combo check, validates diagnostics/duration/boot
phases, scans flaky tests, and SQLite/Chromium verifies server-function coverage
from its capture. Firefox lane validates catalog identity, builds its check,
recovers diagnostics and performs Rust validators/flaky/phase checks. The lane
VM/test graph retains KVM and `nixos-test`, backend, app, Node, Playwright
browser, fonts, telemetry, evidence and reports. Guest browser/font variables
and coverage authority remain untouched. Neither host path invokes the broad
validation, coverage, formatting or wasm tools; host theme-thumbnail Chromium
and node-module IDE provisioning are likewise outside these call paths. Keep all
uncertain inputs.

The host shell uses `pkgs.cargo` only as the Cargo external-subcommand
dispatcher rather than adding the full Fenix toolchain input; the separately
Nix-built xtask derivation retains its pinned toolchain. `cargo xtask`
dispatches to the launcher and does not compile Rust on the host. The Nixpkgs
Cargo closure may still reference rustc at runtime; no closure saving is claimed
until paired measurements. The host shell also omits Emacs, cargo
coverage/nextest/deny/crap, ast-grep, formatters, wasm tools, theme-thumbnail
browser and `devtool provision-node-modules`; broad `.#ci` retains them. Host
Playwright/browser/font variables, `RUST_SRC_PATH`, and
`JAUNDER_DEVTOOL_PRODUCT_CARGO_HOME` are omitted. Nix test derivations retain
their guest dependencies; `jq` remains in lane driver-configuration graph.
`TZDIR` is retained. No host-only PostgreSQL/SQLite, Python, TypeScript, Cachix,
exiftool, unzip or other uncertain inputs are excluded without call-path proof.

## Proof and boundaries

Inspect only the two job substitutions and ensure every other shell consumer is
unchanged. Compare matched broad/narrow shell, xtask and exact lane **output**
closure identities using comparable fresh runners with recorded starting path
inventories; count shell-entry and producer acquisitions, subtract shared
xtask/lane paths, and report unique acquired NAR bytes separately from `df`
filesystem allocation. Detailed paired protocol is in the approved outline.
Named root `df` observations are diagnostic-only: emit explicit success/failure
status but always exit successfully so they cannot suppress or replace producer
verdicts. Observe post-setup, after shell entry, after producer, and final after
artifact upload. Never realize missing roots solely to size, repeat broad `.#ci`
path-info, scan the whole root, sum overlapping closure sizes, purge/GC, add
resource flags or change runner/image/capacity. Material unique acquired-byte
reduction and unchanged full producer/reconciliation/coverage/aggregate verdicts
are required before claiming success. Full applicable CI proof covers both
Chromium backends, all Firefox identities, census/reconciliation and required
aggregate boundary. Rollback only the two named jobs to their prior `.#ci` shell
command. PR #1877 remains unchanged.
