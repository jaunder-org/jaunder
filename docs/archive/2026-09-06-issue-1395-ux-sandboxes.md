# Interactive UX Sandboxes

**Issue:** [#1395](https://github.com/jaunder-org/jaunder/issues/1395)  
**Status:** Draft for approval  
**Date:** 2026-09-06

## Problem

Jaunder has no fast, human-controlled server session for browser-based UX
inspection. `cargo xtask e2e-local` owns a useful host-native server lifecycle,
but couples it to a disposable canonical fixture, Playwright, and unconditional
teardown. The interactive Nix VM provides deployment fidelity, but its tmpfs
root does not preserve a bug state after the VM exits.

Manual environment-variable orchestration leaves artifact freshness, ephemeral
port discovery, process ownership, seeding, workspace isolation, reset safety,
and cleanup to each caller. The existing e2e seed is non-idempotent and
specialized for automated tests, so it is not a persistent-workspace contract.

## Goals

- Provide a fast host-native Jaunder server for direct browser evaluation.
- Support both throwaway sessions and named workspaces that survive invocations.
- Cover unconfigured, minimal login, and content-rich UX states with coherent
  profiles.
- Ensure reopening never reseeds or reconciles profile data implicitly; normal
  database migrations remain part of startup.
- Preserve the exact `e2e-local` lifecycle and Playwright behavior while sharing
  the underlying host server-session machinery.
- Make process ownership, concurrent access, reset, and cleanup deterministic.

## Non-goals

- PostgreSQL sandbox lifecycle or persistent PostgreSQL cluster management.
- Persistent disks or lifecycle changes for the interactive Nix VM.
- A production server mode, detached daemon, or background supervisor.
- A production fixture endpoint, raw-SQL seeding, or arbitrary scenario flags.
- Workspace listing, cloning, snapshots, or explicit deletion commands.
- Automatically opening or controlling the user's browser.

## Command contract

The public interface is:

```text
cargo xtask sandbox [NAME] [--profile empty|standard|demo] [--reset]
cargo xtask sandbox NAME -- <jaunder-command> [arguments...]
```

`NAME` is an optional positional workspace name. Omitting it creates a
disposable sandbox. Supplying it addresses a persistent workspace under
`.xtask/sandboxes/NAME` in the current checkout.

A workspace name is one safe path component: lowercase ASCII letters, digits,
`-`, and `_`, beginning with a letter or digit. Paths, separators, `.` and `..`
are rejected before filesystem mutation.

`--profile` defaults to `empty` only when creating a disposable or previously
absent named workspace. An existing named workspace is resumed with
`cargo xtask sandbox NAME`; passing `--profile` without `--reset` is rejected,
even if it matches the recorded profile.

`--reset` requires `NAME`. Reset without `--profile` reapplies the workspace's
recorded profile. Reset with `--profile` replaces the workspace using the newly
selected profile. Reset of an absent workspace is rejected rather than treated
as creation.

Trailing arguments select one-shot command mode instead of starting a server.
Command mode requires an existing named workspace and is incompatible with
`--profile` and `--reset`. It builds the current cached `jaunder` CLI artifact,
sets the workspace's storage and SQLite database environment, inherits stdin,
stdout, and stderr, and returns the child's exit status. If a managed server is
live, command mode proceeds only when that server's executable content
fingerprint matches the newly built CLI; otherwise it rejects before opening the
database and instructs the user to restart the sandbox.

The admitted top-level Jaunder commands are `site-config`, `user-create`,
`app-password-create`, `user-invite`, `smtp-test`, `backup`, and `websub`.
Lifecycle-owning or storage-replacing commands (`serve`, `init`, `create-pg-db`,
and `restore`) are rejected. Explicit `--storage-path` and `--db` arguments,
including `--flag=value` forms, are rejected so the wrapper cannot silently
target data outside the named sandbox.

Every server-mode invocation prepares current cached CSR, server, and
test-support artifacts before startup. Every command-mode invocation prepares
the current cached `jaunder` CLI artifact. There is no stale-artifact fast path
in the initial interface.

## Runtime experience

The command binds Jaunder to an ephemeral loopback port, discovers the actual
address through the server's runtime file, waits until the server is reachable,
and prints the browser URL. It remains in the foreground until interrupted.

The xtask supervisor handles SIGINT and SIGTERM rather than relying on default
parent termination. The first signal invokes the shared bounded graceful-stop
path and waits for the server to drain. A second signal forces child-tree
termination, then lets workspace cleanup guards run. The command exits with the
conventional signal-derived status after cleanup.

A persistent workspace remains valid and restartable after either path; its
runtime file is process identity, not persistent state. A disposable workspace
is removed after a graceful first-signal shutdown.

The command prints fixed profile credentials on every start of a `standard` or
`demo` workspace. It does not launch a browser.

## Workspace model

A named workspace is one self-contained, gitignored directory containing its
SQLite database, configuration, Media, and small sandbox metadata. The metadata
records the profile used for the most recent successful creation or reset; it
does not record a port, PID, start time, or live runtime file contents.

Two stable per-name xtask locks live outside the replaceable workspace:

- `.xtask/sandboxes/.locks/NAME.workspace.lock` is exclusive during recovery,
  creation, reset, and managed-server generation transitions; a ready server and
  each one-shot command hold it shared for their full active lifetime;
- `.xtask/sandboxes/.locks/NAME.server.lock` is exclusive for the complete
  foreground server lifetime.

The workspace lock prevents reset from racing with a server or operational
command, while the server lease rejects a second sandbox server for the same
name. Commands may run against the live server's workspace, and differently
named sandboxes remain fully isolated and may run concurrently.

A managed server acquires its server lease, holds the workspace lock exclusively
through spawn and readiness, atomically publishes
`.xtask/sandboxes/.locks/NAME.server.json` with state `ready` and its executable
content fingerprint, then downgrades the workspace lock to shared. Command mode
builds first, acquires the workspace lock shared, and holds it through the
complete child lifetime. It proceeds against a live managed server only when the
metadata is `ready` and its fingerprint matches.

On shutdown the supervisor atomically marks the metadata `stopping` while still
holding its shared lock, so new commands reject. After the server drains it
reacquires the workspace lock exclusively, removes the metadata, releases the
server lease, and finally releases the workspace lock. The exclusive transition
pins one server generation across every accepted command and closes the
fingerprint-check time-of-check/time-of-use race.

A held server lease without valid `ready` metadata reports that the server is
starting or stopping and asks the caller to retry. A held storage runtime lock
without a matching managed server lease is treated as an external server and
command mode rejects. When no server is live, the current command may open and
normally migrate the workspace; a subsequent server waits for the command's
shared lock, then starts from the same current checkout.

The server continues to own its existing `<storage>/runtime.lock`. Before
resetting, xtask requires the server lease to be unheld and non-blockingly
checks the runtime lock, refusing to mutate a workspace held by either a managed
or external live server. The xtask workspace lock, xtask server lease, and child
runtime lock are distinct: the first serializes data replacement, the second
pins one sandbox supervisor generation, and the third protects Jaunder storage
while `serve` runs.

Normal Jaunder database opening applies pending migrations when a persistent
workspace is resumed. Resume does not rerun profile creation, reconcile fixture
content, or restore records a human changed while investigating UX.

Creation and reset use deterministic sibling paths under `.xtask/sandboxes`
while holding the control lock: `.NAME.reset-new` for a prepared replacement and
`.NAME.reset-old` for the prior workspace. A partially created or seeded
workspace is never published under `NAME`.

After successful replacement preparation, reset renames `NAME` to the old path
and the new path to `NAME`. If the second rename fails, it immediately restores
the old path. On the next invocation, lock-protected recovery resolves an
interrupted sequence: `NAME` absent plus old restores old; `NAME` absent plus
new but no old discards the unpublished creation debris; `NAME` plus old keeps
the published replacement and removes old; `NAME` plus only new keeps `NAME` and
removes new. All paths share one parent filesystem. This is a serialized,
recoverable two-rename protocol, not an unsupported claim that replacing a
non-empty directory is one atomic filesystem operation.

A preparation or seed failure removes the new path and leaves the old workspace
addressable and unchanged. The replaced workspace is removed only after the new
workspace is published.

## Profiles

### `empty`

A migrated, runnable SQLite workspace with no Users and no explicit site
configuration. This is the initial-setup and unconfigured-state surface.

### `standard`

The sole explicit site setting is `site.title = "Jaunder Sandbox"`.
`site.base_url` remains unset because the loopback port changes between runs,
and registration retains its existing closed default rather than storing the
default redundantly. The profile creates:

- ordinary User `user`;
- operator User `operator`;
- fixed password `jaunder-dev` for both.

No Posts are created. The two roles permit comparison of ordinary and operator
navigation without adding content density.

### `demo`

`demo` retains the complete `standard` state, then adds ordinary author Users
`alice` and `bob`, also with password `jaunder-dev`. It creates exactly 60
published Posts and eight drafts, evenly distributed across the four Users:

- per User, 15 published Posts: 12 Markdown and three Org;
- per User, two drafts: one Markdown and one Org;
- published timestamps use unique one-day offsets 1 through 60 before one
  profile-creation timestamp rounded to the minute;
- each User has one 12-paragraph long Markdown Post; remaining bodies rotate
  through stable short and medium fixture text.

The resulting 60 published Posts exceed the default 50-item timeline page and
exercise timeline, archive, detail, authoring, draft, length, date, author, and
both user-authored source-format surfaces. Titles, slugs, bodies, author
assignment, timestamp offsets, publication state, and format distribution are
defined by one typed fixture manifest in `test-support`.

Counts are not public CLI parameters. Profile creation uses production
CLI/storage APIs exposed through Jaunder and `test-support`; it never writes
database tables directly.

## Architecture boundary

Extract one host server-session component from `e2e-local`. It owns artifact
paths, isolated storage/database environment, process spawning, runtime-file
address discovery, readiness, stderr mirroring, bounded shutdown, and fallback
cleanup.

`e2e-local` remains an adapter that supplies a disposable workspace, the
canonical e2e seed, tracing collector, Playwright invocation, and unconditional
teardown. `sandbox` supplies its workspace policy, profile seeding, one-shot
operational command adapter, user-facing output, and foreground lifetime.
Neither adapter reimplements process control.

The shared component is internal xtask infrastructure, not a new Jaunder server
API. Existing runtime-file and startup-mutex behavior remains authoritative. The
durable workspace, split lock ownership, and recoverable reset protocol are
recorded in `docs/adr/drafts/host-ux-sandbox-lifecycle.md`, preserving ADR-0035,
ADR-0046, and ADR-0051.

## Failure behavior

Build, validation, lock, migration, seed, startup, readiness, and shutdown
failures return a non-zero xtask result with the workspace name and failed
phase. No failure is converted into a successful session. Seed failures never
publish partial named workspaces. Shutdown failures are reported even when
fallback cleanup reaps the child.

Disposable workspace cleanup is best-effort after abnormal process death, as it
is for other operating-system temporary directories. Named workspace data is
never deleted outside a successful explicit reset.

## Verification and documentation

- Parser tests cover positional names, defaults, invalid names, resume/profile
  rejection, reset combinations, command-mode selection, the operational command
  allowlist, and storage-selector rejection.
- Workspace tests cover create, resume without reseed, profile recording, stable
  workspace/server lock placement, live command access, same-name server
  exclusion, different-name isolation, reset exclusion while a server or command
  holds shared access, runtime-lock reset refusal, managed-server fingerprint
  match/mismatch, external-server rejection, atomic ready/stopping transitions
  and generation pinning, every interrupted creation/reset recovery state, and
  failure-safe reset.
- Profile tests assert exact site configuration, Users, roles, fixed login
  credentials, fixture-manifest Posts, drafts, formats, dates, and the
  timeline-page threshold through production storage reads.
- Lifecycle tests cover runtime discovery, parent-owned signal handling,
  graceful and forced stop, stale runtime cleanup, and reuse by `e2e-local`
  without duplicating process-control assertions.
- A focused interactive smoke starts a disposable empty sandbox, reaches its
  URL, sends the first interrupt, and observes server drain and workspace
  teardown.
- A real `standard` smoke logs in through the browser as both fixed accounts.
- A real `demo` smoke observes representative published and draft content
  through the browser.
- A named restart smoke changes database content and site configuration, uploads
  Media, restarts the command, and verifies all three persisted through the real
  browser surfaces.
- Command-mode smokes change `site.title` while the named server is live,
  observe the change in the browser, prove stdout/stderr and a non-zero child
  status are forwarded without rewriting, and prove an artifact mismatch rejects
  before a pending migration can run.
- Existing `e2e-local` coverage remains green, including tracing and Playwright.
- Contributor documentation gives copyable commands for disposable, named,
  standard, demo, resume, and reset use; documents credentials and data
  location; and reserves the Nix VM for deployment-fidelity evaluation.

## Acceptance

The issue is complete when every workflow and failure invariant above is
implemented, all smoke paths above are exercised against a real server, current
`e2e-local` behavior remains intact, and contributors can reproduce all three
profiles from documented commands.
