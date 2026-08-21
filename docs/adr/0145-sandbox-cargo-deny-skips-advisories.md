# ADR-0145: Sandboxed cargo-deny skips advisories

- Status: accepted
- Date: 2026-08-21
- Issue: [#1074](https://github.com/jaunder-org/jaunder/issues/1074)

## Context

`cargo-deny` has two different execution environments in Jaunder's gate. The
host ladder runs `cargo deny check`, which includes advisory checking and may
fetch the RustSec advisory database. Nix derivations must be hermetic: a
sandboxed check cannot rely on network access, and a command that tries to fetch
from the network turns a reproducibility boundary into runtime luck.

[ADR-0052](0052-devtool-unifies-static-checks.md) deliberately left compiling
checks, including `cargo-deny`, outside `devtool check` because they needed
offline Cargo plumbing. Issues #1072 and #1073 added reusable tools-workspace
artifacts and workspace-specific offline Cargo configuration. Issue #1074 is the
point where `cargo-deny` can enter `devtool check`, but only after choosing what
advisory checking means in the sandbox.

There are two viable policies:

- vendor or otherwise provide the RustSec advisory database hermetically, making
  sandboxed and host advisory behavior converge; or
- keep advisory checking host-only for now, and make the sandbox run the
  offline-safe `cargo-deny` checks.

The first option is stronger parity but adds advisory database vendoring,
refresh, and review mechanics to this issue. The second option matches the
current crane `deny` boundary and keeps hermetic static checks honest, but keeps
a named host/sandbox difference.

## Decision

Sandboxed `devtool check cargo-deny` skips `advisories`. In sandbox Cargo mode
it runs only the offline-safe cargo-deny checks: `bans`, `licenses`, and
`sources`.

Host-mode `devtool check cargo-deny` keeps the full host policy and runs
`cargo deny check`, including advisories. The host `xtask` `cargo-deny` StepSpec
also remains native in #1074; issue #276 owns the later host-ladder unification.

The sandbox command must use the product workspace's offline Cargo configuration
and force Cargo offline before spawning `cargo-deny`. A missing product offline
Cargo home is an error before execution.

## Consequences

The Nix `static-checks` derivation can include `cargo-deny` through
`devtool check --all --sandbox-cargo` without network access. The sandboxed
check proves bans, licenses, and sources hermetically.

Advisory freshness remains a host-gate responsibility until a later issue
vendors or otherwise supplies the RustSec advisory database hermetically. The
difference is intentional and documented, not accidental command drift.

This creates temporary duplication: `devtool check --all --sandbox-cargo` and
the existing crane `deny` derivation both exercise the offline-safe dependency
policy, while the host ladder still runs full `cargo deny check`. Issue #276 can
remove that duplication when it moves the remaining compiling checks behind the
shared `devtool check` surface.

Rejected: making sandboxed cargo-deny run `advisories --disable-fetch`. That
would still require a pre-existing advisory database in the sandbox and would
fail closed for reasons unrelated to the dependency policy unless this project
also defines how that database is supplied.

Rejected: silently running `cargo deny check` in the sandbox and relying on Nix
network denial to catch fetches. The command name would imply parity with the
host path while its behavior depended on an environmental failure mode.
