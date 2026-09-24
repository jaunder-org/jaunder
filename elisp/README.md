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
- `jaunder-post-link.el` — bidirectional Local Post Link mapping: exact local
  evidence joins the read-only Collection inventory; publish substitutes
  server-advertised alternate URLs only in the sent body, and pull restores
  relative destinations only from unambiguous current proof.
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

## Post audience metadata

An Org Post may declare its explicit audience with repeated file properties:

```org
#+PROPERTY: JAUNDER_AUDIENCE public
#+PROPERTY: JAUNDER_AUDIENCE subscribers
#+PROPERTY: JAUNDER_AUDIENCE named:42
```

Accepted values are `public`, `subscribers`, `private`, and `named:<id>`, where
`<id>` is a positive canonical decimal integer (no sign, zero, or leading
zeros). `public`, `subscribers`, and any number of distinct Named audiences
compose as a union. `private` is the empty target set and must appear alone.
Jaunder writes server-confirmed properties after a successful create, draft
save, conditional update, or pull, in canonical order: Public, Subscribers, then
Named IDs ascending. This includes a server-selected non-Public Default Audience
when the local create omitted `JAUNDER_AUDIENCE`.

Omitting every `JAUNDER_AUDIENCE` property is intentional compatibility
behavior, not Private: create uses the server's Default Audience and update
preserves the Post's current audience. Use an explicit `private` property when
that is the intended target set. Named audiences currently require their raw
numeric IDs; the Emacs client has no discovery or friendly-name picker.

Every publish or pull requires a valid Service Document for that operation.
Explicit audience properties require the `audience` feature on the exact Jaunder
extension namespace at version `1`. Missing or malformed service evidence stops
synchronization before Post mutation or local pull replacement. An advertising
server must return the complete audience in its Member Entry; a missing value is
an error, not an invitation to guess. A valid legacy server without audience
support may omit that value; in that case existing local audience headers remain
untouched (including on a selected server-ahead refresh). On server-only pull,
there are no local headers to retain.

An uncertain create retains its durable request intent. If an edit changed the
request before a recovered create response arrives, the client checkpoints the
server's identity and ETag but preserves the unsent local audience edit—even
when the author omitted an audience while changing only the body. It remains
local-ahead until an explicit conditional update succeeds; failures and ETag
conflicts never rewrite the local audience.

### One-time ETag rebaseline after upgrading

Audience now contributes to every strong AtomPub Member ETag. After upgrading a
server, previously synchronized Posts therefore show one expected ETag mismatch:

1. Run `M-x jaunder-reconcile`.
2. For an unchanged `server-ahead` Post, mark it and press `f` to fetch the
   remote representation. This installs explicit canonical audience properties
   and the new ETag.
3. For a true `conflict`, review both versions and explicitly choose `l` to keep
   the local authored Post, `r` to keep the remote Post, or `e` for a
   single-Post two-way Ediff merge. Consider a backup before discarding authored
   work. After any blocked or uncertain outcome, refresh and review again.

Do not resolve a conflict by deleting `JAUNDER_SYNCED`, guessing which side
wins, or blindly replacing the remote audience. Reconciliation requires an
explicit review when both local and remote state may have changed.

### AtomPub ETags behind an encoding proxy

