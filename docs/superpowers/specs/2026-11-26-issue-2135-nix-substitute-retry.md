# Issue #2135: CI retries a Nix step once when a substitute download fails

## Outcome

A CI lane no longer fails when Nix can't finish downloading a substitute from
Cachix or `cache.nixos.org`. The failed Nix step runs once more, and the
download starts again from the beginning. If the second try also fails, the lane
fails. Nix's errors name the path and the cache, and `dev` says that the step
was retried. Nothing gets built from source because a download failed.

## Root cause

All 12 failures have one mechanism, which comes from Nix 2.35.6's file transfer
code (`src/libstore/filetransfer.cc`):

1. A connection fault hits every in-flight download on the runner within a
   millisecond, with `curl error: Failed sending data to the peer` or
   `Failure when receiving data from the peer`. Job 139367553983 shows 16 at
   once, twice.
2. Nix retries each one with a ranged request from the byte offset it has
   written (`retrying from offset N`).
3. Sometimes a download already has every byte. The offset then equals the NAR's
   length: Cachix NAR `c9b75982…` is 2309598 of 2309598 bytes, and the
   `cache.nixos.org` NAR for `perl-5.42.3` is 13653001 of 13653001. A
   `Range: bytes=<length>-` request can't be satisfied, so the server returns
   **416**.
4. Nix classes 4xx statuses as `Misc`, not `Transient`, so it doesn't retry. The
   path's substitution fails with
   `is required, but there is no substituter that can build it`.
5. The derivation that needs the path then stops with
   `some substitutes for the outputs of derivation … failed …; try '--fallback'`.
   The lane fails.

The NARs are intact and unchanged, and concurrent cache writes play no part. The
fault is Nix's handling of a resume that is already complete. Upstream
NixOS/nix#16722 ("finish already-completed downloads after HTTP 416") is open
and not merged.

The six example jobs failed in two places:

| Where the failure happened             | Jobs                                                   | The derivation that stopped     |
| -------------------------------------- | ------------------------------------------------------ | ------------------------------- |
| `nix develop` entering the dev shell   | 132077987962, 133042027609, 135941369307, 137822809833 | `nix-shell-env`                 |
| `dev`'s `nix build` inside an e2e lane | 135959112730, 139367553983                             | `nixos-vm`, `vendor-cargo-deps` |

## Approach and tradeoffs

**The retry rule.** A `nix` command that exits with status exactly 1 is run once
more. Nix exits 100 to 104 when a builder ran and failed, timed out or produced
a hash mismatch (`ExitStatusFlags::failingExitStatus` in `build-result.cc`). A
failed substitution sets none of those flags, so it exits 1. A real build
failure therefore never triggers a retry, and nothing reads Nix's output. Status
1 also covers other errors where no builder failed, such as an evaluation error
or a lost daemon connection. Those get the same single retry, and a persistent
one fails again on the second run. A second attempt downloads the dropped path
from offset zero. Paths that downloaded the first time are already in the store,
and an out-link is written only on success, so running the command again is
harmless.

The rule applies in two places:

1. **`dev`'s `nix build`.** Every Nix build `dev` runs goes through one
   function, `nix_build`. When its `build` step fails with status 1 and wasn't
   interrupted by a signal, `nix_build` runs it once more. Nix's stdout and
   stderr are handled exactly as they are now, for interactive and CI runs
   alike. Details:
   - Both attempts stay inside the one `build` trace span, which gains a
     `retried` attribute.
   - The start time used for cache attribution is taken before the first
     attempt.
   - The `eval` step is out of scope, since none of the failures were there.
   - Before retrying, `dev` says that Nix failed without a build failure and
     that it is retrying once.
2. **Workflow steps outside `dev`.** A new composite action,
   `.github/actions/nix-retry`, takes Nix arguments and runs `nix <args>`. If
   that exits 1, it prints a warning and runs it once more. It is a few lines of
   bash. Each job uses it in two ways:
   - **To enter its dev shell.** The job runs
     `develop .#<shell> --accept-flake-config -c true` through the action, just
     before its first `nix develop` step. That keeps the storage and resource
     baselines measured before heavy work. Every job in `ci.yml`, `mutants.yml`
     and `upgrade.yml` enters exactly one shell (`ci`, `ci-nix` or `mutants`).
     `nix develop` realises the same environment whatever its `-c` command is,
     and nothing collects garbage in between, so later steps find the shell
     already in the store.
   - **For direct builds.** The test-checks job's two direct `nix build` steps,
     `.#ux-sandbox` and `.#emacsPackages.x86_64-linux.jaunder`, run through the
     action instead of calling `nix` themselves.

Alternatives:

- **`fallback = true`.** This was rejected because Nix then builds the stopped
  derivation from source. In job 135959112730 that was `nixos-vm`, and other
  failures could land on chromium or the kernel. A build that size could exhaust
  the runner's disk or run for hours.
- **Matching Nix's `try '--fallback'` error line.** This would be more exact,
  but it means copying Nix's stderr through while scanning it. It also means
  keeping nested build-log lines from matching. The exit status gives the same
  practical result with none of that.
- **A `nix` wrapper on PATH.** The dev shells put their own `pkgs.nix` first on
  PATH (`nix/dev-shells.nix`), so `dev`'s builds would bypass the wrapper.
- **Retrying the whole lane step.** That would rerun lanes that take a long
  time, and `dev`'s own retries would stack on top of it.
- **`http2 = false`.** A completed download can still be resumed into a 416.
- **A patched Nix.** Too much to maintain for a bug with a fix pending upstream.

Accepted cost: a small retry rule we maintain until #16722 ships. A step hit
twice in a row still fails the lane, and the extra time is one more run of that
Nix step. We expect that to be rare.

## Acceptance

- `dev` tests with a fake `nix` show four things:
  - A build that exits 1 is retried once and then succeeds.
  - A second exit 1 fails the step, and `dev`'s message says it retried.
  - A build that exits 100 isn't retried.
  - A run interrupted by a signal isn't retried.
- A test runs the `nix-retry` action's command with a fake `nix` and shows three
  things: exit 1 is retried once, exit 100 isn't, and the action exits with
  Nix's final status.
- A workflow test asserts two things for every job in `ci.yml`, `mutants.yml`
  and `upgrade.yml` that runs `nix develop`. The job must first enter that same
  shell through `nix-retry`, before its first `nix develop` step. The job must
  also run no direct `nix build` step outside the action.
- The required CI gates pass.
- The root cause is posted as a comment on #2135.
- After the merge, at least 20 merge-queue runs show no lane failures with this
  signature, and any that do are explained on #2135. Retries that succeed show
  up in the log, and the post-merge note counts them. This is observed after the
  merge, not proved in the PR.

## Boundaries

- `performance.yml` and `theme-repository.yml`, which install Nix themselves,
  are unchanged. So are local developer configuration and Cachix pushes.
- `adr-promoter.yml` uses setup-ci with `nix run .#adr-promoter`, enters no dev
  shell and is out of scope.
- No `fallback` setting. No change to ADR-0267's rules for non-substitutable
  verdicts.
- One retry, not a loop. No custom download or cache logic.
