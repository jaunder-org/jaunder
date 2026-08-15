# #938 — infrastructure and process decision records

Issue: [#938](https://github.com/jaunder-org/jaunder/issues/938). Milestone:
Developer tooling & DX.

## Summary

Five load-bearing parts of Jaunder have shipped without a governing decision:
the process configuration surface, Nix deployment outputs, the Cargo workspace
split, the compensating host-test steps, and the Emacs Protocol Client's App
Password store. This cycle records four ADRs, projects them into the
architecture view, and changes no product behavior.

The ADRs establish the intended contracts already evidenced by the repository.
Where current behavior violates an accepted contract, it must be recorded as a
focused follow-up issue rather than silently canonicalized. The architecture
view must distinguish accepted target from current behavior until every such
issue lands.

## Decisions

### D1 — ADR boundaries

Write four numberless drafts and promote them at ship:

1. process configuration and CLI precedence;
2. declarative Nix deployment and package outputs;
3. the Cargo workspace boundary and its compensating `host_tests` gate; and
4. Emacs `auth-source` App Password storage.

The root workspace exclusion and `host_tests` belong together: `xtask` is
excluded from the root workspace and application derivation source, while the
separate `tools` workspace has unit tests that application coverage does not
execute. Their explicit host test steps compensate for those uncovered unit-test
paths. The other three decisions govern different operator or client boundaries
and remain independently reviewable.

### D2 — process configuration is a stable operator contract

The `jaunder` CLI is the public operator surface. For each applicable argument,
clap's ordinary precedence is explicit flag over matching `JAUNDER_*`
environment fallback over documented default. The ADR must name applicability
rather than imply every setting configures every subcommand: `JAUNDER_VERBOSE`
is global; storage/database settings apply to the storage-using commands; and
`JAUNDER_BIND`, `JAUNDER_ENV`, and `JAUNDER_RUNTIME_FILE` are `serve` settings.

The process-shape variables are `JAUNDER_BIND`, `JAUNDER_DB`,
`JAUNDER_STORAGE_PATH`, `JAUNDER_ENV`, `JAUNDER_RUNTIME_FILE`, and
`JAUNDER_VERBOSE`. `JAUNDER_RUNTIME_FILE` defaults to
`<storage-path>/runtime.json`; `JAUNDER_VERBOSE` defaults to false.
`JAUNDER_DB_PASSWORD` and `JAUNDER_DB_PASSWORD_FILE` are PostgreSQL secret
channels that override a password embedded in `JAUNDER_DB` or `--db`; when both
exist, the file takes precedence and its trailing whitespace is trimmed. A
missing, unreadable, or non-Unicode configured password source is a
configuration error. Secrets are not added to the database URL merely to make
configuration look uniform.

`JAUNDER_ENV=prod` is a security and lifecycle boundary: it enables secure
cookies and does not initialize a missing database, while existing databases
still open and migrate normally. Development auto-initializes only a missing
SQLite database; it does not provision PostgreSQL. Defaults are for local
development. Production configuration is explicit operator guidance, not a claim
that every arbitrary environment setting is mandatory.

This configuration is distinct from the validated, stored `site_config` key
registry governed by ADR-0102. Neither surface is an alias or fallback for the
other.

### D3 — Nix exports one deployable binary and declarative service

`packages.jaunder` is the deployable single binary of ADR-0008. The
`nixosModules.jaunder` module is Jaunder's supported declarative NixOS
integration. Its public options are `enable` (default false), `bind` (default
`127.0.0.1:3000`), `db` (default `sqlite:./data/jaunder.db`), and `prod`
(default false). When enabled, it supplies a dedicated `jaunder` user and group,
durable state via `StateDirectory=jaunder`, a matching working directory,
`JAUNDER_BIND` and `JAUNDER_DB`, optional `JAUNDER_ENV=prod`, idempotent
`jaunder init --db "$JAUNDER_DB" --skip-if-exists` before startup, and a
restarted `jaunder serve` service.

The module adapts only that subset of D2; it exposes no option or secret-file
adapter for `JAUNDER_DB_PASSWORD{,_FILE}`. A PostgreSQL deployment that needs a
password must inject it through its systemd/service-manager configuration
outside this module, without placing it in the database URL. Production import
guidance must explicitly set `prod = true` as well as selecting bind/database
values. The module does not create a second stored configuration model and does
not manage TLS: ADR-0008's external reverse-proxy boundary remains in force.

`packages.site` is not a deployment artifact. The binary embeds the CSR bundle
and assets; `packages.site` remains only as the input to
`cargo xtask audit-wasm` for bundle-size analysis. The interactive and
PostgreSQL NixOS configurations are development/test VMs, not supported
deployment presets.

### D4 — Cargo workspaces are execution-boundary partitions

The root workspace contains product and shared application crates. `xtask` is a
separate host-only workspace, excluded from the root workspace and from the
application derivation source. `tools` is a separate workspace for `devtool`,
`coverage`, and `doctests`: its crates have intentionally mixed execution
locations, so its boundary is workspace ownership rather than a claim that every
member runs only in a derivation. This complements ADR-0028's execution litmus:
host orchestration and analysis remain in `xtask`; code that must execute in a
derivation belongs in `devtool`; reusable pure logic may become a library.

`host_tests` executes `xtask-tests` and `tools-test` on every reached
`cargo xtask check` or `validate` ladder run because application coverage does
not execute their unit-test suites. These are a required compensating gate, not
optional fast-test convenience. They run unit tests but do not distort the
application coverage contract; coverage is governed separately by the existing
Nix checks.

### D5 — Emacs delegates App Password storage to auth-source

The Emacs Protocol Client obtains the App Password required by ADR-0014 through
Emacs `auth-source`. The lookup identity is the active blog's URL host and its
configured username; the URL port is deliberately excluded. The client does not
claim to normalize either value at this boundary. This lets a single Jaunder
instance credential serve its HTTP endpoints without inventing a second
port-specific secret identity.

The client never prompts for, writes, or otherwise persists an App Password
itself. It asks `auth-source` for at most one matching secret and raises a loud
non-interactive error when none is available. `auth-source` owns persistence;
the transport still obtains that secret at request time to construct its Basic
header. A missing entry is a configuration error, not retryable transport work.
Current source still retries that broad signalled-error path before surfacing
the configuration error; #1062 is the focused implementation-debt issue required
by AC7.

ADR-0035's test harness may provision a temporary `auth-source` entry only for
its own live-test scope; that fixture is not a client persistence path. ADR-0038
remains the HTTP transport decision, and ADR-0047 remains the
multi-blog/configuration-threading decision. Issue #76's planned self-provision
of an App Password conflicts with this decision and must be closed, superseded,
or redesigned before promotion.

### D6 — architecture projection and implementation debt

`docs/ARCHITECTURE.md` replaces all five #938 bullets in `Un-ADR'd reality` with
cited, accepted decisions. It records the four ADRs' current/target status
truthfully and retains unrelated un-ADR'd entries. `CONTEXT.md` needs no new
domain term: process configuration, deployment, Cargo workspace, and
`auth-source` are technical boundaries, while **App Password** and **Protocol
Client** already have precise glossary definitions.

No Rust, Nix, Elisp, database, route, or browser behavior changes belong in this
issue. Any observed divergence from D2–D5 needs one focused issue before
promotion, with the relevant ADR and architecture projection linking to it. The
issue must not promote a vague implementation umbrella or silently expand #938
into a feature change.

## Acceptance criteria

- **AC1 — four decisions are accepted.** Four promoted ADRs exist with D1's
  boundaries. Each names #938, its governing ADR constraints, the rejected
  alternatives or trade-offs, and consequences.
- **AC2 — process configuration is complete.** Its ADR names the CLI contract,
  flag/environment/default precedence and command applicability, every D2
  variable group, runtime-file and verbose defaults, PostgreSQL secret
  precedence/error behavior, the SQLite-only development auto-init boundary,
  production effects, and the separation from ADR-0102's stored `site_config`.
- **AC3 — deployment is complete.** Its ADR states the `packages.jaunder` /
  `nixosModules.jaunder` roles; all four module options/defaults; service
  account and durable-state model; initialization/startup behavior; the external
  PostgreSQL secret-injection boundary; production `prod` guidance;
  reverse-proxy boundary; audit-only `packages.site`; and non-production VM
  status.
- **AC4 — workspace and gate are complete.** Its ADR states all three workspace
  roles, accurate `xtask` source-exclusion rationale, crate-level mixed `tools`
  execution, ADR-0028 compatibility, and why `xtask-tests` plus `tools-test` are
  mandatory non-coverage unit-test gates.
- **AC5 — client credential storage is complete.** Its ADR states that
  `auth-source` owns App Password storage; host-plus-configured-username
  identity; deliberate port exclusion; one-result lookup; loud absence failure;
  request-time transport use without client-side persistence/prompt; correct
  non-retry behavior; #76 disposition; and the ADR-0014/0035/0038/0047
  boundaries.
- **AC6 — the materialized view is truthful.** `docs/ARCHITECTURE.md` cites all
  four accepted ADRs and removes only the five #938 bullets from
  `Un-ADR'd reality`.
- **AC7 — implementation debt remains actionable.** Before promotion, every
  observed accepted-target mismatch has exactly one focused open issue. Neither
  ADR nor architecture projection claims that unimplemented behavior has
  shipped.
- **AC8 — documentation gates pass.** ADR format, generated index parity,
  architecture-view parity, link checks, formatting, and documentation
  validation pass after promotion.

## Out of scope

- Changing CLI flags, defaults, environment parsing, or the `site_config`
  registry.
- Adding NixOS module options or changing package outputs.
- Merging Cargo workspaces, changing Nix source filtering, or changing gate
  execution/coverage behavior.
- Adding App Password prompts, client-side secret persistence, or port-specific
  credential identities.
- Implementing any follow-up behavior discovered while documenting these
  decisions.

## Risks

- **Accepted target may precede code.** The architecture view must not present
  the target as current; AC7 requires focused tracking where it does not.
- **Operator compatibility is load-bearing.** Calling the existing CLI/env
  precedence a contract raises the compatibility cost of later changes; that is
  preferable to unrecorded deployment breakage.
- **The Nix module has intentionally narrow configuration.** Expanding it
  without a new design could blur D2 process configuration with stored
  configuration or TLS ownership.
- **Credential identity excludes port.** This intentionally favors one
  instance-level App Password across endpoint ports; changing that requires a
  new secret-identity decision and migration story.
