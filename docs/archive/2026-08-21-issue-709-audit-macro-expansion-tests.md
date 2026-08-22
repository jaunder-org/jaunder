# Issue #709 — audit macro expansion token assertions

## Outcome

The `macros` crate's expansion tests no longer contain token-substring
assertions that can pass while the named emitted behavior is broken. The
completed sweep leaves a durable audit record so future work can see which
assertion sites were mutation-checked and why any retained discriminating
needles prove the intended emission.

## Load-bearing decisions

- Scope is the `macros` crate's tests that assert on generated tokens or macro
  diagnostics with `contains` / `!contains`; current known sites are under
  `macros/src/` and any sibling expansion tests discovered during the sweep.
- Audit means mutation-checking the emission or diagnostic behavior named by the
  assertion: break the specific generated token path, confirm the relevant test
  fails, then restore the production code before landing.
- A positive assertion must use a needle that only the intended emitted
  construct or diagnostic can satisfy. If a nearby generated construct could
  satisfy the same substring, the test must be tightened.
- A negative assertion is acceptable only when the spelling is anchored by a
  positive check nearby or when the audited mutation proves the forbidden token
  would appear if the behavior regressed.
- The durable audit record is checked into this spec file under an
  implementation-time `## Audit record` section, with one row per assertion site
  or intentionally grouped diagnostic-smoke sites.
- This work is a test-quality audit. It must not replace the token-string
  technique wholesale with a parser/AST matcher, and it must not redesign
  proc-macro code generation except where a tiny local seam is needed to make an
  existing behavior mutation-checkable.
- ADR-0062 remains the testing context: `macros` is gate-measured, so proc-macro
  behavior stays covered through in-crate unit tests that drive expansion
  functions directly.

## Acceptance

- Every token-substring assertion in `macros` tests has a checked-in audit
  record in this spec file naming the assertion site and whether its mutation
  check passed, was tightened, or was removed as redundant.
- Every assertion found vacuous is fixed with a discriminating token substring
  or deleted as redundant; diagnostic-smoke assertions get the same treatment
  when their substring can pass vacuously.
- At least one focused `macros` test command demonstrates the edited tests pass
  after production code is restored.
- For each tightened assertion, the verification notes identify the mutation
  that made the old check insufficient and confirm the new check fails against
  that mutation.

## Boundaries

- No broad rewrite from token-string assertions to `syn` parsing or AST
  matching.
- No coverage-policy, xtask, or CI changes.
- No changes to runtime crates except if needed to keep a renamed or moved macro
  test compiling.
- No new ADR unless the audit uncovers a durable testing policy change beyond
  the issue's token-substring assertion discipline.

## Audit record

Audit commands:

- Baseline/final focused crate test: `devtool run -- cargo test -p macros`
  passed after production code was restored
  (`.xtask/run/1787358033677-384788.out`).
- Mutation M1: temporarily hardcoded `NumNewtype`'s sqlx bridge to `i64` and
  changed decode conversion to an infallible wrap.
  `devtool run -- cargo test -p macros` failed exactly the three bridge
  assertions that should catch that regression:
  `num_bridge_uses_the_declared_inner_for_all_three_and_checks_bounds`,
  `num_newtype_emits_bound_checking_sqlx_bridge`, and
  `num_newtype_sqlx_bridge_uses_the_declared_inner_type`
  (`.xtask/run/1787357803999-363840.out`). Restored afterward.
- Mutation M2: temporarily hardcoded the `#[macros::server]` emitted endpoint
  and span name.
  `devtool run -- cargo test -p macros expands_to_absolute_attribute_paths_in_order_with_a_wrapped_body`
  failed (`.xtask/run/1787357850770-369757.out`). Restored afterward.

