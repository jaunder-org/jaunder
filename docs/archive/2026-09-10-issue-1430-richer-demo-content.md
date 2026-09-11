# Issue #1430: Richer UX sandbox demo content

## Outcome

A UX sandbox created with the `demo` profile contains richer published Posts in
both Markdown and Org. Each sandbox User owns locally served image Media that
appears in at least one of their curated Posts, while the existing timeline
volume, publication spread, and draft behavior remain representative.

## Load-bearing decisions

- This change applies only to `cargo xtask sandbox NAME --profile demo`.
  Production initialization and E2E-specific seeds remain unchanged.
- The demo retains its four existing Users, shared development password, 68-Post
  total, 60 published Posts, eight drafts, publication-time spread, and current
  Markdown/Org format totals.
- Eight published Posts receive curated bodies: one Markdown and one Org Post
  per sandbox User. Remaining published Posts and all drafts stay lightweight.
- Each curated Post has a distinct, fixture-defined topic. Across the curated
  set, bodies use headings, emphasis, links, lists, code blocks, and tables
  where the format supports them; tests pin the exact source bodies and
  structures.
- The demo contains four compact, self-authored SVG images, one owned by each
  sandbox User. Assets are deterministic, reviewable, and require no network or
  third-party license.
- Images are real local uploaded Media: each asset has durable bytes at its
  content-addressed path and an owner-specific Media record. A database-only
  placeholder or static-file shortcut does not satisfy the contract.
- Media is placed as `upload` content through the existing production placement
  path inside the staged replacement workspace before Posts are created. Post
  bodies use the returned canonical Media references and continue through the
  production renderer, sanitizer, and reference extraction invariants.
- Each User's image is referenced by at least one of that User's two curated
  Posts. The complete set exercises native image syntax in both Markdown and
  Org, and no Post relies on another User's Media ownership record.
- Sandbox replacement remains fail-safe. A Media-placement or Post-seeding
  failure must remove the unpublished replacement workspace and must not replace
  an existing named workspace with a partial demo.
- The typed fixture manifest maps each User to one Media asset and two curated
  Posts, including required source structures. Existing deterministic manifest
  assertions remain the primary contract for Users, Posts, formats, titles,
  slugs, bodies, and publication times; Media bytes and records receive
  equivalent exact assertions.

## Acceptance

- Resetting a named `demo` sandbox produces exactly four Users and 68 Posts with
  the existing published/draft and Markdown/Org aggregate counts.
- Each User has exactly one seeded local SVG Media asset with matching content
  hash, filename, `upload` source, content type, size, owner record, and bytes
  at the canonical content-addressed path.
- Exactly eight published Posts use the fixture-defined curated bodies: one
  Markdown and one Org Post for each User.
- Exact source-body and rendered-HTML assertions prove the curated set contains
  the required headings, emphasis, links, lists, code, supported tables, and
  image alternative text without raw HTML.
- Every User's image is rendered by at least one of that User's curated Posts,
  and the complete set includes rendered images originating from both Markdown
  and Org source syntax.
- Rendered Post HTML is sanitized and records the expected author-owned local
  Media reference. No source or rendered body contains an external Media URL.
- Launching the actual demo sandbox shows the richer Posts and locally served
  images while a browser smoke check rejects any non-loopback request.
- Re-running without `--reset` preserves the existing workspace; resetting
  recreates the same manifest relative to the new minute-rounded anchor.
- Deterministic test-only failure after Media placement but before Post seeding
  leaves the previously installed workspace intact and removes the unpublished
  replacement workspace, including its Media bytes and records.

## Boundaries

- No changes to standard or empty sandbox profiles.
- No changes to E2E seed helpers, production bootstrap behavior, Post formats,
  rendering syntax, sanitizer policy, or Media URL layout.
- No external images, raw HTML, audio, video, remote caching, or new upload UI.
- No redesign of sandbox Users, credentials, timelines, pagination, themes, or
  the general Media manager API unless an existing invariant cannot be honored.
- No new domain vocabulary or architectural decision is introduced; this applies
  existing sandbox, rendering, and Media ownership contracts.
