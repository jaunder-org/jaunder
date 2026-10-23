# #1811 Host temporary-file routing — approval draft

**Stable request ID:** `issue-1811-spec-v1`  
**Status:** Proposed for human approval; implementation and tests are not
authorized.  
**Repository:** `/home/mdorman/src/jaunder/issue-1811-host-temp-routing`  
**Branch / HEAD:** `issue-1811-host-temp-routing` /
`944a9ace135a91f5d8e30d5ccf6d9b5f7a0d388a`

## Plain-language summary

Propose a checkout-local default `TMPDIR` for processes launched after entering
Jaunder's shared development shell, plus two narrow compatibility adjustments:
isolate the generated Cargo metadata package from ancestor workspace discovery,
and disable unused Unix sockets for the ephemeral devtool PostgreSQL server. The
shared directory routes files; it does not own individual runs, clean leftovers,
or promise that every tool uses the setting.

## Diagnosis and seam

The shared shell environment in `nix/dev-shells.nix` is the existing common
startup boundary for the default, CI, and mutants shells (`shellEnv`, lines
119–138; shell application, 157–166). Its `shellHook` (139–153) currently
configures platform libraries/cache and provisions Node modules; it does **not**
discover the checkout root, set `TMPDIR`, or create a temp directory. Adding a
checkout-specific value therefore entails new hook behavior, not wiring an
existing setting. A shell-entry value naturally reaches direct host tools and
their children that inherit the environment; `devtool run` itself
(tools/devtool/src/run.rs:259–272, 325–341, 367–369) does not set or clear it.
xtask-only wiring would miss direct tool invocations. ADR-0158 treats inherited
environment as immutable startup input, resolved at process boundaries.

### Proposed default contract (requires explicit approval)

At shell entry, determine the actual Git checkout top-level and use its absolute
`.xtask/tmp` directory as `TMPDIR`. This deliberately replaces any inherited
`TMPDIR` on entry to the shell, including an operator's prior setting: the
choice favors a consistent checkout-local default across supported direct
commands, CI/dev shells, and their inheriting children. A later command-local
environment override remains honored by ordinary process inheritance. An
explicit child environment replacement can still omit or replace it.

The setting is bound to the checkout detected at shell entry. Do not
automatically rebase it when the user later changes directories; users must
enter a shell from the intended checkout. The shell must not silently fall back
to `/tmp`: if checkout-root resolution fails or directory creation/use is
unusable, emit a visible diagnostic and make shell setup/command unsuccessful.
Exact shell-hook error propagation and usable-directory checks are
implementation details to specify and prove only after approval; no claim is
made that the existing hook provides them.

`.xtask/` is already ignored (`.gitignore:38–41`), and CONTRIBUTING's Artifact
lifecycle places temporary probes there. Ignore rules prevent source admission;
they do not clean files. A shared ignored parent is routing only, not run
ownership. Preserve existing unique per-run leaves and their existing cleanup
owners. Do not add a per-run manager, deletion service, collector, sweeping
cleanup, blanket unique-path guarantee, or change to #1800 leak/permission work.
Concurrent processes may share the parent; their existing unique-leaf
conventions remain responsible for collision avoidance.

### Producer compatibility dispositions (both require approval)

1. **Generated Cargo metadata:**
   `xtask/src/steps/common_host_target_closure.rs:244–263` writes a temporary
   standalone package manifest without `[workspace]`; `:268–285` obtains its
   root through `tempfile::tempdir()`. A checkout-nested temp root can cause
   Cargo to discover the checkout ancestor workspace and reject this package as
   an undeclared member. Propose adding an empty `[workspace]` table to this
   generated manifest only. This makes the generated package its own workspace
   root without changing root workspace membership/excludes. ADR-0141 preserves
   intentionally separate product, xtask, and tools workspaces; ADR-0159 defines
   the target-closure check. This is a narrow compatibility proposal, not a
   claim that it has been tested or that Cargo failure was newly reproduced
   here. The #1800 handoff records the concrete earlier failure and its specific
   recovery outside the workspace; that recovery is not a product default.

2. **Ephemeral PostgreSQL Unix sockets:** `tools/devtool/src/pg.rs:279–286` puts
   PGDATA below inherited TMPDIR, and `:121–125` currently configures Unix
   sockets there too. Long checkout paths can exceed Unix-domain socket path
   limits. Repository clients use loopback TCP (`HOST = 127.0.0.1`, pg.rs:19;
   bootstrap and `PgEnv` URLs at :65–97, :135–150, :253–259), not the ephemeral
   server's Unix socket. Propose disabling the unused socket producer-locally
   via empty `unix_socket_directories`, preserving PGDATA location,
   TCP/bootstrap behavior, `PgEnv`, startup and cleanup ownership. Before
   implementation, verify the empty setting survives `pg_ctl -o` argument
   construction/quoting. This is a compatibility disposition requiring explicit
   approval, not a discovered failing startup. Do not add an outside-checkout
   fallback or claim runtime proof.

## Scope, guarantees, and non-goals

- Applies to host processes launched from the configured shell and descendants
  that inherit its environment; explicit per-command overrides are supported.
