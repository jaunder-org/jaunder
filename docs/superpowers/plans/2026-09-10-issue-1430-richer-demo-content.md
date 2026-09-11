# Richer UX sandbox demo content implementation outline

> Execute with `jaunder-iterate`, using `jaunder-dispatch` where task ownership
> is independent. This outline exists because Media placement spans filesystem
> and database writes while named-workspace publication must remain atomic.

## Scope

In:

- The `demo` profile manifest, its SQLite seeder, and named-sandbox
  orchestration.
- Four deterministic SVG uploads and eight curated Posts using native Markdown
  and Org.
- Exact fixture, storage, rendering, reset, and browser-smoke verification.

Out:

- Empty/standard profiles, E2E fixture helpers, production initialization,
  renderer or Media URL changes, and general Media-manager redesign.

## Task outline

- [x] Task 1: Define the exact per-User fixture manifest
  - Contract: one static `SandboxUserFixture`-shaped record owns the existing
    User attributes, one deterministic `SandboxMediaAsset`-shaped SVG, and one
    Markdown plus one Org curated Post template. Templates accept only the
    canonical URL returned for their sibling Media asset; all other Post
    generation remains unchanged.
  - Contract: the eight curated sources are deterministic `String` values.
    Across the set they exercise the approved structures without raw HTML or
    external Media URLs, while preserving four Users, 68 Posts, 60 published
    Posts, eight drafts, 52 Markdown Posts, 16 Org Posts, and the existing
    timestamp spread.
  - Verification: exact fixture and manifest assertions pin SVG bytes,
    filenames, source bodies, titles, slugs, formats, publication times, and
    aggregate counts before storage orchestration is involved.

- [x] Task 2: Materialize owner-linked Media and Posts
  - Contract: extend the internal `seed-sandbox-profile` boundary with the
    staged workspace storage path; open SQLite through
    `open_existing_database_with_observer` to obtain the persisted `InstanceId`,
    then construct `MediaContentLocks` at that storage root and `MediaManager`
    with a test-support-private `SandboxMediaOwnershipResolver`. The resolver
    implements the existing trait without server dependency; upload does not
    invoke ownership resolution, and no general Media-manager API changes.
  - Contract: use three sequential write phases: commit site configuration and
    all Users first; call and confirm `MediaManager::upload_bytes` once per
    typed SVG fixture second; then materialize each User's two templates only
    from that upload's canonical URL and commit all Posts as one batch. Never
    hold the User/Post `WriteScope` across an upload. These intermediate commits
    are safe because the seeder runs only inside `.NAME.reset-new`; abort on
    every failed or indeterminate outcome.
  - Verification: exact SQLite assertions cover owner, source, filename, content
    type, size, hash, canonical path, and stored bytes for all four fixtures.
    Manifest equality and rendered-output assertions pin required HTML
    structures, image alternative text, sanitization, and extracted owner Media
    references from both Markdown and Org. Backend parity is not required
    because the sandbox command rejects PostgreSQL and the production upload
    path already owns cross-backend behavior.

- [x] Task 3: Preserve atomic create, resume, and reset transitions
  - Contract: `xtask/src/steps/sandbox.rs` passes the staged workspace as
    `--storage-path` only to non-empty profile seeding; existing workspaces
    still resume without reseeding, and reset publishes only after
    initialization, Media placement, Post creation, and metadata writing all
    succeed.
  - Contract: a private seeder implementation accepts a `#[cfg(test)]` phase
    hook immediately after all real `MediaManager::upload_bytes` calls and
    before Post materialization; the production wrapper supplies no hook. A test
    triggers a sentinel failure there and proves real Media rows and bytes exist
    while Posts do not. The xtask fake-support seam then exits at the equivalent
    phase so orchestration can prove cleanup without adding a runtime fallback
    or user-facing option.
  - Verification: the seeder phase test proves the injected failure boundary,
    while transition tests prove resume retention, successful reset replacement,
    and failed reset preservation of the old workspace with complete removal of
    `.NAME.reset-new`; removing that staged tree removes both its database Media
    records and Media subtree.

- [ ] Task 4: Exercise the installed demo in an actual browser
  - Contract: create/reset a named `demo` sandbox through `cargo xtask sandbox`,
    authenticate with existing credentials, and inspect curated Markdown and Org
    detail pages. Browser request observation must reject any non-loopback
    request.
  - Verification: observe rendered rich structures and successfully loaded local
    SVGs, then rerun without `--reset` to prove workspace retention and reset
    once more to prove deterministic fixture recreation relative to the new
    rounded anchor. Run `devtool run -- cargo xtask check` before each kept
    commit through `jaunder-commit`.

## Risk checks

- Do not represent Media with database-only rows or static assets; bytes and
  records must come from `MediaManager::upload_bytes` under the staged storage
  root.
- Do not attempt one cross-resource database transaction: production upload
  placement owns short registration transactions, while named-workspace staging
  owns publication atomicity.
- Do not create Posts until all four uploads have returned canonical URLs; abort
  on indeterminate or failed mutation outcomes.
- Preserve existing renderer, sanitizer, Media URL, ownership, profile metadata,
  and recovery contracts; update all callsites of the internal seed boundary.
- Keep fixture data offline and reviewable: self-authored SVG bytes only, no
  external Media URLs, raw HTML, audio, or video.
- Review sandbox-facing documentation for claims affected by the richer demo;
  otherwise leave unrelated docs unchanged.
