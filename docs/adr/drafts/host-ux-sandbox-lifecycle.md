# ADR-DRAFT: Host UX Sandboxes Own Persistent Workspace Lifecycles

- Status: proposed
- Date: 2026-09-06
- Issue: [#1395](https://github.com/jaunder-org/jaunder/issues/1395)

## Context

Jaunder needs a host-native server session that a human can keep open for
browser inspection, stop after observing a bug, and restart against the same
state. `cargo xtask e2e-local` already builds, starts, discovers, waits for, and
stops a host server, but it couples that process lifecycle to disposable e2e
fixtures, tracing, Playwright, and unconditional teardown
([ADR-0051](../0051-single-playwright-config.md)). The interactive Nix VM is the
deployment-fidelity adapter, not a persistent inner loop.

Named persistence introduces lifecycle decisions that the disposable harness did
not need: where state lives, which process owns exclusion, whether resume
mutates fixtures, and how destructive reset preserves the state under
investigation when replacement preparation fails. The server already owns an
OS-backed `<storage>/runtime.lock` and runtime-file publication
([ADR-0035](../0035-elisp-live-integration-harness.md)); test-only
out-of-process state manipulation already belongs in the `test-support` binary
and must use real storage paths rather than raw SQL
([ADR-0046](../0046-test-support-seed-binary.md)).

A lock inside a workspace cannot serialize replacement of that workspace: after
a directory rename it protects the old inode and path. Replacing a non-empty
directory is also not one portable atomic rename. Treating either property as
implicit would create a reset race or an unrecoverable missing workspace.

## Decision

Add `cargo xtask sandbox [NAME] [--profile empty|standard|demo] [--reset]` as a
foreground, SQLite-only host workflow. An omitted name uses an operating-system
temporary directory. A named sandbox owns `.xtask/sandboxes/NAME`, including its
database, configuration, Media, and profile metadata. Resume runs normal
database migrations but never reapplies or reconciles profile fixtures.

`cargo xtask sandbox NAME -- <jaunder-command>` instead runs one admitted
operational command against an existing named workspace. The wrapper supplies
the storage and database environment, forwards the terminal and exit status, and
rejects lifecycle/storage-replacing commands and explicit storage selectors.

After readiness, the sandbox supervisor publishes its executable content
fingerprint in ephemeral control metadata valid only while the server lease is
held. Server start holds the workspace lock exclusively until it atomically
publishes `ready` metadata, then downgrades to shared. A one-shot command built
from the current checkout holds that shared lock through its child lifetime and
may run concurrently only when its fingerprint matches.

Shutdown atomically marks the server `stopping`, drains the child, then
reacquires the workspace lock exclusively while retaining the server lease
before removing metadata and releasing both locks. A mismatch, a transitioning
managed server, or an external server holding only the storage runtime lock
rejects before the command opens the database. The lock transition pins the
checked server generation, preventing a new CLI from migrating storage under an
older live server.

Extract one internal xtask host server-session component. It owns current
artifact preparation, isolated environment construction, child spawn,
runtime-file address discovery, readiness, stderr mirroring, signal-aware
bounded shutdown, and fallback child-tree cleanup. `e2e-local` remains the
Playwright/tracing/disposable adapter; `sandbox` supplies persistence, profile,
reset, foreground, and user-output policy. The production server API and the
single Playwright configuration do not change.

Use three deliberately separate exclusion layers:

- xtask holds `.xtask/sandboxes/.locks/NAME.workspace.lock` exclusively for
  recovery, creation, reset, and server-generation transitions; a ready server
  and each one-shot operational command hold it shared for their full active
  lifetime;
- xtask holds `.xtask/sandboxes/.locks/NAME.server.lock` exclusively for the
  foreground server lifetime, rejecting a second sandbox server without
  preventing commands against the live workspace;
- `jaunder serve` continues to hold its existing `<storage>/runtime.lock`; reset
  requires the server lease to be unheld and non-blockingly checks that runtime
  lock, refusing to replace storage held by a managed or external live server.

Reset is a lock-serialized, recoverable two-rename protocol on sibling paths in
one filesystem, not a claim of one atomic directory replacement. The prepared
replacement is `.NAME.reset-new`; the prior workspace is `.NAME.reset-old`. Only
a fully prepared and seeded replacement advances to publication. Reset renames
current to old, then new to current; a second-rename failure immediately
restores old. The next invocation recovers interruption deterministically:
missing current plus old restores old; missing current plus new but no old
discards the unpublished creation debris; current plus old keeps current and
removes old; current plus only new keeps current and removes new. Profile
metadata moves with the workspace and therefore identifies whichever version is
published.

The xtask supervisor handles SIGINT and SIGTERM. The first signal requests the
shared bounded graceful stop and waits for drain; a second forces child-tree
termination before cleanup guards run. Ports and runtime process identity are
always ephemeral and are never sandbox metadata.

Profile mutation stays behind the existing `test-support` architectural
boundary. Typed fixture recipes may deepen that binary, but neither xtask nor
the production CLI/HTTP surface writes database tables or reimplements storage
semantics.

The initial command-mode allowlist is `site-config`, `user-create`,
`app-password-create`, `user-invite`, `smtp-test`, `backup`, and `websub`.
`serve`, `init`, `create-pg-db`, `restore`, `--storage-path`, and `--db` remain
outside the wrapper because they would bypass or replace the lifecycle it owns.

## Consequences

- Human UX inspection gains disposable and repeatable persistent states without
  turning Jaunder into a fixture server.
- `e2e-local` and sandbox process behavior cannot drift because one component
  owns spawn, discovery, readiness, and shutdown.
- A named sandbox remains recoverable after seed failure or interruption during
  reset; reset costs sibling disk space until publication and cleanup complete.
- Stable workspace/server locks and ephemeral executable metadata are additional
  control state outside each workspace. They are intentionally distinct from the
  server's storage lock and must not be folded into it. Shared workspace access
  lets the wrapper run bounded, artifact-compatible operational commands against
  a live sandbox without permitting concurrent reset.
- Named workspaces are checkout-local and can outlive schema changes. Normal
  migrations make forward reuse possible; backward reuse with older code is not
  promised.
- The Nix VM remains necessary when the question is service, filesystem,
  packaging, or deployment fidelity rather than application UX.
- This decision does not change Jaunder's ubiquitous domain language;
  `CONTEXT.md` needs no new term.
