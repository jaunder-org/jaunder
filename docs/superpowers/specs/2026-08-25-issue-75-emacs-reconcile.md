# Issue #75 — Emacs blog-directory reconciliation

## Outcome

Deliver the epic design's Unit D as four review-sized children: inventory one
configured root against its AtomPub Collection, pull server-only Posts, report
without resolving divergence, and delete only by explicit command.

## Load-bearing decisions

- #75 is an administrative umbrella. D1–D4 each own one implementation slice,
  test set, branch, and PR; #75 closes only after all four merge.
- **D1 — inventory:** exhaust Atom `rel="next"` pagination; parse numeric Post
  ID/edit URI and `j:slug`; scan only root-level `.org`; join on `JAUNDER_ID`.
  Return local drafts, server-only Posts, matches, orphans, and conflicts
  without side effects. Duplicate local IDs or target slugs are conflicts, never
  chosen.
- **D2 — pull:** replace `jaunder--atom->org` with deterministic Member Entry →
  Org synthesis built on `jaunder--harvest-response-fields`; D2 alone owns this
  seam. Untitled Posts emit required empty Atom `<title>`, not a slug fallback.
- GET each server-only Member for its strong ETag; write exactly `<slug>.org`
  with native body and server media URLs. ID comes from numeric edit URI, slug
  from `j:slug`, sync marker from ETag, and sync time from the current UTC wall
  clock. An occupied path blocks without overwrite or suffix.
- Header order: optional `#+TITLE`, `#+DATE`, `#+KEYWORDS`, `#+DESCRIPTION`;
  `JAUNDER_STATUS`; optional `JAUNDER_DATE_TZ`, `JAUNDER_DATE_UTC`; then
  `JAUNDER_FORMAT`, `JAUNDER_SLUG`, `JAUNDER_ID`, `JAUNDER_SYNCED`,
  `JAUNDER_SYNCED_AT`; blank line; body. Title/categories/summary supply
  optional fields; empty title is omitted and category order is preserved.
- Map `text/org`→`org`, `text/markdown`→`markdown`,
  `html`/`xhtml`/`text/html`→`html`. Drafts omit dates. Otherwise preserve
  offset-qualified `atom:published` verbatim as `JAUNDER_DATE_UTC`, capture the
  current IANA zone, render `#+DATE` there, and classify future as scheduled,
  present/past as published.
- **D3 — reconcile:** `jaunder-reconcile` obtains D1's inventory and matched
  Member ETags, then keeps `*Jaunder Reconcile*` available with counts/rows for
  unchanged, server-ahead, local-ahead, conflict, unclassifiable, orphan, local
  draft, server-only, and inventory-conflict. The last names duplicate kind and
  paths/slugs, distinct from two-sided divergence.
- Local change means mtime exceeds `JAUNDER_SYNCED_AT` by over two seconds;
  server change means current ETag differs from `JAUNDER_SYNCED`. Invalid or
  missing markers are unclassifiable with a reason and no offer. Server-ahead
  reports pull guidance and local-ahead publish guidance, never batch actions.
- Preview exactly server-only pulls, confirm once, then have D2 recheck every
  destination and apply that set. Cancellation or application leaves the report
  available; matched divergence remains report-only within reconcile.
- **D4 — explicit deletion:** `jaunder-delete-post` requires a visited file,
  valid `JAUNDER_ID`, and strong quoted non-weak `JAUNDER_SYNCED` before prompt
  or I/O. After confirmation it conditionally DELETEs with that ETag. Only `204`
  removes file and buffer; cancellation, `404`, `412`, validation, or transport
  failure preserves both.
- Network work uses the active blog from `jaunder--with-blog` and existing `plz`
  transport. Reconcile never publishes drafts or auto-resolves divergence.

## Acceptance

- **D1:** a multi-page Collection plus root-level draft, match, orphan,
  duplicate ID, duplicate slug, and nested `.org` yields every inventory class,
  exhausts pagination, and ignores the nested file.
- **D2:** an untitled server-only Org Post has an empty Atom title, pulls to its
  exact slug path without `#+TITLE`, preserves body/media URLs, and has exact
  ordered header bytes and values from the specified wire/time sources; an
  occupied path remains byte-identical and is blocked.
- **D3:** fixtures covering all four mtime/ETag combinations, invalid markers,
  inventory conflicts, and every inventory-only class produce exact
  states/counts and one-sided guidance; cancel changes nothing, while confirm
  creates only previewed destinations.
- **D4:** a valid synced Post receiving `204` disappears on both sides;
  cancellation, missing/malformed ETag, `404`, and `412` leave local bytes and
  the visited buffer intact.
- Pure ERT covers parsing, joins, mapping, validation, and status branches; live
  ERT covers pagination, pull, reconcile application, and two-sided deletion. D2
  additionally covers the Rust AtomPub title mapping on shared backends.

## Boundaries

- No recursive scan, automatic pull of server-ahead matches, automatic publish,
  merge/last-write-wins resolution, hand-deletion propagation, media download or
  localization, destination suffixing, or unconditional deletion fallback.
- The optional Post-title policy is already settled; this unit adds no ADR.
