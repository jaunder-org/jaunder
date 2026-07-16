# Plan — #471: `UserId` newtype

- Spec:
  [2026-07-16-issue-471-user-id-newtype.md](../specs/2026-07-16-issue-471-user-id-newtype.md)
- Issue: [#471](https://github.com/jaunder-org/jaunder/issues/471)

## Shape & commit strategy

The change is **two commits**, because of the gated-commit + compile-ripple
constraint:

- **Commit 1 — define the type** (`common::ids::UserId` + unit test). New,
  unused-by-others → the workspace stays green. Independently verifiable.
- **Commit 2 — thread it through everything** (common/visibility, storage
  records/traits/impls/backends/helpers, server, web, all tests). Flipping a
  record field or trait signature to `UserId` forces every reader (across
  `storage`→`server`/`web`) to change **in the same commit** — an intermediate
  per-crate commit would leave the workspace non-compiling, which the
  git-enforced gate rejects. So this lands atomically. It is large (~200 sites,
  ~50 files) but **purely mechanical** (a type substitution + a boundary
  conversion at `.bind`/decode sites); the compiler enumerates every rippled
  site.

No separable-concerns issues to file — the nearby non-user-id ids are already
tracked (#475 AudienceId, #476 SubscriptionId).

The authoritative site list is the **edit-map in the Appendix below** (~200
sites, inlined here so it is durable — it is AC2's completeness surface, since
the compiler catches only the _structural_ ripple and the grep has known holes).
Each task's checkbox is verifiable as stated; Task 2 is complete when the
workspace is green **and** every Appendix item is struck.

---

## Task 1 — Define `common::ids::UserId` + unit test

- [ ] Create `common/src/ids.rs`: `use macros::IdNewtype;` (the `macros` crate —
      exactly as `username.rs:3` does `use macros::StrNewtype;`), then a
      doc-commented
      `#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, IdNewtype)] pub struct UserId(i64);`.
- [ ] Declare `pub mod ids;` in `common/src/lib.rs` (alphabetical position).
- [ ] (No test helper — `UserId::from(n)` is used directly in tests; a thin
      infallible wrapper adds nothing over the `parse_*` string helpers and,
      while unused, trips the line-coverage gate.)
- [ ] Unit test in `ids.rs` (`#[cfg(test)]`) exercising **all generated code**
      (for the coverage gate): `From<i64>` construct, `i64::from(..)`
      round-trip, `Display` (`format!("{}", UserId::from(7)) == "7"`), and serde
      **bare-integer** invariance
      (`serde_json::to_string(&UserId::from(42)) == "42"` and `from_str("42")`
      round-trips).
- **Verify:** `cargo test -p common ids` green;
  `cargo check -p common --all-features`.
- **Commit:** `refactor(common): add UserId id newtype (#471)` — no
  `Co-Authored-By`.

## Task 2 — Thread `UserId` through the workspace

Work in dependency order, keeping edits mechanical; **do not commit until the
whole workspace is green.** Sub-steps (the work order within the single commit):

- [ ] **common/visibility.rs** — `local()`/`account_viewer()` `user_id` params;
      `viewer_user_id() -> Option<UserId>`;
      `SubscriptionPolicy`/`OpenSubscriptionPolicy` `initial_status`
      `author_user_id`. Fix `subscriber_ref` sites: `user_id.to_string()` still
      works via `Display`; the re-parse becomes
      `.parse::<i64>().ok().map(UserId::from)`. **Leave as-is (NOT user ids):**
      `subscriber_ref: String`, `AudienceTarget::Named(i64)` (audience id), and
      `channel_id`/`local_channel_id: i64` (channel id, future #477) — even
      though they sit inside signatures this sub-step edits.
- [ ] **storage record/input structs** — `UserRecord`, `SessionRecord`,
      `PostRecord`, `PostRevisionRecord`, `CreatePostInput`,
      `RenderedPostContent`/`PostCreation` `user_id`;
      `RenderedPostUpdate`/`PostUpdate` `editor_user_id`; `MediaRecord.user_id`;
      `InviteRecord.used_by: Option<UserId>`.
- [ ] **storage trait defs** — object traits **and** the `Backend`-generic
      dispatch/Dialect traits: `user_id`/`author_user_id`/`editor_user_id`
      params; user-id **return types** (`create_user`,
      `create_user_with_invite`, `use_password_reset`/`confirm_password_reset`,
      `use_email_verification -> (UserId, Email)`).
- [ ] **storage impls & sqlx** — `.bind(user_id)` → `.bind(i64::from(user_id))`
      at every user-id bind site; wrap decoded user ids in the `build_*_record`
      helpers
      (`build_user_record`/`build_session_record`/`build_invite_record`/`build_post_record`/
      `media_record_from_row`) and their row-tuple types; wrap
      `RETURNING user_id` `query_scalar`/`query_as` decodes with `UserId::from`.
      **Do NOT** touch the `where i64: Encode/Type` bounds (still binding
      `i64`). **Exclude** post-id and count/`SUM` scalars.
- [ ] **storage backend dirs** — `sqlite/` and `postgres/` dialect impls
      (media/posts/mod) per the edit-map: user-id params, binds, and
      RETURNING/owner-id decode positions.
- [ ] **server** — `atompub/*` (`CreatePostInput { user_id }`, `editor_user_id`,
      test literals), `media.rs` (`ProxyParams.user_id` + the `!=` compare),
      `media_manager.rs`, `commands.rs` (`{user_id}` Display → `i64::from`).
- [ ] **web** — `AuthUser.user_id` + DTO fields; `#[server]`/helper params &
      returns (`viewer_user_id: Option<UserId>`,
      `resolve_author() -> Result<UserId, _>`); `auth.user_id.to_string()` →
      `i64::from(auth.user_id).to_string()`; page components.
- [ ] **tests** — `storage::test_support::seed_user() -> UserId` (+ `seed_posts`
      user param); replace i64-literal user-id construction with
      `UserId::from(n)` across in-file `#[cfg(test)]` mods,
      `web/src/test_support.rs`, and `server/tests/` helpers
      (`make_create_post_input`, `cookie_for`, …). `assert_eq!`/`==` need no
      edit (derived `PartialEq`).
- **Verify (all must pass):**
  1. `cargo check --all-features --all-targets` green (catches
     `#[cfg(feature=server)]` web code the default check skips — per repo gotcha
     #397).
  2. **AC2 completeness** = every Appendix edit-map item struck (the real
     surface). A grep over touched files
     (`user_id`/`author_user_id`/`editor_user_id`/`used_by`/`viewer_user_id` and
     `Result<i64`/`(i64,`/`-> i64`) is a **supplementary backstop only** — it
     has known false-negatives (differently-named params like `owner`/`uid`,
     rustfmt-wrapped multi-line signatures the line-based grep splits). Passing
     the grep does not by itself prove AC2; the struck edit-map does.
  3. **AC5** grep — `subscriber_ref` is still `String`.
  4. `cargo xtask check` green (static + clippy incl. no new
     `unwrap`/`expect`, + coverage — the new `UserId` generated code is covered
     by Task 1's unit test).
- **Commit:** `refactor: thread UserId through storage/server/web (#471)` — no
  `Co-Authored-By`.

## Task 3 — Final gate checkpoint (no commit)

_A gate re-run, not a code task — produces no artifact; folds conceptually into
Task 2's verify but is called out so the full pre-push gate is explicitly run
once the tree is green._

- [ ] `cargo xtask validate --no-e2e` clean (fmt, clippy, coverage — the
      pre-push gate).
- [ ] Confirm **wire invariance** (AC3): the Task-1 serde test proves bare-int
      encoding; spot that no `#[server]`/DTO JSON shape changed (the transparent
      bridge guarantees it). e2e is the ship-step's job (CI matrix); run it
      locally if quick.
- **Verify:** validate --no-e2e exit 0; `xtask-done: … ok=true` sentinel.

---

## Coverage of spec ACs

| AC                                        | Task                              |
| ----------------------------------------- | --------------------------------- |
| AC1 (type exists, derives, no FromStr)    | Task 1                            |
| AC2 (every user-id site is `UserId`)      | Task 2 verify (edit-map + grep)   |
| AC3 (wire byte-identical)                 | Task 1 serde test + Task 3        |
| AC4 (both backends, no migration)         | Task 2 (backend dirs) + Task 3    |
| AC5 (`subscriber_ref` stays String)       | Task 2 (visibility) + verify grep |
| AC6 (`validate --no-e2e` clean, e2e pass) | Task 3 (+ ship e2e)               |

---

## Appendix — edit-map (AC2 completeness surface, ~200 sites)

Line numbers are from the fork point (`wt-base-issue-471`); verify before
editing. `.bind(user_id)` → `.bind(i64::from(user_id))`; i64 literals in tests →
`user_id(n)` / `UserId::from(n)`; `x.user_id.to_string()` →
`i64::from(x.user_id).to_string()`.

### common

- `common/src/ids.rs` — DEFINE `UserId` (new file); `common/src/lib.rs` —
  `pub mod ids;`.
- `common/src/test_support.rs` — add `user_id(n) -> ids::UserId`.
- `visibility.rs:47` `local()` param · `:62` `account_viewer()` param · `:76`
  `viewer_user_id() -> Option<UserId>` · `:97`
  `SubscriptionPolicy::initial_status` `author_user_id` · `:107`
  `OpenSubscriptionPolicy::initial_status` `_a`. LEAVE: `:50/:78`
  `subscriber_ref` (String), `:89` `AudienceTarget::Named` (audience id), `:98`
  `channel_id` (channel id).

### storage — record/input structs (user-id field)

- `users.rs:25` UserRecord · `sessions.rs:15` SessionRecord · `posts.rs:47`
  PostRecord · `posts.rs:114` PostRevisionRecord · `posts.rs:196`
  CreatePostInput · `media.rs:66` MediaRecord · `post_service.rs:29`
  RenderedPostContent · `post_service.rs:125` RenderedPostUpdate.editor_user_id
  · `post_service.rs:260` PostUpdate.editor_user_id · `post_service.rs:415`
  PostCreation · `invites.rs` InviteRecord.used_by (`Option<UserId>`). LEAVE:
  AudienceRecord (no user field), SubscriptionRecord.subscriber_ref (String).

### storage — trait defs (object + Dialect/dispatch)

- `users.rs:144` create*user `->
  Result<UserId,*>`·`:161`get_user ·`:171`update_profile ·`:180`set_email ·`:189`
  set_password.
- `sessions.rs:66` create_session · `:82` list_sessions.
- `media.rs:123/134/147/154` MediaStorage params · `:180/185` MediaDialect
  params.
- `invites.rs:55` use_invite.
- `email.rs:42` create*email_verification · `:58` use_email_verification `->
  Result<(UserId,Email),*>`.
- `password.rs:38` create_password_reset.
- `user_config.rs:15/18/21` get/set/delete.
- `audiences.rs:78/87/94/97/105/114/121` author_user_id params (7).
- `subscriptions.rs:42/50/59/64` author_user_id params (+ check Dialect :103).
- `posts.rs:542/544/635/647/684` user_id params · `:580/781` editor_user_id ·
  `:762` Dialect.
- `atomic.rs:88-95` create*user_with_invite `->
  Result<UserId,*>`. LEAVE: `audiences.rs:121`list_members`-> Vec<i64>`
  (subscription ids).

### storage — impls & sqlx (binds/decodes/helpers)

- `helpers.rs`: `:26` UserRecordParts[0] · `:39/63` build_user_record · `:81/92`
  build_session_record · `:109/117` build_invite_record used_by · `:127`
  PostRecordParts[1] · `:175/203` build_post_record · `:228` UserRow[0] · `:243`
  SessionRow[1] · `:262` InviteRow[4] · `:271` PostRow[1] · `:292/303/308`
  MediaRow[0]/media_record_from_row · plus `#[cfg(test)]` literals/asserts
  (`:433/445/477/490/509/616/633/739/755/766/773/782/784/792/804/906/918/931`).
- `users.rs:256` create_user RETURNING scalar decode · `:360/389/415/430/443`
  binds · `:409/423/436` impl params.
- `sessions.rs:145/196` impl params · `:154/202` binds.
- `media.rs:259/286/332/345` impl params · `:269/299/313` binds.
- `posts.rs:435/466` fetch_post_record param ·
  `:885/1199/1223/1258/1277/1480/1513` binds · `:991/1176/1239/1438` editor/user
  params · `:1868/1894` bind input.user_id. LEAVE `:877/882`
  post_id_for_idempotency_key (post id scalar).
- `invites.rs:106` param · `:119` bind.
- `email.rs:91` param · `:107/118` binds · `:144` query_as `(i64,String)`
  decode.
- `password.rs:82` param · `:93` bind · `:112` query_as `(i64,)` RETURNING
  decode.
- `user_config.rs:38/54/95/112/130` params · `:99/117/132` binds.
- `audiences.rs:160/186/262/286/307` params · `:166/197/219/224/242/273/294/312`
  binds.
- `subscriptions.rs:154/179/194/213` params · `:162/169/184/205/216` binds.
- `post_service.rs:104/153/175/294/338` user/editor params · `:574/987/1089`
  test sites.

### storage — backend dirs

- `sqlite/media.rs:11/24` params · `:15/32` binds (LEAVE `:12` usage-count
  decode).
- `postgres/media.rs:11/24` params · `:12`?/`:15/32` binds (LEAVE count decode).
- `sqlite/posts.rs:28` update_post editor param · `:41` decode
  `(i64,Option<DateTime>)`[0] owner.
- `postgres/posts.rs:29` editor param · `:39` owner decode.
- `sqlite/mod.rs:234` create_user_with_invite RETURNING decode · `:247` local ·
  `:257` bind · `:293` confirm_password_reset RETURNING decode · `:304` local ·
  `:330/335` binds.
- `postgres/mod.rs:118` RETURNING decode · `:144` bind · `:167` RETURNING decode
  · `:206/211` binds. (LEAVE the non-user tuple decodes the edit-map flagged:
  sqlite `:148/212/305`, postgres `:96/179`.)

### server

- `media.rs:204` ProxyParams.user_id field · `:219` `!=` compare (flows).
- `media_manager.rs:164/207/238` params · `:214/268` construct · `:562/611` test
  locals.
- `atompub/posts.rs:381` CreatePostInput{user_id} · `:504` editor_user_id ·
  `:571/616` test literals.
- `atompub/mapping.rs:548` test literal.
- `commands.rs:225/238/261` CLI locals (`:238` `{user_id}` Display →
  `i64::from`).
- (atompub media/posts `auth_user.user_id` reads flow automatically.)

### web

- `auth/server.rs:24` AuthUser.user_id field · `:74` construct (flows).
- `posts/server.rs:9` viewer_user_id param · `:35` compare · `:130` test
  literal.
- `posts/listing.rs:66` viewer_user_id param · `:359/361` test helper.
- `posts/mod.rs:951/955` test helper · `:850/888/911` test literals
  (auth.user_id reads flow).
- `subscriptions/mod.rs:35` resolve*author param · `:36` `->
  Result<UserId,*>`·`:61/79` `.to_string()`→`i64::from`·`:124/126` test helper.
- `auth/mod.rs:97` `Result<i64,_>` annotation.
- `email/mod.rs:64` `(user_id, email)` decode from use_email_verification.
- `password_reset/mod.rs:32-35` `(user_id, email)` (flows).
- `test_support.rs:21/29` auth_parts param + construct.
- (media/sessions/backup/profile/viewer/audiences `auth.user_id` reads flow.)

### tests (i64-literal construction / helpers)

- `storage/src/test_support.rs:645` seed_user `-> UserId` · `:618` seed_posts
  user param (LEAVE `:621` `Vec<i64>` post ids).
- `server/tests/storage/mod.rs:2004/2019/2033/2532/2928/6840` helper params ·
  `:6575/6812` literals.
- `server/tests/web/web_subscriptions.rs:26`, `server/tests/web/audiences.rs:26`
  cookie_for param.
- in-file `#[cfg(test)]` literals per §server/§web above.
