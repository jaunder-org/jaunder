# Interactive UX Sandboxes Implementation Outline

> Execute with `jaunder-iterate`, using `jaunder-dispatch` for the two
> independent foundation slices. This outline exists because workspace
> replacement, shared/exclusive lock transitions, live executable coherency,
> signal ownership, and the xtask/`test-support` contract carry concurrency and
> storage risk.

## Scope

In:

- shared host server-session infrastructure extracted from `e2e-local`;
- disposable and named SQLite sandbox server modes;
- exact `empty`, `standard`, and `demo` profiles;
- recoverable workspace creation/reset and managed-server generation locks;
- allowlisted one-shot Jaunder commands against named workspaces;
- contributor documentation, proposed ADR, architecture projection, and real
  browser smoke proof.

Out:

- PostgreSQL sandbox lifecycle, VM persistence, detached supervision, production
  fixture surfaces, raw SQL, arbitrary seed flags, browser auto-open, workspace
  list/clone/snapshot/delete, and commands that initialize or replace storage.

## Task outline

- [x] Task 1: Extract and preserve the host server session
  - Ownership: `xtask/src/steps/host_server.rs`, the reusable server portion of
    `xtask/src/steps/e2e_local/process.rs`, corresponding callsites in
    `xtask/src/steps/e2e_local.rs`, and only the `steps::host_server`
    registration in `xtask/src/lib.rs`.
  - Contract: `HostArtifacts::prepare` returns current `jaunder`,
    `test-support`, and CSR paths for an explicit build profile;
    `HostServerSession::start(ServerSessionConfig)` returns a ready session
    carrying its discovered `base_url`. The session owns supplied
    storage/database and extra environment, stderr teeing, runtime-file/HTTP
    readiness, `stop`, `force_stop`, and `is_stopped`. Collector, trace, seed,
    and Playwright policy remain in `e2e_local`.
  - Invariant: normal and visual-update `e2e-local` retain their current build
    count, fresh storage/capture per browser, environment, step results, stderr
    capture, zero-panic verification, and teardown ordering.
  - Verification: focused xtask lifecycle tests, then a real
    `cargo xtask e2e-local` run exercising its existing Chromium path.

- [x] Task 2: Add the typed sandbox profile seed boundary
  - Ownership: `test-support/src/lib.rs`, `test-support/src/main.rs`, and only
    the minimal storage seed feature surface needed to construct the manifest
    through production render/write paths.
  - Contract:
    `target/debug/test-support seed-sandbox-profile --db <sqlite-url> --profile <standard|demo>`
    is the sole subprocess interface consumed by Task 3. It exits zero only
    after the complete approved profile succeeds and non-zero with a diagnostic
    on any failure. It seeds the exact site title, Users, roles, credentials,
    and typed Post manifest; `empty` invokes no profile command. The command
    captures one creation instant rounded to the minute before constructing the
    demo manifest. It is non-idempotent and runs only against unpublished
    workspace staging; xtask, not `test-support`, owns profile metadata.
  - Invariant: all writes use typed storage services and one write scope where a
    coherent batch requires it; no production CLI/HTTP fixture surface, raw SQL,
    duplicate renderer, or second fixture manifest.
  - Verification: test `standard` and `demo` separately through `test-support`
    parser/handler entry points and production SQLite storage reads. Standard
    asserts exactly `user`/`operator`, their roles and usable fixed password,
    exactly `site.title`, absent `site.base_url` and registration rows, and zero
    Posts. Demo asserts the complete standard state plus `alice`/`bob`, usable
    credentials for all four Users, and equality with the typed manifest across
    every title, slug, body, author, published/draft state, Markdown/Org format,
    rounded-anchor timestamp offset, distribution, and long-body case.
    PostgreSQL parity is not required because the command rejects non-SQLite
    input at its boundary.

