# Issue #1744: Runtime credentials implementation outline

> Execute with `jaunder-iterate`; delegate bounded work with `jaunder-dispatch`.
> Trigger: security/auth lifecycle and public NixOS options. Outline not
> approved. Spec approval **JAU-1744-SPEC-001**: human answer `1744 approved`,
> relayed through established Speaker → Manager, as supplied by Lead. Governing
> spec: `../specs/2026-10-13-issue-1744-runtime-observability-credentials.md`.
> Approved SHA256:
> `71b79eccf4c1f877566f1572ada020db69ab28aa89a86edf355ef7850e5fdbc9`. Approval
> includes reload-capable rotation and systemd >=260; do not edit the spec.

## Scope and approval summary

In: runtime private credentials, public options, real lifecycle proof, operator
docs. Out: application Rust, storage/schema changes, new routing or secret
managers, production operations. No new storage-backend parity work; preserve
existing stack PostgreSQL compatibility.

Ordered deliverables: **1. complete private-copy tracer → 2. failure/rotation
and compatibility matrix → 3. maintained docs and numberless ADR**. Key
contract: exactly one auth source; no file contents evaluated or built into
store artifacts; strict validation before startup and inside post-refresh
reload. Risk checks: ordinary sandboxed Caddy identity; supported refresh
capability; retained parsed authentication, not mount rollback; unchanged
isolation/ingestion.

## Task outline

- [x] 1. Deliver the smallest complete private-copy startup/reload tracer.
  - Contract: in `nix/nixos.nix`, add approved `credentialsFile` and file-only
    `credentialsFileAlgorithm`; keep inline defaults/inference. Enforce exactly
    one source and clear unsupported-systemd refusal without reading secret
    bytes in Nix. One `LoadCredential`, literal import,
    `RefreshOnReload=credentials`; strict nonempty two-token/hash validation
    before start and INSIDE `ExecReload` after refresh, before
    adaptation/submission. No privileged helper or fallback.
  - Verification: extend real `nix/checks.nix` evaluation and SQLite/bcrypt VM
    seams first: runtime-provisioned file, private read-only copy, ordinary
    Caddy start/config reload, frontend/query auth and store non-disclosure.
    Observe pinned 261.2 identity/environment/mounts under actual hardening
    before matrix expansion; source inspection is not execution proof.
  - Focused checks: `jaunder-stack-module`, `jaunder-stack-sqlite-bcrypt`.

- [x] 2. Complete negative lifecycle, rotation, algorithm, and compatibility
      proof.
  - Contract: build on task 1's real lifecycle; failed refresh/validation never
    submits replacement auth. Prior parsed auth stays active even if the
    credential mount changed. Source replacement alone is inactive; reload and
    restart rotate.
  - Verification: cover approved missing/unreadable/empty/malformed/directive/
    third-token/hash-mismatch and missing-env/copy/skipped-setup failures,
    recovery, and unsupported systemd. Exercise both algorithms and both
    rotation procedures; missing/wrong/correct HTTPS credentials across
    frontends/query/unmatched paths. Preserve inline modes, shared-Caddy reload,
    trusted proxy, native prefixes, loopback isolation, collector ingestion, and
    existing PostgreSQL stack proof.
  - Focused checks: extend task 1 checks plus `jaunder-stack-sqlite-argon2id`
    and `jaunder-stack-postgresql`; no unrelated application/browser matrix
    expansion.

- [ ] 3. Project approved contracts into enduring operator/architecture docs.
  - Contract: after spec AND outline approval, maintain `docs/DESIGN.md`,
    `CONTRIBUTING.md`, `docs/ARCHITECTURE.md`, and a numberless
    `docs/adr/drafts/` decision. Add runtime alternative without rewriting
    ADR-0196 or its inline exposure contract; do not edit generated
    `docs/README.md` or promote an ADR.
  - Verification: docs match executed provisioning, strict format/algorithms,
    root-readable source ordering, stdin hashing, reload/restart
    rotation/recovery, minimum-systemd versus pinned-proof boundary, and runtime
    autosave/admin trust. Check formatting/links and architecture draft citation
    through normal doc gates.

## Coverage and execution boundary

Spec behavioral acceptance bullets 1–3 and 5 begin in task 1; task 2 completes
all eight, including rejection, rotation, compatibility, and retained evidence.
All three enduring-deliverable bullets map to task 3. Keep proof in ignored run
storage/PR evidence; never embed runtime test secret contents in Nix fixtures.

Focused supported invocation examples (after approvals):
`devtool run -- nix build .#checks.x86_64-linux.jaunder-stack-module` and
`devtool run -- nix build .#checks.x86_64-linux.jaunder-stack-sqlite-bcrypt`;
the other named checks use the same public check-output invocation.

Before implementation, obtain outline soundness review/approval, then preserve
approved spec plus outline through normal hook-backed staging/commit. Each task
returns focused proof for Lead integrated-tree acceptance; normal precommit,
prepush and CI boundaries remain authoritative, not replaced by focused checks.
One writer/checkout executor; carry canonical policy, communication, relevant
skills/spec/outline/evidence through `files` on onward delegation. No execution,
staging, commit, metadata mutation, or ADR creation is authorized by this draft.
