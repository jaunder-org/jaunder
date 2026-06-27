# Jaunder Emacs client (`jaunder.el`)

The Emacs blogging front-end for Jaunder over AtomPub. This is the Infra-unit
skeleton (issue #73): shared plumbing and pure helpers that units C (#74,
authoring/publish) and D (#75, management/reconcile) extend.

## Layout

- `jaunder.el` — the package: customization group, pure helpers, and the
  HTTP/auth/mapping seams later units implement.
- `test/` — the ERT suite (one test per pure helper).
- `scripts/run-tests.el` — batch ERT runner.
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

Both run automatically as the `ert` and `elisp-fmt` steps in `cargo xtask check`
and `cargo xtask validate`, and as the `ert-check` / `elisp-fmt-check` nix
checks. elisp is interim-exempt from the Rust coverage gate (follow-on #82);
write an ERT test for every pure mapping/transform function. See
[`docs/adr/0031-elisp-separately-tested-subproject.md`](../docs/adr/0031-elisp-separately-tested-subproject.md).
