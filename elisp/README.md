# Jaunder Emacs Protocol Client (`jaunder.el`)

The Emacs Protocol Client for Jaunder over AtomPub. This is the Infra-unit
skeleton (issue #73): shared plumbing and pure helpers that units C (#74,
authoring/publish) and D (#75, management/reconcile) extend.

## Nix package

The flake exposes this Protocol Client as `emacsPackages.${system}.jaunder` for
every system that `flake-utils.lib.eachDefaultSystem` supports. It is a
standalone Emacs package derivation, version `0.1.0`, intended for an
installed-package list rather than as a server deployment package. For example,
a Home Manager configuration can install it with:

```nix
programs.emacs.extraPackages = epkgs: [
  inputs.jaunder.emacsPackages.${pkgs.system}.jaunder
];
```

The package contains the production `elisp/*.el` Protocol Client modules rooted
at `jaunder.el`; it excludes `elisp/test/`, `elisp/scripts/`, and project
documentation. Its transitive Emacs-package dependencies are Nixpkgs's packaged
`plz` and Jaunder's pinned `cmark`. Nixpkgs's `plz` supplies its immutable Nix
store reference to the `curl` executable, so no separate curl installation or
PATH setup is needed. The distinct `packages.jaunder` flake output remains the
deployable Jaunder server binary.

## Layout

- `jaunder.el` — the package entry point and dependency assembly.
- `jaunder-inventory.el` — Collection and local Post identity inventory shared
  by reconciliation and Local Post Link mapping.
- `jaunder-post-link.el` — Local Post Link publish preflight: exact local
  evidence joins the read-only Collection inventory and only sent-body links
  receive server-advertised alternate URLs.
- `test/` — the ERT suite. Pure-helper tests live in `*-test.el`; server-backed
  live-integration tests live in `*-integration.el` (kept separate so the fast
  pure suite stays serverless).
- `test/jaunder-integration-helper.el` — the live-server harness
  (`jaunder-test--with-live-server`): the batch runner shares one real `jaunder`
  server, provisioned with a user + app password, across the suite. A standalone
  interactive ERT invocation falls back to a throwaway tempdir server for that
  test's dynamic extent (ADR-0035).
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

The `*-integration.el` runner boots one real `jaunder` server for the full
suite. Individual tests use a per-test server only when run independently from
an interactive ERT session. The suite needs a built binary, located via
`JAUNDER_TEST_BINARY` (falling back to `PATH`):

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

## Reconciliation batches

`M-x jaunder-reconcile` opens a persistent inventory report for the configured
root. It only classifies Posts; it never chooses a direction or mutates either
side on its own. Press `m` on a row to toggle its mark. Alternatively, make an
active contiguous region over report rows; the region takes precedence over
marks for the next command. The report keeps display order, so every selected
batch has a predictable order.

Use `g` to refresh the report from current local and remote state, `p` to push
selected local drafts or safely local-ahead Posts, `f` to fetch selected
server-only or safely server-ahead Posts, and `D` to delete selected remote
Posts. Refresh keeps marks for rows that remain, removes marks for rows that do
not, restores point when its row remains, and retains the ordered **Last batch**
summary. Each transfer command shows its selected count and asks once before its
first mutation. Delete has a distinct `SOFT-DELETE` confirmation that shows
fresh reviewed ETags. It creates Jaunder's retained deletion tombstone rather
than physically erasing the remote Post; a matched local file is removed only
after the server confirms deletion, while deleting a server-only Post has no
local-file effect.

A selection does not bypass safety checks. Unchanged Posts are no-ops, while
conflicts, duplicate identities, stale ETags, changed local files, occupied
paths, and rows unsafe for the chosen direction are reported as blocked. Each
Post is independent: a failure does not undo earlier successes or prevent a
later eligible Post from running. The executor can be cancelled only between
Posts, so already completed work remains durable and untouched Posts remain
unchanged.

After a batch completes or is cancelled, the report rebuilds its inventory and
classification while retaining an ordered **Last batch** summary. Use that
refreshed report to review the local and remote effects and retry only the rows
that remain eligible or whose actionable failure has been resolved. Retried
creates reuse their recorded create intent until their server-confirmed Post ID
is written locally, so an interrupted create does not create a duplicate.