The client asks for `Accept-Encoding: identity` on its AtomPub requests, and
Jaunder marks these responses `Cache-Control: no-transform`. A reverse proxy
must honor that directive or exclude `/atompub/*` from encoding. In particular,
Caddy's `encode zstd gzip` can append `-zstd` to a strong Post ETag; that coded
validator is not a valid `If-Match` for Jaunder's canonical Post. The client
does not remove suffixes from ETags or silently repair older `JAUNDER_SYNCED`
markers: a failed conditional write still requires explicit reconciliation
rather than guessing which side changed. See the
[operator encoding guidance](../docs/DESIGN.md#atompub-encoding-and-conditional-writes).

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
root. Opening it or refreshing with `g` shows a fetching/classifying message
before synchronous work begins; it does not run in the background. A Jaunder
AtomPub Collection advertises each Post's strong Member ETag as a read-only
`j:etag` Entry element (`https://jaunder.org/ns/atompub`), with the
`member-etag` Service Document feature. The Emacs Protocol Client uses a valid
element to classify matched Posts without fetching each Member; older servers
and unusable metadata fall back to Member reads. A report is only a preview:
operations still perform their own remote and local safety checks. It never
chooses a direction or mutates either side on its own. Press `m` on a row to
toggle its mark. Alternatively, make an active contiguous region over report
rows; the region takes precedence over marks for the next command. The report
keeps display order, so every selected batch has a predictable order.

Use `g` to refresh the report from current local and remote state, `p` to push
selected local drafts or safely local-ahead Posts, `f` to fetch selected
server-only or safely server-ahead Posts, `l` to keep the local authored version
of a true conflict, `r` to keep its remote version, `e` to merge exactly one
conflict through two-way Ediff, and `D` to delete selected remote Posts. Refresh
keeps marks for rows that remain, removes marks for rows that do not, restores
point when its row remains, and retains the ordered **Last batch** summary. Each
transfer command shows its selected count and asks once before its first
mutation. Delete has a distinct `SOFT-DELETE` confirmation that shows fresh
reviewed ETags. It creates Jaunder's retained deletion tombstone rather than
physically erasing the remote Post; a matched local file is removed only after
the server confirms deletion, while deleting a server-only Post has no
local-file effect.

A selection does not bypass safety checks. Unchanged Posts are no-ops; a true
`conflict` is a uniquely matched Post whose local source and remote Member both
changed since synchronization. Ordinary `p` and `f` do not resolve it. `l` and
`r` run selected conflict rows in display order after one direction-specific
confirmation. For each row they recheck the reviewed local path, bytes, ID and
clean visiting buffer, plus the fresh remote Member identity and strong ETag.
`l` conditionally publishes with that reviewed `If-Match` without writing local
Post metadata before the PUT. `r` stages Member and Local Media Copies before
its final checks and atomic replacement. Other states, duplicate identities,
stale ETags, occupied paths, and changed local files are blocked rather than
being adopted as a new baseline. A failure in one row does not undo earlier
successes or stop later eligible Posts; cancellation takes effect only between
Posts.

For `e`, select exactly one conflict row. Two-way Ediff compares read-only
snapshots of the actual local and staged remote Post; **Ediff's merge output**
is the independent editable Org scratch. Copy either side's hunks through Ediff
or edit its authored fields (title, body, summary, tags, audiences, date and
publication state) directly. There is no saved common content ancestor.
Identity, slug and sync markers in scratch are ignored and restored from the
reviewed local Post. Exiting Ediff **never** publishes. In the scratch,
`C-c C-c` explicitly confirms completion after fresh local and remote checks;
`C-c C-k` cancels but retains the scratch; `C-c C-d` discards it only after
confirmation. Killing the scratch buffer also asks before discarding it. An
initial staging or Ediff setup failure opens no finishable scratch. If Ediff
fails after creating its C result, that result remains available for inspection
or explicit discard but cannot be published; reopen a fresh reconciliation
report and merge session after fixing Ediff. Once a two-way result is open,
edits survive cancellation, blocked completion, an unknown remote outcome, or a
partial commit so they can be inspected later.

A rejected conditional PUT changes neither Post. Uploaded Media or verified
Local Media Copies may persist even if a later Post action blocks. If a PUT
response is lost, **remote outcome unknown** means the remote Post may have
committed; the local Post is not checkpointed and the request is not retried
automatically. Refresh and inspect both sides before another choice. A confirmed
remote commit followed by failed local installation, write-back or rename is
**partial success**: inspect the local path and remote Member, then reconcile;
do not assume either side rolled back. For `r`, an atomic replacement followed
by a failed rename leaves the ID-bearing updated Post at its old path.

After an operation the report rebuilds its inventory and classification while
retaining an ordered **Last batch** summary. If that refresh fails, the old
report and terminal results remain visible; use `g` to refresh before deciding
anything else. Use the refreshed report to review local and remote effects and
retry only rows whose actionable failure has been resolved. Retried creates
reuse their recorded create intent until their server-confirmed Post ID is
written locally, so an interrupted create does not create a duplicate.
