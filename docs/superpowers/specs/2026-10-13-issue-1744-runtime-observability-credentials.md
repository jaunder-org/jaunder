# Issue #1744: Runtime observability Basic Auth credentials

Status: proposed for cold soundness review; not approved. Decision
**JAU-1744-RELOAD-001** follows the Lead-selected proposal below. Implementation
requires spec and triggered outline approval.

## Outcome

Operators can supply observability Basic Auth usernames and password hashes from
an absolute runtime filename without placing their contents in Nix-store
artifacts. Existing inline configurations remain compatible, and every external
observability request remains authenticated before proxying.

## Load-bearing decisions

- Add `services.jaunder.stack.observability.basicAuth.credentialsFile`, a
  nullable runtime absolute filename string, default `null`. Reject Nix path
  values, empty or relative filenames, and glob imports. Evaluation and builds
  never read or copy the credential contents; generated artifacts carry
  references only.
- With an observability hostname enabled, accept exactly one complete source:
  the existing nonempty valid `username`/`passwordHash` pair, or
  `credentialsFile` with both inline fields empty. Reject conflicting and
  incomplete sources. Retain both legacy string defaults (`""`) and inline
  validation behavior. File mode obtains usernames from the file, not the legacy
  username option.
- Add `basicAuth.credentialsFileAlgorithm`, enum `bcrypt` or `argon2id`, default
  `bcrypt`. This selects file-entry validation and Caddy's file-mode algorithm;
  it has no effect on inline mode, which retains hash-based algorithm inference.
- The supported file is a nonempty list of credential-entry lines, each
  containing exactly two whitespace-separated tokens: an ASCII username matching
  `[A-Za-z0-9._-]+` and one complete encoded hash for the selected algorithm.
  Comments, non-entry directives, imports, braces, and third tokens are not part
  of this format. Argon2id's salt is embedded in its PHC hash, not a third
  token.
- Preserve existing complete `$2a$`/`$2b$` bcrypt support and Argon2id PHC
  support: `$argon2id$v=19$m=...,t=...,p=...$salt$hash`, with valid costs and
  strict unpadded standard Base64 salt/hash. Reject plaintext, malformed
  encodings, and hashes that do not match the file algorithm.
- Use one named systemd `LoadCredential` for a private read-only Caddy copy, a
  literal runtime import inside hostname-wide `basic_auth`, and
  `RefreshOnReload=credentials`. Preserve normal unprivileged shared-Caddy
  reload; do not disable it, use a privileged `+` helper, or directly read the
  source as Caddy by default. File mode requires systemd >=260; unsupported
  runtimes must clearly reject it rather than ignore the directive or fall back.
  The pinned systemd 261.2 is the execution-proof target; inline mode is
  unchanged.
- Validate the managed copy before startup and INSIDE `ExecReload`, after
  systemd credential refresh and before Caddy adaptation/submission. Reject
  absent credential environment/copy, skipped credential setup, unreadable,
  empty, malformed, non-entry, and algorithm-mismatched contents. Never skip
  validation or fall back to the source. Caddy import/provisioning alone is
  insufficient.
- Failed reload retains the previous parsed authenticated configuration, not
  necessarily the previous credential mount. A refresh may replace the copy
  before validation fails; repair the source and retry reload or restart.
- Correct the original issue's restart-only assumption: with this supported
  refresh setting, successful reload may rotate credentials. Atomic source
  replacement alone does not activate them; documented reload or restart does.
  Restart remains supported, but is not the exclusive rotation procedure.
- Prove actual startup/reload identities, adaptation/validation, hardening,
  credential environment, and mount visibility under pinned NixOS Caddy. Static
  source evidence establishes the proposal, not executed lifecycle correctness.
- Preserve separate normalized application/observability hostnames, existing
  hostname-wide protection, native `/metrics`, `/logs`, and `/traces` routes,
  loopback backend/collector bindings, and credential-free internal ingestion.
  Preserve the application route's existing trusted-proxy boundary.