- [x] Task 3: Implement sandbox workspace, server, and command modes
  - Depends on Tasks 1 and 2. Ownership: a cohesive new xtask sandbox module
    plus narrow `cli.rs`, `dispatch.rs`, `lib.rs`, `result.rs`, and
    dependency-manifest integration.
  - CLI contract: parse
    `sandbox [NAME] [--profile empty|standard|demo] [--reset]`; trailing args
    after `--` select an existing named workspace's command mode. Keep `NAME`
    positional. Reject every invalid profile/resume/reset combination and unsafe
    path component before filesystem or build work.
  - Workspace contract: metadata has an explicit version and profile only.
    Persistent paths are `.xtask/sandboxes/NAME`; disposable mode owns a
    `TempDir`. Creation/reset stages at `.NAME.reset-new`, retains old at
    `.NAME.reset-old`, and implements every approved lock-protected recovery
    state before normal dispatch.
  - Lock contract: acquire the server lease before a server-generation workspace
    transition. Recovery/create/reset use an exclusive workspace lock. Spawn and
    readiness remain exclusive through atomic `ready` metadata publication; the
    ready server and each accepted command hold shared access for their complete
    lifetime. Shutdown publishes `stopping`, drains, reacquires exclusive access
    while retaining the lease, then removes metadata and releases locks. Reset
    requires the lease free and the server runtime lock acquirable.
  - Coherency contract: server metadata carries a stable executable-content
    digest. Command mode builds the current CLI before shared-lock acquisition,
    admits only the approved seven top-level commands, rejects explicit database
    or storage selectors in both flag forms, and opens storage only after a live
    managed digest match or proof that no server/runtime holder exists.
  - Process contract: server mode prints URL and applicable credentials, then
    blocks in a parent signal loop. First SIGINT/SIGTERM marks stopping and uses
    bounded graceful stop; a second forces the process tree while allowing
    guards to clean control state. The first signal determines the conventional
    server exit override (`130` for SIGINT, `143` for SIGTERM); a second signal
    only escalates cleanup and does not replace that status. Command mode
    transparently inherits stdin/stdout/stderr. Extend `CommandResult` with a
    command-specific exit override so the normal report, sidecar, and
    `xtask-done` path remain intact while returning either the server's
    signal-derived status or the exact command child status; all other commands
    retain binary 0/1 semantics.
  - Verification: xtask parser tests; pure filesystem/lock/recovery tests with
    injected operations; process tests that assert cleanup and returned status
    for first and repeated SIGINT/SIGTERM; digest transition and
    migration-before-open rejection tests; real command-mode status/I/O smoke.
    Use the repository's xtask workspace test lane with
    `--manifest-path xtask/Cargo.toml`.

- [x] Task 4: Prove the browser workflows and document the interface
  - Ownership: contributor-facing documentation plus final integration fixes;
    keep the approved spec, ADR draft, and `docs/ARCHITECTURE.md` projection
    synchronized with implemented names and behavior. Do not edit generated
    `docs/README.md`.
  - Disposable proof: start empty, discover and browse the URL, interrupt once,
    observe drain, and verify storage teardown.
  - Persistent proof: create standard and login as both `user` and `operator`;
    create demo and observe public/draft representative content; mutate a Post,
    `site.title` through command mode, and Media through the browser, restart,
    and observe all three persisted.
  - Concurrency proof: while a server is live, run a matching operational
    command; reject a second server, reset, an external runtime holder, and a
    mismatched executable before database open. Run two differently named
    sandboxes concurrently on distinct discovered ports.
  - Documentation: copyable disposable/create/resume/reset/command examples,
    fixed credentials, profile contents, data/control paths, migration and
    artifact-mismatch behavior, cleanup semantics, and the Nix VM's distinct
    deployment-fidelity role.
  - Verification: focused root and xtask test lanes,
    `cargo xtask check --no-test`, the real browser smokes above, and the
    unchanged `e2e-local` path. Each completed work item reaches
    `jaunder-commit`; the hook owns the single `precommit` run, with no lint
    suppressions and no `Co-Authored-By` trailer.

## Execution order and delegation

1. Dispatch Tasks 1 and 2 in parallel. Task 1 alone adds `steps::host_server` to
   `lib.rs`; Task 2 does not touch xtask, so their file ownership remains
   disjoint. Their named `HostArtifacts`/`HostServerSession` and
   `seed-sandbox-profile` contracts are the interfaces Task 3 consumes.
2. Integrate Task 3 after both foundations are checked. The integration owner
   makes the later sandbox-specific edits to shared xtask registration/reporting
   files (`cli.rs`, `dispatch.rs`, `lib.rs`, `result.rs`, and
   `xtask/Cargo.toml`).
3. Execute Task 4 after the actual command surface is stable; documentation and
   browser evidence describe only observed behavior.

## Risk checks

- No profile seed or interrupted staging directory becomes a resumable named
  workspace without complete versioned metadata.
- Every reset interruption preserves either the old published workspace or the
  fully prepared replacement according to the approved recovery table.
- Lock acquisition and transition order is single-valued; no command can pin one
  digest and open storage under another server generation.
- A current CLI never applies migrations beneath an older or unmanaged live
  server.
- Ctrl-C cannot bypass child reap, metadata cleanup, or persistent workspace
  preservation; forced exit does not claim graceful drain.
- Operational command allowlisting is fail-closed when Jaunder gains a new
  top-level subcommand.
- Passwords appear only as the documented fixed local fixture values; command
  mode preserves interactive password prompting and does not log arbitrary argv.
- `e2e-local` remains disposable, traced, Playwright-driven, and panic-gated;
  sandbox adds no collector or automated browser policy.
- `CONTEXT.md` remains unchanged because sandbox is developer tooling, not new
  domain language. The proposed ADR and architecture projection land with the
  feature; ADR numbering and `docs/README.md` remain promoter-owned.
