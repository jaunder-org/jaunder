# Issue #1490: Fix demo Post image links

## Outcome

A newly created or reset `demo` sandbox serves every image referenced by its
curated Posts from its own local Media store. The existing demo content remains
intact while the seed manifest accurately describes all seeded Media.

## Load-bearing decisions

- The change applies only to `cargo xtask sandbox NAME --profile demo` and its
  test-support seed contract.
- Preserve the current four Users, 73 Posts, and Alice-owned
  `baseline-seeded.txt` Media Record.
- Add the four existing self-authored SVG fixtures as local uploaded Media, one
  owned by each demo User. The complete demo therefore contains five Media
  Records; Alice owns two and every other User owns one.
- Place each SVG through the production Media placement path before creating
  Posts. Build curated Post source from the canonical URL returned by that path;
  fixture code must not independently hard-code or rederive content-addressed
  URLs.
- Each of the eight curated Posts keeps its existing native Markdown or Org
  source shape and refers only to its author's SVG Media Record.
- The versioned cross-process sandbox seed manifest describes every seeded Media
  Record as a collection. Its schema advances to version 2 and serializes
  `media` as an array; profiles with no Media serialize an empty array.
- Sandbox replacement retains ADR-0180's staged-publication behavior. A failure
  while placing Media or creating Posts must not publish a partial replacement
  or damage an existing named workspace.
- Existing rendering, sanitization, Media-reference extraction, canonical Media
  identity, and strict serving behavior remain authoritative.

## Acceptance

- Resetting or creating the SQLite `demo` sandbox yields exactly four Users, 73
  Posts, and five Media Records. The test-support demo seed contract yields the
  same counts on both SQLite and PostgreSQL.
- Every demo User owns the expected SVG Media Record with the fixture filename,
  SHA-256 digest, `image/svg+xml` content type, byte length, and exact durable
  bytes. Alice's baseline text Media Record remains unchanged, including its
  deterministic profile-anchor `created_at` value.
- Each curated Markdown and Org Post retains its expected source body and
  renders an image whose canonical root-relative URL identifies its author's SVG
  Media Record.
- Stored Post-to-Media references match the rendered image references and never
  transfer ownership between Users.
- Requests to all four curated image URLs through the public Media route return
  success, `image/svg+xml`, and the exact fixture bytes.
- The demo seed command emits manifest version 2 with all five Media entries;
  Media-free seed manifests emit an empty `media` array.
- Deterministic failure coverage proves a failed demo replacement publishes no
  partial new workspace and preserves any existing named workspace.
- Targeted regression coverage proves the profile and serving behavior without
  adding a dedicated browser smoke test.

## Boundaries

- Do not remove or rewrite the five newer demo Posts or the baseline text Media
  fixture.
- Do not change empty or standard sandbox content beyond the versioned manifest
  representation of an empty Media collection.
- Do not change Post rendering, HTML sanitization, Media URL/path layout,
  Media-reference policy, the public Media endpoint, or production bootstrap.
- Do not add external images, new Post formats, new upload UI, or general Media
  behavior.
- No new domain vocabulary or architectural decision is introduced; this work
  restores the existing sandbox, Media, and Post contracts.
