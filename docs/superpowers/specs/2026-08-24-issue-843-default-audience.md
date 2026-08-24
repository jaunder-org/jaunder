# Default Audience domain type

Issue: #843

## Outcome

Jaunder represents the instance-wide Default Audience as a closed domain value
that cannot contain a Named audience. A new Post without an explicit audience
uses the configured Default Audience; an absent or invalid stored default
resolves to `Private`.

## Load-bearing decisions

- Default Audience is distinct from the per-Post audience target. Its complete
  value set is `Public`, `Subscribers`, and `Private`; Named audiences remain
  per-author and are not valid instance-wide defaults.
- Its stored tokens are exactly `public`, `subscribers`, and `private`.
  Surrounding whitespace is invalid rather than normalized.
- Closed-token parsing, formatting, enumeration, serialization, and database
  integration follow the repository's `text_enum` convention. No bespoke Default
  Audience token matcher remains.
- Missing and unparseable stored values both resolve defensively to `Private`.
  Database access errors still propagate. This deliberately changes the prior
  `Public` fallback for unconfigured sites and corrupt legacy rows.
- The config-key registry names the Default Audience type directly and has no
  custom-parser escape for this key.
- Widening Default Audience into a per-Post audience target is exhaustive and
  infallible. The reverse conversion is neither required nor exposed because a
  Named target has no Default Audience representation.
- The payload-bearing per-Post audience target keeps its existing typed row
  representation. No new string grammar is introduced for it; its Named payload
  makes it ineligible for the unit-only `text_enum` convention.
- The storage interface exposes Default Audience at its typed getter and setter
  boundary. Wider per-Post consumers convert only where they need an audience
  target.
- Existing dependency-injection, object-safety, SQLite/PostgreSQL parity, and
  defensive config-read invariants remain unchanged.

## Acceptance

- The compiler rejects passing a Named audience target to the typed Default
  Audience setter without an explicit, impossible-by-design conversion.
- Each valid Default Audience token round-trips through the standard closed-enum
  interfaces, while unknown and whitespace-padded tokens are rejected.
- Reading an unset Default Audience returns `Private` on both storage backends.
- Reading a raw invalid or legacy value returns `Private` on both storage
  backends; database failures are not converted into a fallback.
- Setting each Default Audience value persists and reads back the same value on
  both storage backends.
- Web Post creation starts from a Private audience when the setting is absent.
- AtomPub Post creation widens each configured Default Audience to the matching
  per-Post audience target: `Public` to `Public`, `Subscribers` to
  `Subscribers`, and `Private` to `Private`.
- The site-config registry validates the three exact tokens through the declared
  type and contains no Default Audience custom validator or custom-parser macro
  arm.
- No hand-written token matching remains for Default Audience.
- Documentation distinguishes Default Audience from the payload-bearing per-Post
  audience target and reflects the closed-enum adoption.

## Boundaries

- Do not change the four per-Post audience target variants, their union
  semantics, persistence representation, or visibility resolution.
- Do not add a string parser or formatter for the payload-bearing per-Post
  audience target.
- Do not migrate or rewrite existing config rows; defensive reads provide the
  compatibility policy.
- Do not change config write validation outside the Default Audience row.
- Do not add storage schema changes, protocol fields, or new dependency seams.