- Does not duplicate `TMP` or `TEMP`: supplied Linux API evidence identifies
  `TMPDIR` as the relevant documented setting for Rust temp defaults, Emacs Unix
  temp initialization, and Node `os.tmpdir()` behavior. This is not a universal
  guarantee for every library/API.
- No guarantee for explicit Playwright output/profile paths, every browser
  temporary file, Nix daemon/build environment, or sandbox storage. A host
  environment cannot redirect those boundaries by assertion.
- No wrapper, generic temp framework, collector, deletion service,
  historical/global `/tmp` scan, permissions change, environment/config mutation
  outside the shell contract, or workspace membership/excludes change.
- #1800 leak/permission repairs remain independent; routing is not cleanup and
  does not fix leaks, permissions, or read-only outputs. #1753 remains deferred.
  No blocker dependency on #1800 is asserted.
- The alternative is documented per-invocation opt-in `TMPDIR`, with no built-in
  default. It is smaller and avoids replacing inherited shell policy, but
  provides weaker consistency and does not establish a consistent built-in
  routing default. Any Cargo or PostgreSQL producer repair under that
  alternative needs separate approval rather than being presumed authorized.

## Future acceptance criteria (not executed)

After approval, focused implementation and regression proof should establish:

1. Default, CI, and mutants shell entry resolve the actual checkout root, set an
   absolute `<root>/.xtask/tmp`, and create it; unusable root/directory produces
   visible failure and no silent `/tmp` fallback.
2. Inherited entry TMPDIR is deliberately replaced, while a later command-local
   override wins; direct tools and inheriting children receive the setting. A
   shell entered in checkout A remains bound to A after `cd`; entering checkout
   B creates a B-bound shell.
3. Generated metadata includes producer-local empty `[workspace]`; focused
   cargo-metadata regression succeeds with temp nesting under the checkout
   without altering root workspace membership/excludes.
4. Devtool PostgreSQL retains checkout-local PGDATA and loopback
   TCP/bootstrap/PgEnv behavior with Unix sockets disabled; verify `pg_ctl -o`
   quoting and actual startup/bootstrap plus child TCP connectivity under a
   deeply nested/long TMPDIR. This required runtime proof has not been run.
5. Existing unique run leaves and cleanup owners remain unchanged; no test
   claims routing equals cleanup or universal API coverage.

Keep future proof focused on these seams; do not run a broad workload or gate
merely to satisfy this draft.

## Evidence inventory and confidence

**Static diagnosis confidence: high for the mapped source boundaries and
supplied prior findings; no runtime verification for proposed behavior.** No
source research was repeated for this task. Reused evidence:

- `evidence:project-brief#9` — bounded assignment/scope and known Cargo failure
  context.
- `evidence:policy#28`, `evidence:communication#28` — canonical policy and
  approval routing.
- `evidence:evidence#3` — supplied issue #1811 body and evidence; issue was
  reported OPEN/unassigned, with no claim authorized.
- `result:identity#1` — preflight identity/clean state and tracker read. It
  records attempted cargo helper commands that yielded no useful helper; do not
  interpret this as verification that no compilation occurred.
- `result:intake#120`, `result:intake#121` — shared seam, source anchors,
  workspace isolation analysis, and compatibility proposal.
- `result:routing#2`, `result:temp-docs#1` — opt-in alternative and qualified
  Linux API/platform limits.
- `evidence:handoff#1` — concrete earlier #1800 Cargo ancestor-workspace
  failure; no retry or fix implied.
- `result:soundness#6`, `result:soundness#7` — PostgreSQL Unix socket
  path-length risk, loopback-TCP client mapping, and proposed disposition.
  Actual PostgreSQL startup and `pg_ctl -o` quoting remain unproved.

Canonical guidance explicitly read:
`/home/mdorman/.agents/skills/jaunder-develop/policy.md`,
`/home/mdorman/.agents/skills/jaunder-develop/communication.md`;
`CONTRIBUTING.md`, `CONTEXT.md`; ADR-0035, ADR-0141, ADR-0158, ADR-0159,
ADR-0180. ADR-0141's workspace separation and ADR-0159's target-closure contract
are respected; ADR-0158 supports inherited startup configuration. No ADR
conflict identified.

No tests, builds, gates, or workload proof were run for this draft. The identity
evidence mentions attempted cargo-helper commands but provides no useful helper
and does not support a claim of verified no compilation. No source changes,
tracker claim, commit, staging, or push is authorized.

## Approval packet

**Explicit ask:** For request `issue-1811-spec-v1`, approve or reject (a) the
shared-shell checkout-local TMPDIR default, including deliberate replacement of
inherited entry TMPDIR and shell-entry checkout binding/no rebasing; and (b)
both producer compatibility changes: generated-manifest empty `[workspace]` and
disabling the unused ephemeral PostgreSQL Unix socket while retaining loopback
TCP. The document is an approval proposal, not approved implementation. Withhold
all implementation, tests/workloads, staging, commits, tracker claim mutation,
and push until explicit approval is relayed and acknowledged by the responsible
Lead.
