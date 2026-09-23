# Issue #1635 — Reflect synchronized Post audience in Org headers

## Outcome

The Emacs Protocol Client's local Org Post reflects the server-confirmed Post
audience in repeated `JAUNDER_AUDIENCE` properties after successful
synchronization. A newly published local Post no longer lacks an audience header
merely because the author relied on the server's Default Audience.

## Load-bearing decisions

- The server-confirmed complete audience set is authoritative after an ordinary
  successful create or conditional update. Materialize it in the local Org
  header in canonical order: Public, Subscribers, then Named audiences by
  ascending ID; Private is one explicit property. Preserve even narrower targets
  currently dominated by Public. Do not infer the audience from local omission,
  the server's configured default, or the effective visibility of the Post.
- A successful create or update of a draft follows the same rule. Publishing a
  Post with an explicit audience reflects the _returned_ set, not merely the
  locally requested set. An existing ID-bearing Org Post with no audience
  property receives its current server audience after a successful update;
  omission still means “preserve current audience” on the wire, not “make
  Private.”
- Pulling a server-only Post into a new Org file and explicitly refreshing a
  server-ahead Post both synthesize the complete canonical audience from the
  authenticated Member representation. Never replace an edited local file
  without the existing reconciliation safety checks.
- A rejected write, conditional conflict, or failed/blocked pull leaves local
  audience metadata untouched. A recovered create whose local Entry has changed
  since the recorded create attempt retains the author's unsent audience edits
  (including omission), remains local-ahead, and records the remote
  identity/validator for a later conditional update. A matched replay may
  install the server-confirmed audience.
- Before each publish or pull, obtain a valid AtomPub Service Document for that
  server and bind its exact Jaunder-namespace, version-1 `audience`
  advertisement to the operation. Do not infer capability from whether the
  author supplied an audience or from optional, session-warmed feature checks.
  If the capability document is unavailable or malformed, stop before the Post
  write or local pull replacement; do not guess legacy behavior.
- Preserve compatibility with servers whose valid Service Document does not
  advertise `audience`: when their successful response lacks audience
  information, keep existing local audience properties, including absence. Do
  not guess or synthesize Private. A server that advertises the feature but
  omits the complete audience on a successful response is invalid; do not
  checkpoint incomplete audience state as synchronized. Existing
  explicit-audience capability checks remain in force.
- Respect ADR-0207's repeated namespaced Atom elements and audience union
  semantics, ADR-0155's structured-over-Org precedence, and ADR-0024's
  metadata-free server body. No new wire format or audience policy is
  introduced.

## Acceptance

- A live Emacs publish test starts with no `JAUNDER_AUDIENCE`, creates a Post
  under a non-Public Default Audience, and asserts that the saved local Org file
  contains the server-returned property as well as the matching remote audience.
  Exercise both ordinary publish and draft save through the shared path.
- Live or pure tests cover an existing ID-bearing Post without a local audience
  property: a successful conditional update preserves the server's current
  audience and writes it into the file. An explicitly authored multi-target
  audience remains a complete, canonically ordered set after a successful
  update.
- The existing live server-only pull test continues asserting explicit
  `private`; a selected server-ahead refresh asserts the resulting local
  properties reflect a remote audience change without relaxing local-file
  safety.
- A recovered create with unchanged local Entry reflects the response audience.
  A recovered create with changed local audience retains that edit and the
  local-ahead marker, allowing the next conditional update to publish it without
  silently reverting to the replayed audience. A second changed-create case
  edits only the body while local `JAUNDER_AUDIENCE` remains omitted: the replay
  keeps the omission and local-ahead state until a conditional update succeeds.
- Capability evidence is obtained for ordinary create/update, server-only pull,
  and selected server-ahead refresh. Across those response-processing paths, a
  valid legacy Service Document plus absent audience leaves local headers
  unchanged; an advertising document plus absent audience fails without
  recording an audience-synchronized checkpoint; an unavailable or malformed
  document stops before Post mutation or local pull replacement.
- Rejection/412 and blocked pull do not add, delete, or alter local audience
  headers.
- Ordinary Emacs unit and live integration suites and the applicable repository
  verification gates pass; no server-side behavior or database migration is
  needed.

## Boundaries

- Do not backfill already synchronized local files in bulk or change them on a
  no-op reconcile: a successful publish/update or selected pull supplies
  authoritative audience evidence.
- Do not add Named audience discovery, audience editing UI, automatic conflict
  resolution, or a new AtomPub extension version. Do not make absence of
  `JAUNDER_AUDIENCE` mean Private or drop legacy-server compatibility.