- The security goal is reducing exposure to unrelated local/store readers, not
  protecting credentials from root or Caddy. Imported credentials may appear in
  Caddy's protected runtime autosave and admin configuration; those remain
  inside the documented runtime trust boundary. Manual provisioning may use a
  root-readable `root:root 0600` source: systemd's manager reads it, and the
  same trusted Caddy consumer reads its private copy. No new secret manager,
  account system, or root-adversary protection is added.

## Acceptance

### Behavioral proof to execute

- Evaluation accepts valid inline and file modes for both algorithms, including
  file mode without a legacy username. It rejects mixed sources, either missing
  inline field, invalid filenames/types, and unsupported file algorithms. File
  algorithm selection leaves existing inline inference unchanged. Unsupported
  systemd file-mode configurations are clearly rejected; inline still works.
- Using runtime-provisioned synthetic credentials, inspect generated Caddy
  configuration, unit contents, and relevant derivation/store artifacts: no
  file-mode username or hash appears; only runtime filenames/references do.
  Provision test secrets at runtime rather than embedding them in Nix fixtures.
- HTTPS requests with missing or wrong credentials fail for every frontend and
  representative metrics, logs, and traces query API route; correct credentials
  succeed. The hostname-wide authentication boundary also covers unmatched
  paths.
- Missing, unreadable, empty, malformed, directive-containing, third-token, and
  algorithm-mismatched credential files prevent successful startup and never
  permit unauthenticated proxy access. Demonstrate failure at the actual owning
  read boundary, not an assumed permission failure caused only by mode bits.
- Prove ordinary startup/reload identities and copy access under the pinned
  sandbox. Successful config reload keeps shared Caddy functionality. Missing or
  unreadable refresh, skipped setup, missing environment/copy, and invalid
  content reject reload before submission, retain previous parsed
  authentication, and never enable unauthenticated access. Prove recovery
  without assuming mount rollback or stopping the healthy main service on failed
  reload.
- Rotate using BOTH documented restart and supported credential-refresh reload:
  old credentials cease working and new credentials succeed on frontend/query
  routes. Source replacement alone leaves active authentication unchanged.
- Existing bcrypt/Argon2id inline configurations, loopback isolation, prefix
  routing, application proxy behavior, and collector ingestion continue working.
- Use existing focused seams in `nix/checks.nix`: `jaunder-stack-module`
  evaluation and the stack HTTPS NixOS VM checks (`jaunder-stack-sqlite-bcrypt`,
  `jaunder-stack-sqlite-argon2id`, `jaunder-stack-postgresql`). Extend real
  lifecycle coverage rather than substituting mocks; retain execution evidence
  in ignored run storage or PR evidence, not committed audit reports.

### Enduring deliverables after approvals

- Maintain `docs/DESIGN.md` with a complete file-mode example, exact entry and
  algorithm format, restrictive ownership/permissions, manager-readable source
  requirements, manual/sops-nix/agenix provisioning and startup ordering,
  interactive stdin hashing without plaintext arguments, restart/reload rotation
  and failure recovery, systemd >=260 requirement versus pinned 261.2 proof, and
  runtime/store security boundary.
- Update `CONTRIBUTING.md` compatibility wording and `docs/ARCHITECTURE.md` to
  describe both sources and the approved runtime lifecycle.
- Record the runtime-secret/lifecycle decision as a numberless draft under
  `docs/adr/drafts/`, adding a runtime alternative to ADR-0196 while retaining
  its inline store-exposure and routing/isolation contracts. Explain supported
  refresh/reload semantics and portability limits without rewriting that
  accepted historical decision. Cite the draft in the architecture; do not edit
  the ADR or generated `docs/README.md`, allocate a number, or promote from this
  branch.

## Boundaries

No new frontend routing, application authentication, database behavior,
retention, telemetry pipeline, production deployment, or secret-manager
dependency. Preserve existing inline users and the narrow minimal NixOS module.
Approval of this spec and any triggered outline precedes implementation; this
drafting assignment creates neither an outline nor an ADR and performs no
staging or commit.
