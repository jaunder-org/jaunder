# #1236 `devtool run` processkit supervision implementation outline

> Execute with `jaunder-iterate`, using `jaunder-dispatch` when delegation is
> useful. This outline exists because a synchronous owner must safely drive and
> drop an asynchronous process-tree supervisor.

## Scope

In:

- Replace `devtool run`'s manual direct-child lifecycle with processkit 3.3.4
  behind its synchronous interface.
- Preserve the parked-file, JSON, signal, timeout, and process-exit contract.
- Add focused adapter-contract tests and perform the approved one-off descendant
  teardown probe.

Out:

- Public asynchronous or cancellation APIs.
- Shared supervision infrastructure between the separate tools and xtask
  workspaces.
- Other finite command call sites, PostgreSQL cluster lifecycle, and an ADR.
- A permanent duplicate of processkit's process-tree containment tests.

## Task outline

- [x] Replace manual run supervision with the private processkit adapter.
  - Contract: `execute(argv, cwd, timeout)` and `run(argv, cwd, timeout)` retain
    their signatures and observable result schema. The private owner keeps its
    Tokio runtime alive while starting and waiting on one `RunningProcess`;
    processkit direct truncate redirects write the allocated `.out` and `.err`
    files. `Outcome` translation preserves natural exit codes, Unix signals,
    timeout `timed_out: true`, Unix `SIGKILL` presentation, and process
    exit 124. Processkit/ Tokio dependencies and `tools/Cargo.lock` belong to
    the tools workspace.
  - Verification: focused devtool tests prove successful, non-zero, signalled,
    and timed-out child results and parked output; a disposable real-process
    probe proves timeout and unwind/drop remove a parent and descendant; the
    repository static and pre-delivery gates pass.

## Risk checks

- The `RunningProcess` drops while the owning Tokio runtime is still alive;
  unwind cannot drop the runtime first and strand processkit teardown work.
- Timeout classification comes from `Outcome::TimedOut`, while the adapter
  explicitly preserves devtool's existing Unix signal field and exit-124
  behavior.
- Direct processkit file redirects remain separate, truncate-mode, byte-exact
  artifacts; existing metadata/LF summarization and pruning stay unchanged.
- Teardown failures become runner failures rather than successful or ordinary
  child outcomes.
- Manual `try_wait`, sleep polling, direct child kill/wait, and their obsolete
  warning helper/tests are removed cleanly.
- The only production caller in `tools/devtool/src/main.rs` requires no
  migration; no documentation or CLI help changes are needed because the public
  contract is unchanged.
