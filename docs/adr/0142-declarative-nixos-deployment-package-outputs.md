# ADR-0142: Declarative NixOS Deployment and Package Outputs

- Status: accepted
- Date: 2026-08-14
- Issue: [#938](https://github.com/jaunder-org/jaunder/issues/938)

## Context

[ADR-0008](0008-deployment-model.md) chose a single-binary deployment model with
an external reverse proxy, but the repository now exposes a concrete NixOS
module and package set that go beyond that high-level statement. Operators can
import `nixosModules.jaunder`, and flakes expose `packages.jaunder` plus
`packages.site`. The architecture view already described those outputs, but no
ADR said which output was deployable, which was test or audit infrastructure, or
what the NixOS module promised.

The current module is intentionally small. It adapts the process configuration
contract from
[the process-configuration decision](0144-process-configuration-cli-contract.md)
into systemd environment variables, creates durable state under the standard
NixOS state directory, and runs the single binary. It does not own TLS or the
stored `site_config` registry, and it does not expose a secret-file option for
PostgreSQL passwords.

`packages.site` is a near-miss. It once looked like a deployment artifact, but
the server binary now embeds the CSR bundle and public assets. The `site` output
survives because `cargo xtask audit-wasm` builds it to inspect the wasm bundle
for size, not because a production service should serve it from disk.

## Decision

`packages.jaunder` is the deployable package output. It builds the `jaunder`
server package as a self-contained binary, including the embedded CSR bundle and
public assets.

`nixosModules.jaunder` is Jaunder's supported declarative NixOS integration. Its
public options are:

- `services.jaunder.enable`, default false;
- `services.jaunder.bind`, default `127.0.0.1:3000`;
- `services.jaunder.db`, default `sqlite:./data/jaunder.db`;
- `services.jaunder.prod`, default false.

When enabled, the module:

- creates a `jaunder` group and normal `jaunder` user;
- sets the user's home to `/var/lib/jaunder` and installs the `jaunder` CLI for
  that user;
- runs the service as `User = "jaunder"` and `Group = "jaunder"`;
- uses `StateDirectory = "jaunder"` and `WorkingDirectory = "%S/jaunder"`;
- maps `bind` to `JAUNDER_BIND` and `db` to `JAUNDER_DB`;
- maps `prod = true` to `JAUNDER_ENV=prod`;
- runs `jaunder init --db "$JAUNDER_DB" --skip-if-exists` in `preStart`;
- starts `jaunder serve` as `ExecStart`;
- restarts `on-failure` after `2s`.

The module adapts only this subset of the process configuration contract. It has
no option for `JAUNDER_DB_PASSWORD` or `JAUNDER_DB_PASSWORD_FILE`; PostgreSQL
deployments that need a password inject the secret through systemd or another
service-manager mechanism outside the module, without putting the password in
`services.jaunder.db` or the database URL.

Production imports should set `prod = true` and select deployment-specific bind
and database values. The module does not configure TLS; the ADR-0008 external
reverse-proxy boundary remains in force.

`packages.site` is not a deployment artifact. It is retained only as the wasm
bundle tree used by `cargo xtask audit-wasm` for bundle-size analysis. The
interactive and PostgreSQL NixOS configurations are development/test VMs, not
supported deployment presets.

## Consequences

The NixOS module is now an operator compatibility surface. Changes to option
names, defaults, account/state layout, init/start commands, restart policy, or
which process variables it maps require compatibility review.

The module remains deliberately narrow. That keeps it easy to audit and avoids
creating a second configuration model, but PostgreSQL password injection stays
an operator/systemd concern until a future ADR expands the module's secret
handling.

The deployable artifact is unambiguous: use `packages.jaunder`. `packages.site`
may change to support audit tooling and must not be documented as a runtime
bundle.

Rejected alternatives:

- Treating the NixOS module as incidental test scaffolding. It is exported as a
  public flake module and has stable operator-facing options.
- Serving `packages.site` in production. The binary embeds assets, and keeping a
  separate site tree would reintroduce the external asset edge ADR-0008 and the
  embedded-asset work removed.
- Adding module-managed TLS or reverse-proxy configuration here. ADR-0008 keeps
  TLS outside Jaunder.
- Adding database password options without a design. Secret ownership affects
  systemd unit materialization, Nix store exposure, and operator expectations;
  that needs its own decision.
