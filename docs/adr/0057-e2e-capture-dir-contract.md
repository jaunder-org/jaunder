# ADR-0057: Single `JAUNDER_CAPTURE_DIR` output-dir contract for e2e capture

- Status: accepted
- Date: 2026-07-08
- Issue: [#227](https://github.com/jaunder-org/jaunder/issues/227)

## Context

The server had a growing pile of per-stream "write this diagnostic to a file"
env vars, each read independently and each wired separately into every e2e
surface: `JAUNDER_MAIL_CAPTURE_FILE` (mailer), `JAUNDER_WEBSUB_CAPTURE_FILE`
(websub), and `JAUNDER_DIAG_LOG_FILE` (the scoped diag log,
[ADR-0049](../0049-app-driven-scoped-server-diagnostics.md)). Every new capture
stream added another `_FILE` var, another `systemd.services.jaunder.environment`
entry, another line in `flake.nix`'s `e2eRunAndCapture`, and (for diag) a
copy-out plus a filename-prefix match in the xtask artifact-lift filter.
ADR-0049 flagged this sprawl as future work. These vars are e2e-only —
production never sets them.

## Decision

Replace the three per-stream `_FILE` vars with a single **output-dir contract**:
one env var `JAUNDER_CAPTURE_DIR` names a dedicated directory into which each
capture stream writes a **well-known filename by convention** — `mail.jsonl`,
`websub.jsonl`, `diag.log`. The directory is dedicated
(`/var/lib/jaunder/capture` in the VM; a `capture/` subdir of the per-run temp
dir on the host), **not** the state root, so it holds only capture streams and
can be lifted wholesale.

- **Clean break, no shim.** The three `_FILE` vars are removed everywhere.
- **Single source of the convention.** The dir-var name and the three filenames
  live in exactly one Rust place — the `host` crate (see the host-crate-layering
  ADR) — so the server (writer), `test-support` (`reset-mail` / `capture-path`),
  the Playwright readers (via `test-support capture-path`), and the flake never
  restate a filename.
- **Injection, not env-reads-in-a-unit.** The server resolves each stream's path
  once at a composition root (`serve` for mailer/websub; the observability
  bootstrap for diag) via `host::capture::file(capture::Stream::…)` and passes
  it in; unit tests inject a path value with no process-global env.
- **Whole-dir lift as a tarball.** The e2e harness tars `$JAUNDER_CAPTURE_DIR`
  out per combo as `capture-<backend>.tar.gz` (mirroring the existing
  `playwright-artifacts` file-copy pattern — a directory-source `copy_from_vm`
  is unproven in this flake).
- **Diag/panic-hook trigger var changed** from `JAUNDER_DIAG_LOG_FILE` to
  `JAUNDER_CAPTURE_DIR` (writing `diag.log`), superseding that mechanism in
  ADR-0049.

Adding a new capture stream now needs **zero** new env-var or copy-out plumbing
— one filename constant in `host`, and the writer.

## Consequences

- One env var to set per environment; one directory (as a tarball) to copy out
  per combo. The mailer/websub JSONL are now lifted too (previously in-VM only)
  — harmless, useful for post-mortem.
- `reset-mail` and `test-support capture-path` derive their target from
  `JAUNDER_CAPTURE_DIR` and **fail loudly** (non-zero) when it is unset — an
  e2e-only misconfiguration, distinct from the server read sites where unset
  means "capture off" (production-inert).
- The in-VM zero-panic gate reads `capture/diag.log` directly, independent of
  the lift.
- Follow-up: the collector-written `otel-traces.jsonl` is **not** folded in (it
  is produced by the otel-collector, not the app, and has its own copy-out
  layout) — tracked in
  [#332](https://github.com/jaunder-org/jaunder/issues/332).
- Realizes the direction ADR-0049 flagged and matches #153's convention-over-env
  ethos.
