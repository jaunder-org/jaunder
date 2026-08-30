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
steps in `cargo xtask check` and `cargo xtask validate` — both via
`devtool check` — and, through the same implementation, as part of the
`static-checks` Nix check (so `nix flake check` covers them too).

### Live integration tests

The `*-integration.el` suite boots a real `jaunder` server per test. It needs a
built binary, located via `JAUNDER_TEST_BINARY` (falling back to `PATH`):

```sh
cargo build -p jaunder
JAUNDER_TEST_BINARY=target/debug/jaunder \
  emacs --batch -Q -l elisp/scripts/run-integration-tests.el
```

The authoritative gate is `cargo xtask validate --no-e2e`. It builds one
hermetic `elisp-coverage-producer` VM that runs the pure and live ERT
populations once, then realizes
`$out/elisp-coverage/{lcov.info,summary.txt,status.json}`. The host consumer
reconciles its pre-test production module/form census against LCOV: every
ordinary point has exactly one LCOV record. It automatically counts as
ignored/exempt without a marker only a zero-stop form whose census contains
exactly its single synthetic opening-line point and which is `require`,
`provide`, `declare-function`, `defgroup`, or `cl-defstruct`; or `defvar`,
`defconst`, or `defcustom` with an absent, `nil`/`t`, number, string, character,
keyword, quote/function-quote, or literal vector initializer. Computed calls,
variable references, backquote/unquote, and all other evaluated or unknown
initializers remain measurable or need a trailing same-line
`;; cov:ignore: <reason>` marker with a non-empty trimmed reason. An ordinary
point or LCOV observation on a structural candidate is a guard violation.
Controlled ERT, instrumentation, or invalid-report statuses and coverage
findings fail the consumer; uncontrolled Nix or VM failures remain build
failures. Full `cargo xtask validate` inherits this verdict and does not rerun
live ERT.

## Pulled media

When a server-only Post is pulled, eligible same-instance media links are
rewritten to relative files under `local-media/<sha256>/` and their verified
bytes are retained there. These **Local Media Copies** are durable blog content,
not a cache: include `local-media/` in backups and do not expect automatic
eviction or repair. The configured root is trusted, author-owned local state;
the client rejects symlinks during creation and immediately before mutation, but
cannot defend a malicious replacement after its final check without dirfd APIs.

Markdown pull localization uses the pinned upstream `cmark-el` CommonMark
parser. It rewrites only AST-recognized link, image, and autolink destinations;
code, raw blocks, malformed link text, and other source remain unchanged. The
client maps parser block source positions back to exact source spans, so bytes
outside localized destinations are preserved. The dependency is fetched with its
upstream license notices because it is not packaged by Nixpkgs or MELPA.

The Post file is installed only after its media verifies. If a pull fails, its
Post remains server-only while already verified Local Media Copies remain safe;
rerun `jaunder-reconcile` to retry and reuse those copies.
