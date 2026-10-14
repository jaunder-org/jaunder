# ADR-DRAFT: Runtime observability credentials with private refreshable copies

- Status: proposed
- Date: 2026-10-13
- Issue: [#1744](https://github.com/jaunder-org/jaunder/issues/1744)

## Context

[ADR-0196](../0196-single-host-nixos-deployment-stack.md) provides a separate
HTTPS operator hostname, hostname-wide Basic Auth and credential-free loopback
Victoria ingestion. Its inline username/hash interface embeds the evaluated hash
in generated Caddy configuration and the Nix store. This allows offline password
guessing by unrelated store readers, even though plaintext is not disclosed.

Runtime credentials must compose with manual provisioning, sops-nix or agenix
without selecting a secret manager. Caddy must consume them under the real NixOS
unit's ordinary identity and hardening on startup and reload. Importing a file
alone is insufficient: Caddy permits empty imports and does not fully validate
encoded hash contents during provisioning.

## Decision

Add a runtime alternative while retaining ADR-0196's inline exposure contract
and routing/isolation decisions. The minimal Jaunder module and
application/storage interfaces do not change.

- `services.jaunder.stack.observability.basicAuth.credentialsFile` is a nullable
  absolute runtime filename string, default `null`, not a Nix path or glob.
  Evaluation/build never reads or copies credential contents into store
  artifacts. An enabled observability host requires exactly one complete source:
  file mode with both legacy fields empty, or the compatible inline
  username/hash pair.
- File-only `credentialsFileAlgorithm` is `bcrypt` by default or `argon2id`.
  Inline algorithm inference and defaults are unchanged. Runtime entries are
  nonempty ASCII lines of exactly two tokens, username and complete encoded
  hash, with username grammar `[A-Za-z0-9._-]+`. Blank/comment/directive lines
  and third tokens are rejected. Bcrypt accepts `$2a$`/`$2b$`; Argon2id uses v19
  PHC encoding, valid positive bounded costs and canonical unpadded standard
  Base64 salt/hash. Salt is embedded, not a separate field.
- Systemd's manager reads the source into one named private read-only
  `LoadCredential` copy, imported literally inside hostname-wide Basic Auth. A
  root-owned 0600 source in a 0700 directory needs no direct Caddy readability.
  Operators own provisioning and ordering before `caddy.service`.
- Require configured systemd >=260 and set `RefreshOnReload=credentials`.
  Unsupported file-mode configurations fail clearly; inline remains supported.
  Preserve ordinary unprivileged shared-Caddy reload, without a privileged
  helper, source fallback or sandbox relaxation. Pinned systemd 261.2 / Caddy
  2.11.4 are the real lifecycle proof target, not a portability claim for older
  nixpkgs.
- Strict managed-copy validation runs before startup and inside `ExecReload`
  after credential refresh and before Caddy adaptation/submission. Missing
  environment/copy, skipped setup, unreadable/empty/malformed entries and
  algorithm mismatch fail closed. Caddy import/provisioning alone is not enough.
- Source replacement alone leaves active authentication unchanged. Both
  supported reload and restart can refresh the copy and rotate credentials;
  restart is not exclusive. Failed reload retains the prior parsed authenticated
  configuration in the healthy main process, but does not promise
  credential-mount rollback. Repair source/provisioning and retry reload or
  restart.

## Consequences

Operators can keep username/hash contents out of generated Nix-store artifacts
while preserving existing inline deployments. The runtime copy reduces unrelated
local-reader exposure, but root and Caddy remain trusted consumers. Caddy's
protected autosave and admin configuration can contain imported credentials;
operators must protect those runtime surfaces too. This is not root-adversary
protection or a new account/role system.

Normal reload also applies configuration for other virtual hosts sharing Caddy;
restart interrupts that shared service. Provisioners must atomically replace and
keep sources stable during lifecycle operations. Manual `/run` secrets need
reprovisioning after boot; secret managers supply their own supported
provisioning units and ordering. No secret-manager dependency is imposed.

Direct source imports would require Caddy-readable source permissions and lose
private-copy isolation. Disabling shared reload or adding a privileged helper is
unnecessary on the supported refresh-capable systemd. Older systemd consumers
retain inline mode rather than receive an unsafe silent fallback.

Existing distinct hostname rules, native prefixes, loopback bindings, trusted
application proxy and credential-free collector traffic remain unchanged.
Procedures and exact format belong in
[the deployment runbook](../../DESIGN.md#optional-https-observability-host-and-basic-auth);
current architecture is projected in [ARCHITECTURE](../../ARCHITECTURE.md).
