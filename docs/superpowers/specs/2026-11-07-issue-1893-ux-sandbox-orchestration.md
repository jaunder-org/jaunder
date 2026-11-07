# #1893: Replace xtask UX sandbox orchestration

## Outcome

Replace `cargo xtask sandbox` with one public `just sandbox ...` command and one
focused root-workspace `ux-sandbox` binary. One Tokio runtime directly owns
processkit children/groups; no xtask/private API, host/test-support package
dependency, or shared-runtime crate. Preserve lifecycle behavior and
test-support seed subprocess. Lead accepts design-level simplification, not
xtask relocation; conformance is unproven and implementation unauthorized.
Overall simplification remains binding.

## Approach and tradeoffs

### Package and front door

Add one root-workspace binary using root `Cargo.lock`; minimal direct
dependencies: processkit 3.3.4, Tokio, clap, serde/serde_json, anyhow, sha2,
tempfile. No generic library.

Just per-recipe positional arguments preserve caller argv; never interpolate
user text:

```just
[ positional-arguments ]
sandbox *args:
    #!/usr/bin/env bash
    set -euo pipefail
    cargo build --locked -p ux-sandbox
    exec ./target/debug/ux-sandbox "$@"
```

`just sandbox` is disposable empty mode; named form is
`NAME [--profile empty|standard|demo] [--reset]`; command form is
`NAME -- COMMAND [ARGS...]`. Preserve one-component lowercase ASCII/digit name
grammar, allowlist (`site-config`, `user-create`, `app-password-create`,
`user-invite`, `smtp-test`, `backup`, `websub`), reject lifecycle commands and
`--storage-path`/`--db`, and forward exact argv/status.

### Build ownership

Bootstrap publishes distinct `target/debug/ux-sandbox`, not protected e2e
Jaunder/test-support outputs, so needs no isolated cache/artifact lock.
Inspected build scripts show no protected-output mutation; verify helper output
after approval. Then hold `.xtask/host-artifacts.lock` around protected builds;
release after validation/selection, before child execution:

- Server: `just build-csr debug`, `cargo build --locked -p jaunder`,
  `cargo build --locked -p test-support`; select debug outputs.
- Command: build/select debug Jaunder only; no CSR/test-support.

Keep e2e-local lock ownership through consumer teardown unchanged. Bootstrap
adds a cold build in both modes; command mode newly builds helper before
Jaunder-only preparation.

### Lifecycle and storage

Disposable and new named workspaces always run `jaunder init` (including empty),
optionally seed, then serve. Remove disposable storage only after process-tree
reap. Resume never initializes/reseeds; normal migrations run. Reset requires an
existing name and explicit/recorded profile. Fully prepare sibling
`.NAME.reset-new`; publish through `.NAME.reset-old` using two renames and
restore old on publication failure. Recovery: no current+old restores old; no
current+new/no old removes debris; current+old keeps current/removes old;
current+new/no old keeps current/removes new. Profile metadata stays inside
published workspace.

Keep `test-support seed-sandbox-profile` subprocess with Jaunder, DB URL,
profile and storage path; preserve real-storage/CLI behavior. No crate
relationship, Media API or seed redesign. Retain name/path/symlink fences,
versioned atomic metadata, executable SHA-256 generation identity, runtime
identity, bounded readiness and current lock roles. Workspace profile metadata
stays in workspace; stable locks and ephemeral ready/stopping metadata stay
under `.xtask/sandboxes/.locks/`.

Named server lease precedes exclusive workspace transitions. Ready server and
admitted command hold shared workspace ownership. Stopped command acquires
server lease (proving none held), then workspace-exclusive recovery/admission;
validates metadata/path and acquires `<workspace>/runtime.lock`. While retaining
the server lease and runtime guard, it releases exclusive workspace ownership,
acquires shared workspace ownership, then releases the server lease. Hold
runtime+shared locks through the child. Reset requires server lease and
runtime-lock check. Live command cannot acquire lease: require ready metadata
and matching digest before storage access; hold shared workspace lock for child
lifetime, not runtime lock. Shutdown marks stopping, contains/reaps processes,
takes exclusive cleanup barrier while retaining lease, then removes
metadata/releases locks.

Duplicate only Jaunder runtime-info identity/address and HTTP readiness parsing;
xtask session API is private/entangled. Keep `host_server`, `host_signals`, and
`process` for e2e-local. Lifecycle responsibility moves, not disappears.

Remove this path's sync `Process` facade, nested runtime/background bridge and
xtask CLI/dispatch/result-report protocol. One Tokio runtime directly owns
processkit child/group wait, shutdown and reap. Unexpected parent exit cannot
release locks or clean metadata before descendants are contained/reaped. Prove
actual descendants on supported platforms; no `kill_on_drop`/parent-exit
inference or arbitrary kernel/filesystem guarantee.

Preserve initiating status (SIGINT 130/SIGTERM 143) and child status. First
signal drains; further SIGINT may force earlier; repeated SIGTERM joins drain
without escalation (ADR-0181). Separately propose automatic bounded escalation
after 10 seconds without a second signal. D1 accepts direction, not the value;
approval is required. `jaunder shut-down` retains its independent 30-second
wait/no-escalation behavior.

Remove sandbox xtask protocol, broad linkage/private adapters and sync/runtime
bridges. Costs: package/manifest, recipe, helper CLI/status/tests, artifact and
shutdown adapters, duplicate probe, cold command build. Shared e2e code remains.
Judge total ownership/coupling, not lines; reject relocation or greater
complexity.

## Acceptance

After spec/ADR and required outline approval/commit, prove:

1. Exact argv/modes/profiles/allowlist; one package/no forbidden deps/distinct
   output; mode-specific builds/locks and unchanged e2e-local ownership.
2. Focused name/path/symlink, init/resume/reset/recovery, metadata/fingerprint,
   lock, stopped/live admission, runtime identity/readiness, and seed subprocess
   behavior; no partial publication or premature cleanup.
3. Signals/status, proposed deadline, SIGINT force versus SIGTERM drain, actual
   descendant containment/reap and cleanup barrier on supported platforms; no
   broader guarantee.
4. Independent review must find lower total package/runtime complexity, not
   relocation. Current feasibility is argv/source/API/build-script inspection;
   integration proofs follow approval.

Follow CONTRIBUTING: production validation on x86_64 Linux; periodic committed
Apple Silicon macOS development checks; no ARM Linux native-delivery gate. Use
normal focused checks; no invented matrix or suppressed gates. This ownership
and concurrency/storage boundary requires an outline after spec approval; none
is created here. Approve and commit it before implementation.

## Boundaries

No implementation or tracker/staging/commit; no Media/test-support API,
production CLI/HTTP or broad host change; no framework, registry/quarantine,
control socket or universal Process Compose claim. ADR-0028 exception only;
ADR-0046/0058 and remaining ADR-0180/0181 contracts stay. Ten seconds needs
approval.
