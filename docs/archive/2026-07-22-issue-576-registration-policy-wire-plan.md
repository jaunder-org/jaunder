# Plan — #576: `RegistrationPolicy` enum on the wire

Spec: [2026-07-22-issue-576-registration-policy-wire-spec.md](2026-07-22-issue-576-registration-policy-wire-spec.md)
· Issue [#576](https://github.com/jaunder-org/jaunder/issues/576)

## Review header

**Goal.** Teach `StrEnum` to default multi-word variant tokens to `snake_case`
(scene-setting), then move `RegistrationPolicy` from `storage` into a new
`common::registration` module carrying the `StrEnum` trailer (serde, **no rename**),
return it typed from `get_registration_policy()`, and delete the two `"invite_only"`
string comparisons in `web/`.

**Scope.**
- **In:** `macros` `StrEnum` snake_case default + coverage + ADR-0074 amendment; new
  `common::registration` module + type-behavior tests; `storage` re-export (delete
  the hand-rolled dup, keep `load_registration_policy` + dual-backend tests); `web`
  typed return + two typed consumer rewrites.
- **Out:** `host::metrics::RegistrationPolicy` (separate `enum_attr!` enum with a
  `CliBypass` variant); the `storage/src/atomic.rs` `"open"` DB-seed write.

**Tasks.**
0. `macros`: `StrEnum` default token → `snake_case` (scene-setting); ADR-0074 +
   coverage.
1. Create `common::registration::RegistrationPolicy` (StrEnum + serde, no rename)
   with type-behavior tests.
2. `storage` re-uses it: delete the dup, re-export, keep `load_registration_policy`
   and its dual-backend tests.
3. `web`: typed return + rewrite the two literal comparisons.

**Key risks / decisions.**
- The `snake_case` default changes **zero** existing tokens (all five prior adopters
  are single-word). Task 0 adds multi-word coverage proving `InviteOnly` →
  `invite_only`, not `inviteonly`.
- Task 0 must precede Task 1: with the old default, Task 1's rename-free enum would
  emit `inviteonly` and its token test would fail.
- `api.rs` imports `RegistrationPolicy` from `storage` (server-gated); the ungated
  `common` import needed for the typed return collides (E0252) — Task 3 drops it.

**For agentic workers:** execute with **jaunder-iterate**; delegate via
**jaunder-dispatch** if useful.

## Global constraints

- No `Co-Authored-By` trailer.
- Each task: `cargo xtask check` (fmt + clippy + coverage) clean before commit; the
  pre-commit hook enforces it. Final task closes with `cargo xtask validate --no-e2e`.
- Follow `CONTRIBUTING.md` (backend parity, coverage policy, import discipline).
- Preserve the `"open"` / `"invite_only"` / `"closed"` wire+DB tokens exactly.

---

## Task 0 — `StrEnum` defaults to `snake_case` (macros)

**Files**
- `macros/src/str_enum.rs`: replace the `variant_wire` fallback
  `v.ident.to_string().to_lowercase()` with `to_snake_case(&v.ident.to_string())`;
  add the `to_snake_case` helper (lowercase; insert `_` before each non-first
  uppercase letter). Update the module/`variant_wire`/`collect_variants` doc
  comments (`lowercased` → `snake_case`).
- `macros/src/lib.rs`: update the derive doc; add unit test
  `str_enum_multiword_variant_uses_snake_case` (drives `expand` with
  `Policy { Open, InviteOnly, Closed }`, asserts `"invite_only"` present and
  `"inviteonly"` absent — this covers the new conversion at runtime, since
  integration enums expand at compile time invisibly to coverage).
- `macros/tests/str_enum.rs`: add a `Policy { Open, InviteOnly, Closed }` fixture +
  `multiword_variants_snake_case_their_tokens` asserting `as_str`/parse round-trip.
- `docs/adr/0074-str-enum-trailer.md`: amend the token-default bullet
  (`lowercased identifier` → `snake_case`, note #576 as the first multi-word user).

**Run**
- `cargo nextest run -p macros` — expect PASS.
- `cargo xtask check` — clean (coverage covers the new branch; all five existing
  adopters' tokens unchanged).

**Commit:** `feat(macros): default StrEnum tokens to snake_case, not concatenated lowercase (#576)`

---

## Task 1 — `common::registration::RegistrationPolicy`

**Files**
- `common/src/lib.rs`: add `pub mod registration;` (after `post_title`, before
  `render`).
- `common/src/registration.rs` (new): the enum with
  `#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, StrEnum)] #[str_enum(serde)]`
  and **no** per-variant rename (the Task-0 default yields `invite_only`). In-file
  tests: `tokens_parse`, `unknown_token_is_error` (incl. explicit `"inviteonly"`
  reject), `display_round_trips`, `invite_only_wire_token_is_snake_case` (asserts
  `as_str`/serde == `"invite_only"`).

**Run**
- `cargo nextest run -p common registration::` — expect PASS.
- `cargo xtask check` — clean.

**Commit:** `feat(common): RegistrationPolicy StrEnum in common::registration (#576)`

---

## Task 2 — `storage` re-uses the `common` type

**Files** — `storage/src/auth.rs`: delete the hand-rolled enum + `Display` +
`FromStr` + `InvalidRegistrationPolicy` + the now-unused `std::{fmt, str::FromStr}` /
`thiserror::Error` imports; `pub use common::registration::RegistrationPolicy;`; keep
`load_registration_policy` and its four `#[apply(backends)]` tests; drop the five
type-behavior tests (now in `common`).

**Run**
- `cargo nextest run -p storage auth::` — expect PASS.
- `cargo xtask check` — clean.

**Commit:** `refactor(storage): re-use common::RegistrationPolicy, drop the duplicate (#576)`

---

## Task 3 — `web` typed return + delete the literals

**Files**
- `web/src/registration/api.rs`: drop `RegistrationPolicy` from the server-gated
  `use storage::{…}`; add ungated `use common::registration::RegistrationPolicy;`;
  return `WebResult<RegistrationPolicy>`, body `Ok(policy)`; doc lists the variants.
- `web/src/registration/component.rs`: ungated import;
  `matches!(p, Ok(RegistrationPolicy::InviteOnly))`.
- `web/src/pages/invites.rs`: ungated import;
  `if policy.await != Ok(RegistrationPolicy::InviteOnly)` (drops `unwrap_or_default`).

**Run / final gate**
- `cargo check -p web` (client + `--features server`) compiles.
- `cargo xtask validate --no-e2e` — green. Confirm no `"open"`/`"invite_only"`/
  `"closed"` comparison literals in `web/src`.

**Commit:** `refactor(web): return typed RegistrationPolicy, delete string compares (#576)`

## Self-review

- Each task compiles/tests independently: Task 0 self-contained in `macros`; Task 1
  relies on Task 0's default; Task 2 leaves `web` compiling (still `String`-free via
  the re-export); Task 3 flips the wire type.
- Every spec acceptance criterion maps to a task (snake_case default + coverage +
  ADR → Task 0; enum-once + no-rename → Task 1; no-dup + re-export → Task 2; typed
  return + no comparison literals + gate → Task 3).
