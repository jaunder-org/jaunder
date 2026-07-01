# Jaunder Emacs client (`jaunder.el`)

The Emacs blogging front-end for Jaunder over AtomPub. This is the Infra-unit
skeleton (issue #73): shared plumbing and pure helpers that units C (#74,
authoring/publish) and D (#75, management/reconcile) extend.

## Layout

- `jaunder.el` — the package: customization group, pure helpers, and the
  HTTP/auth/mapping seams later units implement.
- `test/` — the ERT suite. Pure-helper tests live in `*-test.el`; server-backed
  live-integration tests live in `*-integration.el` (kept separate so the fast
  pure suite stays serverless).
- `test/jaunder-integration-helper.el` — the live-server harness
  (`jaunder-test--with-live-server`): boots a real `jaunder` server in a
  tempdir, provisions a user + app password, and tears it down (ADR-0035).
- `scripts/run-tests.el` — batch ERT runner for the pure suite (globs
  `-test.el`).
- `scripts/run-integration-tests.el` — batch ERT runner for the live suite
  (globs `-integration.el`).
- `scripts/format.el` — `jaunder-fmt-fix` / `jaunder-fmt-check` (built-in
  `emacs-lisp-mode` indentation; prettier cannot format Emacs Lisp).

## Running locally

From the repo root, inside the dev shell (`nix develop .#ci`):

```sh
# tests
emacs --batch -Q -l elisp/scripts/run-tests.el
# format check / fix
emacs --batch -Q -l elisp/scripts/format.el -f jaunder-fmt-check
emacs --batch -Q -l elisp/scripts/format.el -f jaunder-fmt-fix
```

The pure suite and format steps run automatically as the `ert` and `elisp-fmt`
steps in `cargo xtask check` and `cargo xtask validate`, and as the `ert-check`
/ `elisp-fmt-check` nix checks.

### Live integration tests

The `*-integration.el` suite boots a real `jaunder` server per test. It needs a
built binary, located via `JAUNDER_TEST_BINARY` (falling back to `PATH`):

```sh
cargo build -p jaunder
JAUNDER_TEST_BINARY=target/debug/jaunder \
  emacs --batch -Q -l elisp/scripts/run-integration-tests.el
```

In the gate it runs hermetically as the `elisp-integration` `nixosTest` check
under `cargo xtask validate` (not the fast `check --no-test` loop). See
[`docs/adr/0035-elisp-live-integration-harness.md`](../docs/adr/0035-elisp-live-integration-harness.md).

elisp is interim-exempt from the Rust coverage gate (follow-on #82); write an
ERT test for every pure mapping/transform function. See
[`docs/adr/0031-elisp-separately-tested-subproject.md`](../docs/adr/0031-elisp-separately-tested-subproject.md).
