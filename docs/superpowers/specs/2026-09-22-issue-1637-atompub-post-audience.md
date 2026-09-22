# Issue #1637 — AtomPub Post audience round-trip

## Outcome

The Emacs Protocol Client can publish and reconcile an explicit Post audience
without silently falling back to the instance Default Audience or preserving a
stale audience. Jaunder carries audience state as a discoverable AtomPub wire
extension, round-trips it through authenticated Member Entries, and preserves
the existing omission semantics for clients that do not supply it.

## Load-bearing decisions

- The Jaunder Atom namespace gains repeated text-valued `j:audience` elements on
  AtomPub Entries.
- Each element contains exactly one canonical audience token: `public`,
  `subscribers`, `private`, or `named:<id>`. A Named audience ID uses the exact
  lexical form `[1-9][0-9]*`, must fit in signed 64-bit range, and is never
  normalized from zero, a sign, a negative value, or leading zeros.
- Audience values are semantic set members under ADR-0020's union rule. Any
  deduplicated combination of `public`, `subscribers`, and Named audiences is
  valid. `public` makes the effective visibility Public without erasing the
  other targets, so removing it later restores the narrower union.
- `private` represents the empty target set and is valid only by itself. Empty,
  malformed, duplicate, foreign, or `private`-plus-other targets reject the
  complete write rather than being ignored or partially applied.
- Canonical output order is `public`, then `subscribers`, then Named audiences
  in ascending numeric-ID order. Every supplied target remains represented even
  when `public` currently dominates effective visibility.
- On incoming create and update Entries, `j:audience` is structured presence.
  When present, its complete set wins over any `JAUNDER_AUDIENCE` values in the
  Org metadata block. Org-header audience input remains supported for clients
  that already use the server-side Org metadata contract.
- Omitting `j:audience` preserves existing behavior: create applies the instance
  Default Audience, and update preserves the Post's current audience, unless an
  Org metadata header supplies the field under the existing structured-field
  precedence rules.
- Authenticated AtomPub Member and Collection Entries always emit the Post's
  complete current target set, including targets whose visibility is presently
  dominated by Public. Private is represented explicitly as `j:audience` value
  `private`; response omission never stands for Private.
- The Service Document advertises the additive `audience` feature under the
  existing Jaunder extension version `1`. The namespace and version do not
  change.
- A valid capability advertisement is specifically a direct Jaunder-namespace
  `j:extension` with supported version `1` and an `audience` feature token. A
  foreign-namespace element, missing or unsupported version, or absent feature
  does not advertise audience support.
- When a local Post explicitly contains `JAUNDER_AUDIENCE` but the target
  Service Document lacks that valid advertisement, the Emacs Protocol Client
  fails before mutation. RFC-compatible silent ignoring must never publish with
  unintended visibility.
- The Emacs Protocol Client maps repeated local `JAUNDER_AUDIENCE` properties to
  repeated `j:audience` elements and maps response elements back into canonical
  repeated properties.
- Audience participates in ordinary pull and reconciliation state and in the
  strong AtomPub Member ETag's canonical mutable-representation projection. An
  audience-only remote change therefore changes the ETag, contributes to
  divergence, and makes a stale conditional write fail instead of overwriting
  the change.
- Named audiences use their canonical `named:<numeric-id>` form. Friendly
  discovery, labels, or an audience picker are not part of this issue.
- This public protocol decision is recorded in
  `docs/adr/drafts/atompub-post-audience-round-trip.md` and projected into the
  architecture view.

## Acceptance

- Publishing a local Org Post with explicit `public`, `subscribers`, or
  `private` audience sends the corresponding `j:audience` value.
- Repeated local properties containing `public`, `subscribers`, and Named
  audience IDs produce the complete canonical repeated wire representation
  without collapsing targets dominated by Public visibility.
- Creating a Post with explicit audience persists that exact authorized set on
  both SQLite and PostgreSQL.
- Updating an existing Private Post with `public` makes it Public; updating a
  Post with another valid set replaces its complete prior audience.
- A create without audience still uses the Default Audience, and an update
  without audience still preserves the current audience.
- If both Atom and Org audience representations are supplied, the complete Atom
  representation wins.
- Invalid, duplicate, empty, foreign Named, noncanonical Named ID, out-of-range
  Named ID, and `private`-plus-other target combinations fail atomically with
  the existing masked AtomPub validation response.
- Member and Collection responses expose the complete target set in
  deterministic order, including explicit `private` and targets dominated by
  `public`.
- Pull reconstructs canonical `JAUNDER_AUDIENCE` properties. An audience-only
  server change alters the Member ETag, produces server-ahead or conflict
  reconciliation state as applicable, and rejects a stale `If-Match` update.
- An explicit local audience against a server without the exact supported
  namespace/version/feature advertisement fails before any Post or Media
  mutation; foreign-namespace, wrong-version, missing-version, and
  missing-feature advertisements are covered. An omitted audience remains
  compatible with that server.
- Pure Rust tests cover wire parsing, serialization, union preservation,
  ordering, precedence, exact Named ID grammar, exact capability advertisement,
  ETag projection, and validation. Existing backend-parametric integration
  coverage proves create, update, and conditional-write semantics.
- Pure ERT coverage locks Org-to-Entry and Entry-to-Org audience mapping, exact
  capability recognition and enforcement, and deterministic representation.
- Live Emacs integration coverage proves explicit create, Private-to-Public
  update, omission semantics, and round-trip reconciliation against the real
  server.
- User documentation explains the property grammar, omission behavior,
  capability requirement, raw Named audience ID limitation, and the one-time
  reconciliation rebaseline after a server upgrade. It gives actionable steps:
  fetch unchanged `server-ahead` Posts; for a conflict, preserve the local file
  outside the managed root, fetch the remote Post and its audience metadata,
  reapply the intended local edits, then publish conditionally.

## Boundaries

- No friendly Named audience discovery, completion, or selection UI.
- No new authenticated endpoint solely for audience lookup.
- No change to browser audience controls, Default Audience configuration, or
  audience membership semantics.
- No change to public Syndication Feed visibility or serialization.
- No migration or persisted-schema change: this issue changes protocol mapping
  around the existing Post audience model.
- No attempt to make third-party AtomPub clients author Jaunder audiences unless
  they implement the advertised extension or existing Org metadata contract.
- No automatic conflict resolution during the ETag rebaseline: when local and
  remote state both may have changed, the client preserves both for explicit
  review rather than guessing which content or audience should win.
