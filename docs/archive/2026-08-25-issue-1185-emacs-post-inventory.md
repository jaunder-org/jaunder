# Issue #1185 — Enumerate and join local and AtomPub Posts

## Outcome

Provide a side-effect-free inventory of one configured Jaunder directory and its
authenticated AtomPub Collection. The inventory exhausts Collection pagination,
joins root-level Org files to Posts by `JAUNDER_ID`, and exposes every input as
one unambiguous ordinary class or inventory conflict for D3 to report.

This child implements D1 of the approved
[issue-75 contract](2026-08-25-issue-75-emacs-reconcile.md) and the epic
design's Unit D section.

## Load-bearing decisions

- Resolve the configured root through the shipped directory-to-blog contract and
  perform Collection requests only inside `jaunder--with-blog`; authentication,
  URL construction, and HTTP status behavior remain owned by the existing
  transport.
- Start at authenticated `GET /atompub/{username}/posts`. Each page must contain
  zero or one Atom `rel="next"` link. Follow it exactly as returned, track
  fetched page URIs, and stop when absent; multiple or repeated/cyclic next
  URIs, non-2xx pages, or malformed required Member data fail without a partial
  result.
- Each Member has exactly one `rel="edit"` link whose path is the configured
  Collection path plus one non-empty decimal Post ID, and a non-empty `j:slug`.
  Duplicate server IDs are malformed Collection data. Successful parsing is
  read-only and preserves page/Entry order.
- Scan only regular `.org` files directly in the configured root. Do not descend
  into subdirectories; nested configured roots reconcile independently.
- Read `JAUNDER_ID` through the existing Org property reader. Absence means
  local draft. A present value must be non-empty decimal digits; otherwise the
  file is an invalid-local-ID inventory conflict. A unique valid ID on both
  sides is matched; local-only is orphan; server-only is server-only.
- Duplicate local IDs conflict, naming the ID and every path. Duplicate target
  slugs conflict, naming the slug and every Member. Conflicts own all implicated
  local files and server Members, including counterparts joined by ID;
  overlapping duplicate conditions merge into one connected conflict group. An
  input owned by a conflict never appears in an ordinary class. Inventory
  conflict remains distinct from D3's later two-sided divergence state.
- The result is pure inventory data: enumeration and joining do not create,
  modify, rename, visit, publish, pull, or delete local files or mutate Posts.
- No ADR is required: this child applies the approved Unit D contract through
  existing Atom, Org, blog-resolution, and transport boundaries.

## Acceptance

- Pure ERT parses the exact edit-link grammar, `j:slug`, and zero/one
  `rel="next"`; multiple pages preserve order until next is absent.
- Pure ERT partitions fixtures into local-draft, server-only, matched, orphan,
  invalid-local-ID conflict, duplicate-local-ID conflict, and
  duplicate-target-slug conflict. Overlapping duplicate kinds merge, and every
  local file and Member occurs exactly once across ordinary classes/conflicts.
- A root fixture containing a draft, match, orphan, duplicate ID, invalid ID,
  and nested `.org` reports root-level classes and excludes the nested file.
- Error cases prove non-2xx pagination, malformed/multiple edit links, duplicate
  server IDs, malformed Member fields, multiple next links, and next-link cycles
  terminate without returning a partial inventory.
- Live ERT creates uniquely identified Posts exceeding the default page size and
  proves each created ID appears exactly once through the real transport while
  allowing unrelated Members in the shared Collection.
- The D1 PR lands this child spec and the already-approved shared issue-75 spec.

## Boundaries

- No pull, Org synthesis, reconciliation report/UI, ETag/mtime classification,
  publish guidance, or deletion; D2–D4 own those behaviors.
- No recursive scan, automatic conflict selection, offset pagination, local
  mutation, server mutation, or new protocol extension.
