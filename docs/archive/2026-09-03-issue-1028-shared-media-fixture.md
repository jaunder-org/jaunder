# Issue #1028 — reuse the shared media fixture

## Outcome

The four audited web-media tests create ordinary uploaded media through the
existing shared storage fixture. Test-specific identity use comes from the
fixture's returned `MediaRef`; externally observable test behavior is unchanged.

## Load-bearing decisions

- Migrate exactly these four tests in `server/tests/web/web_media.rs`:
  `list_my_media_returns_inserted_item`, `list_my_media_with_source_filter`,
  `delete_nested_request_maps_identity_without_force`, and
  `delete_nested_request_refuses_referenced_without_force`.
- Use `storage::test_support::seed_media` for row creation and its returned
  `MediaRef` wherever a request or embedded media URL needs the created
  resource's composite identity.
- Allow unobserved hash, byte-size, content-type, and timestamp values to take
  the fixture's valid defaults. Preserve each test's filename and every value
  that contributes to an assertion or exercised request.
- Derive the referenced-media test's embedded URL from the returned identity so
  the post reference and deletion request identify the same seeded resource.
- Remove imports and setup made obsolete by the migration.
- Do not add a `MediaRecord` builder or another repository helper.
- This mechanical test refactor adds no domain terminology or architectural
  decision; domain context, ADRs, and architecture documentation remain
  unchanged.

## Acceptance

- All four audited fixtures use `seed_media`; no repeated direct `MediaRecord`
  construction or `create_media` call remains at those sites.
- Identity-sensitive requests and embedded URLs use the seeded resource's
  returned `MediaRef`.
- Existing assertions and public/test interfaces are unchanged.
- The affected focused web-media tests pass.
- `cargo xtask check` passes.

## Boundaries

- Keep the deliberately specialized fixtures in
  `delete_uses_one_global_live_ownership_snapshot`,
  `delete_refusal_reports_the_reference_snapshot_despite_a_concurrent_post`, and
  `delete_nested_request_force_can_break_owner_retained_history` explicit.
- Test-support splitting owned by #963 is excluded.
- No production behavior, storage contract, or unrelated test changes.
