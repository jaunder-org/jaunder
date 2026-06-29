# Design: `devtool run` — a focused command runner

**Date:** 2026-06-29
**Issue:** #158
**Status:** Draft (spec)
**Relates to:** ADR-0028 (devtool/xtask boundary) — proposed supplement

## Motivation

Analysis of recent agent transcripts found that ~79% of Bash invocations carry
shell "complexity" (pipes, `$()`, redirects, `;`/`&&` chains, `nix develop -c`
wrappers). Agents add this scaffolding for three reasons, and it backfires on all
three:

1. **To witness success.** The Bash tool surfaces a pass/fail boolean (`is_error`)
   and `stderr` separately, but **no visible numeric exit code**. So agents append
   `; echo "exit: $?"`.
2. **To capture/condense output.** Agents append `2>&1 | tail`, `| rg`, `| head`.
   This **corrupts the pass/fail signal**: a failed command piped into `rg`
   (which exits 0 when it matches) makes the pipeline — and therefore
   `is_error` — report success. Observed in real transcripts
   (`gh run view … 2>&1 | rg …` reported success on an exit-1 run).
3. **To enter the toolchain.** Agents wrap gate commands in `nix develop -c`,
   which defeats the `Bash(cargo xtask *)` allowlist (the matcher sees the leading
   `nix`) and contradicts the documented bare invocation.

Separately, this scaffolding is the dominant source of permission prompts: a
"complex" command falls back to a hard-coded prompt **regardless of the
allowlist**, so the allowlist cannot fix it.

`devtool run` removes the need for the scaffolding: run exactly **one** program,
with **no shell**, and get back an honest exit code plus output parked in files.

## Goals

- One command, executed via `exec` — **no shell**, so pipes/`$()`/redirects/chains
  are structurally impossible.
- A **structured JSON result**: exit code, per-stream output paths + sizes.
- **Honest pass/fail**: the runner's own exit status equals the child's, so the
  Bash tool's `is_error` is correct by construction; no `$?` ritual.
- **Output parked, not dumped**: raw bytes go to files; only metadata returns to
  the conversation. The caller slices the files with whatever tool it likes.
- **Worktree-aware** for free: invoked through the Bash tool, it inherits the
  worktree cwd (direnv loads the flake devShell, so `devtool` is on PATH).

## Non-goals

- **Composition / data transformation.** Real pipelines (`rg … | sort | uniq`),
  `$()`, and loops stay in `ctx_execute`. Clean division of labor:
  `devtool run` = "run one process, tell me the result"; `ctx_execute` =
  "compute over data".
- **A daemon / detach mode.** The runner is synchronous. Commands that exceed the
  Bash tool's 600s cap (e.g. `cargo xtask validate`) are invoked via the Bash
  tool's `run_in_background`, which is orthogonal to the runner.
- **A new binary name.** `devtoolBin` already exists; this is a new `run`
  subcommand on it.

## Form & distribution

- A `Run` variant on devtool's existing clap `Command` enum, in a new
  `tools/devtool/src/run.rs` beside `pg.rs`.
- `devtoolBin` (flake.nix:336, `craneLib.buildPackage` over the `tools/`
  workspace) already runs from PATH inside the coverage sandbox
  (flake.nix:966). Add it to the **`default` devShell**'s PATH so direnv exposes
  `devtool` interactively → `devtool run -- …` with no wrapper.
- **Staleness:** a PATH binary reflects the last direnv/flake reload, not live
  edits to `tools/`. Acceptable for a rarely-edited runner; when developing the
  runner itself, invoke it live via `cargo run -p devtool -- run -- …`. Stamp the
  binary with a build id (git short-sha via `devtool --version`) so staleness is
  detectable. (This is the inverse of the xtask "host-only, always rebuilt live"
  rule — devtool deliberately ships as a vendored binary because it must run in
  offline sandboxes.)

## Interface contract

```
devtool run [--cwd DIR] [--timeout SECS] -- <argv…>
```

- **No shell, ever.** Everything after `--` is `Command::new(argv[0]).args(rest)`.
  Shell metacharacters in arguments are passed literally to the program.
- **cwd** defaults to the inherited working directory (the worktree); `--cwd`
  overrides.
- **stdin** = `/dev/null` (non-interactive; commands never hang waiting on input).
- **env** inherited (so the devShell toolchain, PG vars, etc. flow through).
- **--timeout SECS** optional; default none. On expiry the child is killed.

### Exit code

`devtool run` **exits with the child's exit code** so the Bash tool's `is_error`
is correct without any scaffolding, *and* prints the JSON result on stdout.

| Situation | Process exit | JSON |
| --- | --- | --- |
| Child ran, exit N | N | full `RunResult` |
| Child killed by signal | 128 + signo (conventional) | `exit_code:null, signal:"SIG…", ok:false` |
| `--timeout` expired | 124 | `ok:false, signal:"SIGKILL", timed_out:true` |
| Empty argv / shell re-entry refused | 64 | `{error, kind:"usage"|"shell_refused"}` |
| Spawn failure (e.g. ENOENT) | 64 | `{error, kind:"spawn"}` |