| Site                                                                                     | Audit outcome                                                                                                                                                                               |
| ---------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `id_newtype::tests::id_bridge_uses_i64_for_all_three_inners_and_wraps_infallibly`        | Kept: the `i64` Type/Encode/Decode and infallible-wrap needles name the only legal ID bridge scalar; the negative `str` needle is anchored by positive string-bridge tests.                 |
| `num_newtype::tests::num_bridge_uses_the_declared_inner_for_all_three_and_checks_bounds` | Kept: M1 failed this site at the declared-`i32` Type needle.                                                                                                                                |
| `str_newtype::tests::validating_bridge_decodes_a_borrowed_str_without_allocating`        | Kept: the borrowed-`str` Decode and `from_str(v)?` needles are unique to the validating bridge; the negative allocation needles are paired with that positive path.                         |
| `str_newtype::tests::validating_bridge_keeps_string_for_type_and_encode`                 | Kept: the exact `String` Type and annotated Encode-local needles name the two non-decode bridge inners.                                                                                     |
| `str_newtype::tests::infallible_bridge_is_untouched_on_all_three_inners`                 | Kept: the `String` Type/Encode/Decode needles name the infallible bridge, and the negative borrowed-`str` needle is positively asserted in the validating bridge test.                      |
| `server_fn` rejection tests                                                              | Grouped diagnostic-smoke sites kept: each branch-specific needle (`endpoint`, `name`, `fields`, `skip`, `web/src`, `vertical`, `submodule`) comes from its own rejection string.            |
| `server_fn::tests::expands_to_absolute_attribute_paths_in_order_with_a_wrapped_body`     | Kept: M2 failed the derived endpoint/name assertions; absolute-path ordering uses `find`, and `skip`, boundary, and `async move` name separate emitted constructs.                          |
| `sqlx_bridge::tests::type_impl_delegates_to_type_inner`                                  | Kept: fully qualified Type `type_info` and `compatible` needles name the generated Type impl.                                                                                               |
| `sqlx_bridge::tests::encode_binds_an_annotated_local_and_keeps_size_hint`                | Kept: annotated `inner` local, `encode_by_ref`, `size_hint` signature, and `size_hint(inner)` needles name distinct Encode constructs.                                                      |
| `sqlx_bridge::tests::decode_delegates_to_decode_inner_then_converts`                     | Kept: decoded-local assignment and `Ok(X(v))` conversion needles name the Decode body.                                                                                                      |
| `sqlx_bridge::tests::the_three_inners_are_independent`                                   | Kept: `String`, `&'q str`, and `&'r str` needles use deliberately different inners, so a hardcoded shared inner fails.                                                                      |
| `sqlx_bridge::tests::the_users_generics_thread_through_all_three_impls`                  | Kept: each impl-header needle carries the expected generic/lifetime merge order.                                                                                                            |
| `sqlx_bridge::tests::a_users_where_clause_survives_the_merge`                            | Kept: count assertions are substring checks over both the user's predicate and the bridge predicate; either replacement/removal changes the counts.                                         |
| `sqlx_bridge::tests::output_is_feature_gated_and_marked_derived`                         | Kept: the feature-gate needle and automatically-derived counts distinguish the default three impls from the opt-in fourth array impl.                                                       |
| `sqlx_bridge::tests::pg_array_impl_delegates_to_type_inner_when_enabled`                 | Kept: PgHasArrayType impl, `array_type_info`, and `array_compatible` needles are present only when `pg_array` is enabled.                                                                   |
| `sqlx_bridge::tests::pg_array_impl_is_absent_when_disabled`                              | Kept: the negative `PgHasArrayType` needle has the enabled-array positive companion above.                                                                                                  |
| `sqlx_bridge_derive::tests::emits_only_the_three_bridge_impls`                           | Kept: the three impl-header positives prove the bridge surface; forbidden constructor/trailer negatives are anchored by positive trailer tests elsewhere.                                   |
| `sqlx_bridge_derive::tests::all_three_inners_are_the_field_type_and_decode_moves`        | Kept: field-type Type/Encode/Decode and `Ok(Self(v))` needles distinguish the default mode.                                                                                                 |
| `sqlx_bridge_derive::tests::text_option_makes_the_column_text_and_parses_on_decode`      | Kept: `String` Type, `to_string` Encode, borrowed Decode, `parse::<SmtpPort>()`, no `map(Self)`, and stored-value diagnostics distinguish text mode from raw wrapping.                      |
| `sqlx_bridge_derive::tests::text_option_still_emits_no_constructor`                      | Kept: forbidden constructor/trailer negatives use the same anchored list as the default bridge surface test.                                                                                |
| `sqlx_bridge_derive::tests::without_the_option_every_inner_is_still_the_field_type`      | Kept: `u16` Type/Encode and `Ok(Self(v))` positives plus no `parse::<` distinguish non-text mode.                                                                                           |
| `sqlx_bridge_derive` diagnostic tests                                                    | Grouped diagnostic-smoke sites kept: `compile_error` is paired with `sqlx_bridge` or `SqlxBridge`, so the branch cannot pass on an unrelated anonymous error.                               |
| `text_enum` diagnostic tests                                                             | Grouped diagnostic-smoke sites kept: `compile_error` is paired with branch-specific needles (`bare identifier`, `string literal`, `const_into_str`, `text_enum`) where specificity matters. |
| `text_enum::tests::injects_nothing_when_the_author_wrote_all_four`                       | Kept: per-derive occurrence counts prove the macro does not duplicate author-written strum derives.                                                                                         |
| `text_enum::tests::the_parse_fn_name_is_snake_cased_from_a_multi_word_enum`              | Kept: the `__post_format_parse_err` needle uniquely names the snake-cased generated parse function.                                                                                         |
| `text_enum::tests::injects_the_four_uniform_derives_path_qualified`                      | Kept: each `::strum::…` positive needle is path-qualified to avoid satisfying on a same-named non-strum derive.                                                                             |
| `text_enum::tests::injects_the_strum_parse_err_pair_naming_the_declared_error`           | Kept: `parse_err_ty=InvalidX` and `parse_err_fn=__x_parse_err` jointly prove the declared error and generated function are wired.                                                           |
| `text_enum::tests::generates_a_unit_error_matching_the_num_newtype_precedent`            | Kept: unit-struct, derives, Display, message, Error impl, parse fn, returned error, no `thiserror`, and normalized literal needles each name the generated public error surface.            |
| `text_enum::tests::preserves_author_attributes_and_derives`                              | Kept: `VariantArray` and `serialize_all` needles prove unrelated author attributes survive expansion.                                                                                       |
| `text_enum::tests::a_same_named_derive_from_another_crate_does_not_suppress_strums`      | Kept: positive `::strum::Display` distinguishes strum injection from `derive_more::Display`.                                                                                                |
| `text_enum::tests::a_strum_qualified_derive_does_suppress`                               | Kept: negative `::strum::Display` has the injection positive companion immediately above.                                                                                                   |
| `text_enum::tests::does_not_duplicate_a_uniform_derive_written_below_the_attribute`      | Kept: occurrence count proves one author-written `::strum::Display`, not duplicate injection.                                                                                               |
| `text_enum::tests::serialize_writes_the_static_token_without_allocating`                 | Kept: `serialize_str` and `Into<&'static str>` positives anchor the no-`to_owned`/no-`clone` negatives.                                                                                     |
| `text_enum::tests::deserialize_routes_an_owned_string_through_from_str`                  | Kept: owned-`String` Deserialize and `from_str(&s).map_err` needles name both sides of the deserialize path.                                                                                |
| `text_enum::tests::sqlx_flag_emits_the_bridge_with_the_three_declared_inners`            | Kept: `String` Type, no `str` Type, static-token Encode, no `to_owned`, borrowed Decode, and `from_str(v).map_err` jointly distinguish the sqlx bridge mode.                                |
| `text_enum::tests::the_decode_error_echoes_the_offending_token`                          | Kept: normalized `format!("{e}; stored value: {v:?}")` needle names the stored-value diagnostic.                                                                                            |
| `text_enum::tests::without_the_sqlx_flag_no_bridge_is_emitted`                           | Kept: negative `::sqlx::` is anchored by the `sqlx` flag positive bridge test.                                                                                                              |
| `lib::tests` shape helper tests                                                          | Grouped diagnostic-smoke sites kept: macro-name needles come from the helper parameter; acceptance tests cover the opposite branch.                                                         |
| `lib::tests` compile-error-only option/shape rejection tests                             | Grouped diagnostic-smoke sites kept: these assert rejection of invalid fixtures; branch-specific diagnostics are covered in the per-macro parser/generator tests above.                     |
| `lib::tests::str_newtype_secret_selects_redacting_trailer`                               | Kept: `redacted` positive and no `Serialize` distinguish the secret trailer from the serde secret variant.                                                                                  |
| `lib::tests::str_newtype_secret_serde_adds_the_serde_bridge_to_the_redacting_trailer`    | Kept: `redacted` plus `Serialize` and no `Display` distinguish `secret, serde` from both default and plain secret.                                                                          |
| `lib::tests::str_newtype_infallible_emits_from_string_serde_and_omits_fallible_door`     | Kept: full-trailer positives plus no `TryFrom`/`FromStr` distinguish infallible mode; fallible-door positives exist in default/newtype tests.                                               |
| `lib::tests::str_newtype_generic_threads_the_users_generics_through_every_impl`          | Kept: generic impl-header and qualified-`Self` needles name the two #875 token-level traps.                                                                                                 |
| `lib::tests::num_newtype_min_max_default_emit_full_trailer`                              | Kept: full-trailer positives and `v < 1` / `v > 100` bound needles avoid generic-angle false positives.                                                                                     |
| `lib::tests::num_newtype_min_only_omits_max_check_and_default`                           | Kept: `v < 1` positive anchors the no-`v >` and no-Default negatives.                                                                                                                       |
| `lib::tests::num_newtype_error_message_overrides_generated`                              | Kept: custom message literal can only come from the override path.                                                                                                                          |
| `lib::tests::num_newtype_generates_public_unit_error_shape`                              | Kept: unit error type, derives, Display message, and Error impl needles name the generated error surface.                                                                                   |
| `lib::tests::num_newtype_max_only_emits_max_check_and_at_most_message`                   | Kept: `v > 100` and `at most 100` positives anchor the no-min negative.                                                                                                                     |
| `lib::tests::num_newtype_no_bounds_generates_valid_integer_message`                      | Kept: `a valid integer` positive anchors no-min/no-max negatives.                                                                                                                           |
| `lib::tests::str_newtype_default_emits_sqlx_bridge`                                      | Kept: `has_sqlx_bridge`, feature gate, and `from_str` positives name default validating storage.                                                                                            |
| `lib::tests::str_newtype_no_sqlx_omits_the_bridge`                                       | Kept: no-bridge negative is anchored by the default bridge positive; `Serialize` proves the rest of the trailer remains.                                                                    |
| `lib::tests::str_newtype_secret_omits_the_bridge`                                        | Kept: no-bridge negative is anchored by the `secret, sqlx` positive companion.                                                                                                              |
| `lib::tests::str_newtype_secret_sqlx_readds_the_bridge`                                  | Kept: `redacted`, bridge, and `from_str` positives name stored-secret mode.                                                                                                                 |
| `lib::tests::str_newtype_infallible_emits_the_infallible_sqlx_bridge`                    | Kept: bridge positive plus no `from_str` distinguishes infallible storage.                                                                                                                  |
| `lib::tests::str_newtype_infallible_no_sqlx_omits_the_bridge`                            | Kept: no-bridge negative is anchored by the infallible bridge positive.                                                                                                                     |
| `lib::tests` ordering option tests                                                       | Grouped by one option surface: positives key on method names (`fn partial_cmp`, `fn cmp`) rather than trait names, and negatives have default/infallible positive companions.               |
| `lib::tests::str_newtype_no_ord_with_secret_emits_compile_error`                         | Kept: `already unordered` pairs with `compile_error`, so this does not pass on an unrelated guard.                                                                                          |
| `lib::tests::str_newtype_infallible_no_ord_is_accepted`                                  | Kept: no `compile_error` plus no `fn partial_cmp` proves accepted unordered infallible mode.                                                                                                |
| `lib::tests::id_newtype_emits_sqlx_bridge`                                               | Kept: bridge and feature-gate positives name unconditional ID storage.                                                                                                                      |
| `lib::tests::id_newtype_sqlx_decode_is_an_infallible_wrap`                               | Kept: `sqlx :: Decode` positive anchors the no-`try_from (v) ?` negative, whose positive companion is NumNewtype.                                                                           |
| `lib::tests::num_newtype_emits_bound_checking_sqlx_bridge`                               | Kept: M1 failed this site at the `try_from (v) ?` bridge call needle.                                                                                                                       |
| `lib::tests::num_newtype_sqlx_bridge_uses_the_declared_inner_type`                       | Kept: M1 failed this site at the non-`i64` declared-inner Type needle; the no-`i64` negative catches the original vacuity trap.                                                             |
| `lib::tests::num_newtype_clamp_emits_bounds_and_clamped_constructor`                     | Kept: `const MIN`, `const MAX`, and `fn clamped` positives name all opt-in clamp artifacts.                                                                                                 |
| `lib::tests::num_newtype_without_clamp_omits_clamped_constructor`                        | Kept: no-`fn clamped` and no-`const MAX` negatives are anchored by the clamp positive test.                                                                                                 |
| `macros/tests/*.rs`                                                                      | Out of scope for token-expansion audit except as anchors for emitted trait behavior; they do not assert on stringified generated tokens or macro diagnostics.                               |
