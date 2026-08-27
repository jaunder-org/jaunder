# Issue #755: Guard AtomPub Media Deletion

## Outcome

Deleting an AtomPub media Member refuses with a clear conflict response when the
authenticated user's live Posts reference the media. That response directs the
user to Jaunder's web media library; safety refusals that the web force action
cannot bypass instead explain that deletion remains blocked.

## Load-bearing decisions

- A bare AtomPub media Member `DELETE` is always guarded. AtomPub exposes no
  force query parameter, request header, retry-confirmation behavior, or other
  Jaunder-specific override.
- This deliberately changes referenced-media deletion from unconditional
  deletion to refusal. Existing Protocol Clients need no extension knowledge:
  unreferenced deletion remains successful, while referenced deletion produces
  an actionable error they can display.
- A guarded refusal returns `409 Conflict` with an `application/problem+json`
  representation whose stable members are:
  - `type`: `https://jaunder.org/problems/media-delete-conflict`
  - `title`: `Media deletion refused`
  - `status`: the JSON number `409`
  - `detail`: one of the two exact conflict explanations below
  - `post_ids`: a unique, ascending JSON array of numeric Post IDs
- An owner-reference conflict uses the detail
  `Media is referenced by live Posts. Use Jaunder's web media library to review references and force deletion.`
  Its `post_ids` contains only the authenticated owner's live Posts that
  reference the media. Deleted Posts and Posts owned by other users are neither
  reported nor disclosed.
- A global rowless-reference safety conflict uses the detail
  `Media deletion is blocked because Jaunder cannot prove that removing this record would preserve media referenced by a live Post.`
  Its `post_ids` is empty, and it does not promise that the web force action can
  succeed.
- The web media library remains the sole force-delete surface. Its current
  confirmation and force behavior is unchanged.
- Forced web deletion continues to bypass only the owner's live-Post guard. It
  cannot bypass the global rowless-reference safety invariant from ADR-0154.
- Successful deletion remains `204 No Content`; a missing media Member remains
  `404 Not Found`.
- Unexpected storage and serialization failures retain typed internal sources
  and cross the public boundary masked, consistent with ADR-0017. Ownership
  probe failures, malformed responses, and ambiguous responses are ordinary
  fail-closed evidence uncertainty under ADR-0154 and produce the applicable
  guarded conflict rather than an internal error.

## Acceptance

- Deleting unreferenced media through AtomPub returns `204 No Content`, and a
  subsequent lookup observes that the Member is absent.
- Deleting media referenced by an owner live Post returns `409 Conflict`, does
  not delete the media, and returns `application/problem+json` with the exact
  stable values above and that Post's numeric ID.
- The owner-reference detail explicitly directs the user to Jaunder's web media
  library to review references and force deletion.
- Multiple owner live-Post references are reported as unique, ascending numeric
  IDs in `post_ids`.
- Deleted Posts do not cause owner-reference refusal and never appear in
  `post_ids`.
- Proven foreign references do not cause owner-reference refusal. Unknown or
  ambiguous ownership continues to fail closed under ADR-0154 without disclosing
  another user's Post IDs; a resulting global safety refusal returns the
  specified global-conflict detail and an empty `post_ids`.
- Both SQLite and PostgreSQL preserve identical guarded deletion behavior.
- AtomPub exposes no force override, and the existing web confirmation path can
  still force owner-reference deletion subject to global rowless-reference
  safety.

## Boundaries

- This change does not add media deletion to the Emacs client or alter any
  Protocol Client.
- It does not establish a general problem-details conversion for every AtomPub
  error; the representation is the contract for referenced-media conflicts.
- It does not change media-reference extraction, ownership probing, retention,
  or physical reclamation policy.
- It does not change Post Member deletion or other AtomPub resources.