This is deliberately the opposite of `devtool coverage emit` (which always exits
0): `run` exists to gate, so its exit status must reflect the child.

### Result schema (Lean+)

```json
{
  "command": ["cargo", "xtask", "check"],
  "exit_code": 0,
  "ok": true,
  "signal": null,
  "duration_ms": 1234,
  "stdout": { "path": ".xtask/run/1719000000123-12345.out", "bytes": 12044, "lines": 210 },
  "stderr": { "path": ".xtask/run/1719000000123-12345.err", "bytes": 0, "lines": 0 }
}
```

`timed_out` (bool) is added only when a timeout fired.

## Component architecture

Each unit is independently testable:

1. **CLI surface** (`main.rs` + `run.rs`): clap `Run { cwd: Option<PathBuf>,
   timeout: Option<u64>, cmd: Vec<String> }`, where `cmd` is the trailing args
   after `--`.
2. **`validate_argv(&[String]) -> Result<(), RunError>`**: rejects empty argv;
   refuses shell re-entry — argv[0] basename ∈
   {`bash, sh, zsh, fish, dash, ash, eval`} or the token pair `nix develop`.
   (Deliberately *not* refused: `env VAR=x cmd` — a legitimate per-command env
   idiom, not a shell — and `xargs`, which is neutered here by the `/dev/null`
   stdin.) Pure function, no I/O.
3. **`OutputStore`**: computes `id = "<unix_millis>-<pid>"` (lexically sortable),
   ensures `.xtask/run/` exists, yields the two file paths, and prunes the
   directory to the newest `RUN_HISTORY_LIMIT` (= 50) entries by mtime
   (best-effort; ignores races and errors so concurrent runs never fail on
   pruning).
4. **`exec_capture(argv, cwd, timeout, out_path, err_path) -> Outcome`**: spawns
   the child with stdin `/dev/null`, streams stdout/stderr to the two files while
   counting bytes and lines, waits (killing on timeout), and returns
   exit/signal/duration + counts.
5. **`RunResult`** (serde): assembled from the outcome + store paths/counts,
   serialized to stdout.

**Data flow:** parse → `validate_argv` → `OutputStore::alloc` → `exec_capture`
(writes files) → assemble `RunResult` → print JSON → `exit(child_code)`.

## Error handling

Runner-level failures never masquerade as child results: they exit 64 and emit
`{error, kind}` JSON (so a caller can still distinguish "runner refused" from
"child failed"). Pruning and file-stat errors are swallowed (best-effort) and
never fail an otherwise-successful run. Timeout kills the process group to avoid
orphaned children.

## Testing

- **Unit**
  - `validate_argv`: table of accept (`cargo`, `git`, `gh`, `rg`, `nix build`,
    `emacs`, `prettier`) and refuse (`bash -c …`, `sh`, `eval`, `nix develop -c`,
    empty) cases.
  - id sortability (later id sorts after earlier).
  - prune keeps the newest N and deletes older.
- **Integration**
  - fixture program emitting known stdout + stderr + exit code → assert
    `exit_code`, `ok`, per-stream `bytes`/`lines`, and file contents.
  - `true` → exit 0, empty files, `ok:true`.
  - `false` → exit 1, `ok:false`, runner process exits 1.
  - `bash -c 'echo hi'` → refused, exit 64, `kind:"shell_refused"`.
  - `--cwd` is honored (run `pwd`-equivalent, assert).
  - `--timeout` on a sleep → killed, exit 124, `timed_out:true`.

(Note: the `tools/` workspace is excluded from the main coverage gate's source
filter, so these tests are required by CONTRIBUTING for correctness, not driven
by the coverage baseline.)

## Out-of-tool deliverables (in the implementation plan, not the binary)

1. **Flake:** add `devtoolBin` to the `default` devShell PATH; add a git-sha
   build stamp surfaced by `devtool --version`.
2. **Allowlist:** add `Bash(devtool run *)` to `.claude/settings.local.json`.
   Because there is no shell, this prefix actually binds (unlike `Bash(cargo *)`,
   which the complexity fallback defeats), and the shell-re-entry refusal makes it
   meaningfully narrower than the existing `bash *`.
3. **Docs:** a CLAUDE.md routing note — "run commands you gate on through
   `devtool run`; reserve `ctx_execute` for data computation; keep raw Bash for
   atomic mutations" — this is where the behavioral guardrail (#2 from the
   investigation) lands. Plus an **ADR-0028 supplement** recording the `run`
   subcommand and the gate-via-runner convention.

## Open questions

None blocking. The two flagged during design were resolved: child exit-code
propagation (chosen) and ADR-0028 supplement as the documentation home (chosen).
