# #1656 — Org special strings and offline code migrations

## Outcome

Org Posts display Emacs Org's default special-string typography in rendered
prose: `---` → em dash, `--` → en dash, and `...` → ellipsis. Existing Posts
receive the corrected presentation without requiring an author edit. Jaunder
gains a reusable, offline queue for future data migrations implemented in Rust
and triggered by ordinary SQLx migrations.

## Load-bearing decisions

- Fork `PoiScript/orgize` from its **`v0.10` branch** into `jaunder-org/orgize`,
  retain the `0.10.0-alpha.10` crate line, and pin Jaunder's dependency to an
  immutable fork revision. Do not start from the older branch or absorb
  unrelated upstream bugs/PRs into this change. Record the exact upstream
  `v0.10` base commit and review the complete fork diff against it. Build the
  fork hermetically in Nix as well as locally.
- The fork implements Org's default special-string export in rendered prose,
  including inline prose in headings and titles. Preserve source text and
  literal contexts (code, verbatim, link destinations, and other non-prose
  attributes). No blanket HTML post-processing or Jaunder-only textual
  substitution. The Org `#+OPTIONS: -:nil` override and other Org compatibility
  gaps are outside this issue.
- Jaunder persists rendered body HTML and rendered titles at write time. Once
  the fork is integrated, a queued `rebuild_rendered_posts` operation revisits
  **every current Post**, across formats, and updates only derivatives whose
  bytes change. This includes retained Deleted Posts but not immutable Post
  Revisions. Authored body/title, identifiers, publication state, authorship,
  and semantic edit times are unchanged; no user edit or new Post Revision is
  manufactured.
- The rebuild owns its dependent projections: reconcile any changed current-Post
  Media references, prevent stale Syndication Feed cache representations, and
  preserve the existing affected-public-feed/WebSub publish contract. Do not
  notify for byte-identical or non-public representations. Public document
  validators must reflect changed bytes.
- Introduce a durable **queue of pending operations**, not a history of
  completed operations. A SQLx migration creates the queue; **only when a Rust
  operation is needed** does a companion SQLx migration insert its operation
  name and diagnostic enqueue time. Ordinary SQL migrations require no marker,
  callback, or special ceremony. Re-enqueuing a supported operation in a later
  SQL migration is valid.
- Drain pending operations in stable insertion order (a monotonic queue key, not
  timestamp order) after SQLx succeeds and before ordinary storage handles are
  returned to serving or CLI commands. Dispatch only registered operations; an
  unknown operation fails closed. For each row, acquire a transaction, run its
  Rust operation, delete that row, and commit together. On error or process
  death, rollback leaves the row pending and startup refuses to serve;
  restarting after SQLx has become a no-op still retries the queued work. No
  intermediate partially rebuilt result may be served.
- These operations are **offline**. Operators stop older Jaunder binaries and
  other database writers before deploying; an older binary cannot participate in
  a new lock protocol. Every new-version server and CLI open for one storage
  directory takes the same exclusive `<storage>/database.lock` through SQLx and
  queue draining, releasing it before normal work. The existing server-lifetime
  `runtime.lock` retains its distinct single-server/upload-cleanup role: when
  another server holds it for that directory, a CLI may perform ordinary work if
  no queue is pending, but must refuse to drain pending work. Same-directory
  SQLite/PostgreSQL openers cannot concurrently drain; separate storage
  directories aimed at one PostgreSQL database remain an operational exclusion,
  not a second lock scheme. Backup export holds `database.lock` through its
  snapshot. Restore is not an ordinary CLI open: it refuses a live
  same-directory server via `runtime.lock` and holds `database.lock`
  continuously from before the target-emptiness check through database, theme,
  and Media restoration and validation. An opener cannot observe a partially
  restored state between the database import and Media placement. A long
  transaction performing rendering is a narrowly documented offline exception to
  ADR-0092's literal all-path SQLite write-lock discipline, not permission for
  slow online transactions.
- Move the existing startup media-reference backfill into this queue as a
  separate operation, preceding the rendering rebuild. Preserve its existing
  candidate selection and atomic reference replacement, but do not repeat it on
  every ordinary database open. Both backends use the same operation order and
  completion semantics.
- Do not use SQLx's private `_sqlx_migrations` layout, a per-operation numbered
  Rust filename, a separate version history, or a generic migration CLI. The
  queue records work to do; SQLx remains responsible for whether the enqueueing
  SQL migration has run.

## Acceptance

1. A fork-based exporter test compares Org HTML export for ordinary prose,
   nested inline prose, headings, and titles against Emacs's `&mdash;`,
   `&ndash;`, and `&hellip;` semantics, and proves code/verbatim/link
   destinations and Org source remain literal. Jaunder's rendering tests
   exercise the pinned fork (including title projection). Record the exact
   upstream `v0.10` base SHA and verify that the reviewed fork diff contains
   only this exporter change and its focused tests.
2. On both SQLite and PostgreSQL, an upgrade from a database with existing Org,
   Markdown, and HTML Posts rebuilds current rendered bodies and titles through
   the shared operation. The Org punctuation becomes visible in web and
   Syndication Feed output; unchanged projections stay byte-identical and do not
   acquire revisions or changed edit timestamps. Deleted current Posts are
   updated and historical Post Revisions stay unchanged. A separate rebuild
   fixture where newly rendered HTML changes the derived Media reference set
   proves exact replacement of current-Post references and preservation of
   revision references and Media Record ownership, not merely no change under
   the punctuation fixture.
3. The media-reference repair runs once via a pending queue row before the
   render rebuild. Reopening a fully drained database does no migration work; a
   later SQLx migration can enqueue `rebuild_rendered_posts` again without
   adding a second Rust implementation.
4. A test crashes or injects failure after SQLx commits an enqueue and during a
   Rust operation: the queue row and all of that operation's changes survive
   **only together** on success; failed attempts leave no committed partial
   operation and subsequent startup retries successfully. Unknown operations,
   failed SQLx migration, or a pending drain under another live same-directory
   server's `runtime.lock` prevent service. Test server/CLI overlap: a CLI
   without pending work still operates while a server runs, but a pending drain
   refuses while it holds the runtime lock; concurrent openers serialize through
   `database.lock`. Both backends prove ordering, transactional deletion, and
   repeat enqueues.
5. After the rebuild, affected public Syndication Feeds and their validators
   reflect current rendered bytes rather than stale cache; WebSub publish work
   follows the existing public-change rule. Pending queue rows are durable,
   portable backup data (not excluded as a regenerable cache): a backup may
   capture them before an attempt or after rollback, and restoring into the
   required exact matching schema retains them for the next open to drain. A
   backup/restore round trip cannot serve a database with undrained work. An
   export concurrent with a migration either waits for the lock and captures the
   post-migration state or captures a consistent pending pre-attempt state;
   restore cannot race the migration. A paused-after-database-import restore
   blocks another opener until its Media restoration and validation finish;
   restore refuses while a same-directory server remains live.
6. The recorded architecture explains the offline queue and its ADR-0092
   exception; the existing SQL-only migration contract remains ordinary for
   unrelated migrations. The pinned fork and Nix build resolve to the same
   revision.

## Boundaries

No source rewriting, retroactive revision rewrite, new rendering options, Org
parser upgrade, unrelated upstream fixes, blanket rerender on each startup,
per-row online work, change to backup format compatibility, or new user-facing
migration controls. If a production-scale offline rebuild cannot complete safely
as one transaction, return for a separate size/operation decision rather than
silently introducing online-style batch checkpoints or relaxing the rollback
contract.
