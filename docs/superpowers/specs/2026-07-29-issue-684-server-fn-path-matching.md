# Spec: match `#[server]` fns by vertical, drop vestigial vertical nouns (#684)

- Issue: [#684](https://github.com/jaunder-org/jaunder/issues/684)
- Date: 2026-07-29
- Base: `df6a7fab` (immediately after #511 / PR #699 merged)

## Context

The 55 `#[server]` fn idents in `web/src` carry a vertical noun the module path
already states — `audiences::create_audience`, `site::get_site_identity`,
`media::list_my_media`. Since the split into `web/src/<vertical>/` directories
those nouns are vestigial.

Dropping them is blocked by ADR-0066's registrar gate, which matches `#[server]`
fns by **leaf type name** and hard-fails on a duplicate leaf
(`xtask/src/steps/server_fn_registrar_check.rs:192-209`, tested at `:414`).
Stripping the nouns collides immediately: `Create` across
audiences/invites/posts, `Delete` across audiences/media/posts, `List` across
invites/sessions/tags, `Get` and `Update` across posts/profile, `ListMine`
across audiences/media.

This work removes that blocker, performs the rename, and — because the design
interview found the wire URLs are in the same bind — re-namespaces the
endpoints.

### What the interview established that the issue got wrong

Three premises in the issue body did not survive investigation:

1. **"#511 … derives the span name from the fn ident, so this is free."** #511
   was unmerged at the time (PR #699, merged 2026-07-29T19:45Z). Its span names
   are _literal strings_, not derived at runtime — but its gate derives the
   _expected_ value and **rewrites the literal** under `Mode::Fix`. So the span
   half is free in effort, not in diff.
2. **"`endpoint` values are wire URLs called only by the wasm client."** They
   are also named by **228** quoted literals across 15 files in
   `server/tests/**` (231 mentions including 3 in prose at
   `server/tests/web/web_auth.rs:645,694`), 15 edit points across 7 files in
   `end2end/tests/**`, and several live docs.
3. **"Renaming them is optional churn."** Renaming them to follow the idents is
   _impossible_ as-is: `server/src/lib.rs:65` mounts every server fn under one
   wildcard `"/api/{*fn_name}"`, a flat global namespace. Three verticals'
   `create` would all want `/api/create`.

### What the crate source says (verified, not assumed)

Versions are the ones this workspace resolves (`Cargo.lock`): `axum 0.8.9`,
`matchit 0.8.4`, `server_fn 0.8.12`, `server_fn_macro 0.8.10`,
`leptos_axum 0.8.9`.

- `server_fn_macro-0.8.10/src/lib.rs:483-546` — with `endpoint` set the URL is
  `prefix + mod_path + fn_path` and **carries no hash**; without it the URL is
  `prefix + "/" + mod_path + fn_name + hash`.
- `:515-521` — that hash is `xxh64(CARGO_MANIFEST_DIR + ":" + module_path!())`.
  The doc comment above it claims "a hash of the function name and location";
  the code hashes neither. It is **manifest-dir + module path**, which makes the
  default URL vary by checkout directory. That is why this repo pins `endpoint`,
  and that decision stands.
- `server_fn-0.8.12/src/lib.rs:220` — `ServerFn::PATH` is a public associated
  const, emitted on the generated struct at `server_fn_macro/src/lib.rs:670`.
  This is the supported way to name a server fn's URL from test code; it is not
  privileged to components.
- `server_fn-0.8.12/src/lib.rs:1116-1124` — dispatch is an exact-match lookup on
  `(path, method)` against `REGISTERED_SERVER_FUNCTIONS`; `register_explicit`
  (`:1060`) inserts under `(T::PATH, T::Protocol::METHOD)` (`:1072-1073`).
- `leptos_axum-0.8.9/src/lib.rs:383-387` — the handler keys off
  `req.uri().path()`, the **full** request path. Nothing reads the `{*fn_name}`
  capture.
- `matchit-0.8.4/src/lib.rs:39-48` — a catch-all `{*p}` matches
  **multi-segment** remainders; the crate's own executed doctest asserts route
  `/{*p}` against `/c/bar.css` yields `p == "c/bar.css"`. So `"/api/{*fn_name}"`
  matches `/api/posts/create`.

### Why the duplicate check cannot simply be deleted

An earlier draft of this spec deleted the duplicate-leaf hard-fail, arguing that
two same-ident `#[server]` fns in one vertical are a compile error. **That is
false**, and a cold review caught it. Verified with `rustc` (exit 0, no error):
an item defined in `api.rs` **silently shadows** a glob-imported name of the
same ident from `pub use listing::*` (`web/src/posts/api.rs:16`) — glob imports
have lower precedence than local items. So `posts/api.rs::create` and
`posts/api/listing.rs::create` coexist, `posts/mod.rs` re-exports only the
`api.rs` one, both collapse to a single `(posts, Create)` key, one registrar
entry satisfies both, and the unregistered fn **silently 404s** — precisely the
#358 hole ADR-0066 exists to close.

A second route to the same hole: every vertical's `mod.rs` re-exports an
explicit list from `api` only, so a `#[server]` fn added in
`<vertical>/server.rs` or `<vertical>/component.rs` duplicating an `api.rs`
ident never reaches a `pub use` conflict either.

The check is therefore **narrowed to per-vertical**, not deleted (D2).

## Decisions

| #   | Decision                                                                                                                                                                                                                                    | Rationale                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| --- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| D1  | The registrar gate matches on **`(vertical, leaf)`**, where vertical is the first path segment under `web/src`.                                                                                                                             | The source module path is not usable: every vertical declares `mod api;` **privately** and re-exports explicitly, so `web::posts::api::CreatePost` is not a nameable path. `(vertical, leaf)` is a directory lookup needing no name resolution, and it is already the shape of all 55 registrar entries.                                                                                                                                                                                                                                                                                                                         |
| D2  | The duplicate-leaf hard-fail is **narrowed to per-vertical**, not deleted.                                                                                                                                                                  | `(vertical, leaf)` dissolves the _cross_-vertical collisions that blocked the rename, but does **not** make the within-vertical check redundant — the compiler does not catch that case (see above). Narrowing also closes a residual hole: `list_mine` and `listMine` are distinct idents that both PascalCase to `ListMine`, so they produce _different_ endpoints and would slip past the endpoint gate; a per-vertical leaf check catches them.                                                                                                                                                                              |
| D3  | `pub use listing::*;` (`web/src/posts/api.rs:16`) needs **no resolution**.                                                                                                                                                                  | `posts/api.rs` and `posts/api/listing.rs` share the vertical `posts`. The re-export problem ADR-0066 cited as the blocker for path matching is not in the way.                                                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| D4  | Idents drop the vertical's own noun; where a bare strip **misreads**, the name is rephrased.                                                                                                                                                | `posts::list_user_posts` → `list_user` reads as _listing users_. The goal is that `<vertical>::<ident>` reads as the operation, which a backwards name fails just as `create_post` does. Two fns are affected.                                                                                                                                                                                                                                                                                                                                                                                                                   |
| D5  | Endpoints are re-namespaced to **`/api/<vertical>/<ident>`**.                                                                                                                                                                               | Restores the ident↔endpoint correspondence the rename would otherwise break, and makes the wire mirror the module tree. Verified dispatchable.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                  |
| D6  | `server/tests/**` stops hardcoding endpoint strings and names **`<web::…::Type as ServerFn>::PATH`**.                                                                                                                                       | `server`'s tests already link `web` and already name those exact types in the registrar. Kills the drift class permanently and makes D5 a leaf change for 228 Rust literals.                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| D7  | A **new sibling gate** `server-fn-endpoint` owns the endpoint rule, with the `Mode::Fix`/`Mode::Check` split.                                                                                                                               | `endpoint` becomes a derived literal in exactly the sense `name` is. Auto-fix means the 55 endpoint rewrites are generated, not typed, so they cannot be typo'd on a wire format.                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| D8  | `vertical_of()` and the attribute-literal rewrite machinery move into the shared `xtask/src/web_server_fns.rs`.                                                                                                                             | `vertical_of` is private in the tracing gate and now needed by three gates. `rewrite_name`/`LineFix`/`apply_fixes` are generic "replace `<key> = "…"` in an attribute" code that only happens to be spelled for `name`.                                                                                                                                                                                                                                                                                                                                                                                                          |
| D9  | The **12 hand-written wire DTOs** that carry their vertical's noun are renamed too.                                                                                                                                                         | They exist _because_ the fn was `create_post`; leaving `posts::create` taking `CreatePostArgs` reintroduces the redundancy one layer down. Scope is drawn at "appears in a `#[server]` fn signature" — which excludes `auth::AuthUser`, `auth::AuthRejection` (server extractors) and `tags::TagInputState` (client state). Those are a separate cleanup.                                                                                                                                                                                                                                                                        |
| D10 | **Diff discipline.** Comments and doc comments are edited **only** where the rename makes them factually incorrect (a stale ident, a stale URL, a stale claim). No opportunistic rewording, no reflowing, no "while I'm here" improvements. | The diff is already large (228 + 55 + 42 + 12 sites). Every non-essential comment edit is noise a reviewer must read and dismiss. Keeping the diff mechanical is what keeps it reviewable.                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| D11 | The **`boundary!("…")` label** is renamed along with its fn — a _third_ derived literal, alongside the span name and the endpoint.                                                                                                          | Every `#[server]` body wraps itself in `boundary!("<fn ident>", { … })`; `web/src/lib.rs:15-19` forwards the string to `error::server_boundary(server_fn: &'static str, …)` (`web/src/error.rs:115-127`), which emits it as the ADR-0011 structured-log/metric field naming the **failing server fn**. Unlike the other two it is enforced by **nothing** — not the compiler (an opaque `&'static str`), not a gate, not a test. Leaving the 42 labels stale would attribute failures to fns that no longer exist, silently. A gate for it is out of scope (#714), so the rename is hand-done and verified by an explicit sweep. |

### Rename table — fns (42 renamed, 13 unchanged)

Independently recomputed and confirmed: zero per-vertical leaf collisions, zero
endpoint collisions, zero collisions with existing public items.

| Vertical       | Rename                                                                                                                                                                                                                                                                                                                                                           | Unchanged (no vertical noun)                                                         |
| -------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------ |
| audiences      | `create_audience`→`create`, `rename_audience`→`rename`, `delete_audience`→`delete`, `list_my_audiences`→`list_mine`, `add_subscriber_to_audience`→`add_subscriber`, `remove_subscriber_from_audience`→`remove_subscriber`, `list_audience_members`→`list_members`                                                                                                | `list_my_subscribers`                                                                |
| auth           | —                                                                                                                                                                                                                                                                                                                                                                | `login`, `logout`, `session`                                                         |
| backup         | `backup_warning_visible`→`warning_visible`, `get_backup_settings`→`get_settings`, `update_backup_settings`→`update_settings`                                                                                                                                                                                                                                     | —                                                                                    |
| email          | `request_email_verification`→`request_verification`, `verify_email`→`verify`                                                                                                                                                                                                                                                                                     | —                                                                                    |
| invites        | `create_invite`→`create`, `list_invites`→`list`                                                                                                                                                                                                                                                                                                                  | —                                                                                    |
| media          | `list_my_media`→`list_mine`, `media_usage`→`usage`, `delete_media`→`delete`, `upload_media`→`upload`                                                                                                                                                                                                                                                             | —                                                                                    |
| password_reset | `request_password_reset`→`request`, `confirm_password_reset`→`confirm`                                                                                                                                                                                                                                                                                           | —                                                                                    |
| posts          | `list_user_posts`→**`list_by_user`**, `list_posts_by_tag`→`list_by_tag`, `list_user_posts_by_tag`→**`list_by_user_and_tag`**, `create_post`→`create`, `get_post`→`get`, `get_post_preview`→`get_preview`, `update_post`→`update`, `post_audience_selection`→`audience_selection`, `publish_post`→`publish`, `delete_post`→`delete`, `unpublish_post`→`unpublish` | `list_local_timeline`, `list_home_feed`, `default_audience_selection`, `list_drafts` |
| profile        | `get_profile`→`get`, `update_profile`→`update`                                                                                                                                                                                                                                                                                                                   | `get_default_post_format`, `set_default_post_format`                                 |
| registration   | `get_registration_policy`→`get_policy`                                                                                                                                                                                                                                                                                                                           | `register`                                                                           |
| sessions       | `list_sessions`→`list`, `revoke_session`→`revoke`                                                                                                                                                                                                                                                                                                                | `create_app_password`                                                                |
| site           | `get_site_identity`→`get_identity`, `update_site_identity`→`update_identity`                                                                                                                                                                                                                                                                                     | `base_url_warning_visible`                                                           |
| subscriptions  | `subscribe_to`→`subscribe`, `unsubscribe_from`→`unsubscribe`, `is_subscribed_to`→`is_subscribed`                                                                                                                                                                                                                                                                 | —                                                                                    |
| tags           | `list_tags`→`list`                                                                                                                                                                                                                                                                                                                                               | —                                                                                    |

**Bold** entries are D4 rephrases, not bare strips.

### Rename table — wire DTOs (12, per D9)

All bare strips; none collides in its target namespace.

| Old                 | New                    | Site                          |
| ------------------- | ---------------------- | ----------------------------- |
| `AudienceSummary`   | `audiences::Summary`   | `web/src/audiences/api.rs:35` |
| `InviteInfo`        | `invites::Info`        | `web/src/invites/api.rs:29`   |
| `MediaItem`         | `media::Item`          | `web/src/media/api.rs:39`     |
| `MediaUsageData`    | `media::UsageData`     | `web/src/media/api.rs:51`     |
| `DeleteMediaResult` | `media::DeleteResult`  | `web/src/media/api.rs:59`     |
| `CreatePostResult`  | `posts::CreateResult`  | `web/src/posts/api.rs:76`     |
| `UpdatePostResult`  | `posts::UpdateResult`  | `web/src/posts/api.rs:89`     |
| `PublishPostResult` | `posts::PublishResult` | `web/src/posts/api.rs:117`    |
| `CreatePostArgs`    | `posts::CreateArgs`    | `web/src/posts/api.rs:128`    |
| `UpdatePostArgs`    | `posts::UpdateArgs`    | `web/src/posts/api.rs:142`    |
| `ProfileData`       | `profile::Data`        | `web/src/profile/api.rs:32`   |
| `SessionInfo`       | `sessions::Info`       | `web/src/sessions/api.rs:23`  |

Explicitly **not** renamed (not wire types): `auth::AuthUser`,
`auth::AuthRejection`, `tags::TagInputState`.

## Acceptance criteria

### Gate: registrar

- **AC1** Matching is on `(vertical, leaf)`: a unit test with `fn create` in
  `web/src/posts/api.rs` and `fn create` in `web/src/audiences/api.rs`,
  registered as `web::posts::Create` and `web::audiences::Create`, yields
  `problems(...) == None`.
- **AC2** With the same two fns but only `web::audiences::Create` registered,
  the gate fails and the detail names `web/src/posts/api.rs` **and** the
  vertical `posts`.
- **AC3** The duplicate check is **scoped to one vertical**: two `#[server]` fns
  with the same ident in `web/src/posts/api.rs` and
  `web/src/posts/api/listing.rs` fail the gate **even when a matching registrar
  entry exists**, because leaf matching cannot tell them apart. Unit-tested with
  that exact shape.
- **AC4** Two same-ident `#[server]` fns in _different_ verticals do **not**
  trigger the duplicate failure. Unit-tested — this is the behavior change that
  unblocks the rename.
- **AC5** A fn in `web/src/posts/api/listing.rs` registered as
  `web::posts::ListHomeFeed` passes — the glob re-export is transparent to the
  gate.
- **AC6** A registrar entry not of the form `web::<vertical>::<Leaf>` (wrong
  segment count, or a first segment that is not `web`) is reported as a
  malformed entry rather than silently failing to match. Unit-tested with
  `web::posts::api::Create` and with `posts::Create`.
- **AC7** A `#[server]` fn directly under `web/src` (no vertical directory) is a
  hard error naming the file, in both the registrar and endpoint gates — a
  deliberate tightening inherited from the shared `vertical_of`. Unit-tested.

### Gate: endpoint (new)

- **AC8** `cargo xtask validate --no-e2e` reports a step named
  `server-fn-endpoint`, wired in `xtask/src/lib.rs` alongside the registrar and
  tracing gates — `Mode::Fix` under `check`, `Mode::Check` under `validate`.
- **AC9** The gate fails a fn whose `endpoint` is not
  `/<its vertical>/<its ident>`, and the message states the expected value.
  Unit-tested with `posts::create` carrying `endpoint = "/create_post"`.
- **AC10** The gate fails when two fns would produce the same endpoint, naming
  both `file:line`s. Unit-tested.
- **AC11** The gate fails a `#[server]` fn with **no** `endpoint` argument,
  stating that the URL would otherwise be a `CARGO_MANIFEST_DIR`-dependent hash.
  Unit-tested.
- **AC12** `Mode::Fix` rewrites a wrong `endpoint` literal in place, preserving
  the attribute's other arguments (`input = Json`, …). Unit-tested on the
  rewrite fn.

### Shared xtask module

- **AC13** `vertical_of` is defined exactly once, in
  `xtask/src/web_server_fns.rs`, used by the registrar, tracing, and endpoint
  gates.
- **AC14** The attribute-literal rewrite machinery (`LineFix`, `apply_fixes`,
  and a key-parameterized rewrite) is defined exactly once and used by both the
  tracing and endpoint gates. `rg 'fn rewrite_name'` returns no matches.

### The rename

- **AC15** All 42 fn renames are applied; for each old ident,
  `rg '\bfn <old_ident>\b' web/src` returns no matches.
- **AC15b** Every renamed fn's `boundary!("…")` label matches its new ident
  (D11). All 55 `boundary!` call sites are still present, and none names an old
  ident: `rg -o 'boundary!\("[a-z_]+"' web/src` cross-checked against the 42 old
  idents returns no matches.
- **AC16** The 13 unchanged idents are untouched.
- **AC17** All 12 wire DTOs are renamed per the table; for each old type name,
  `rg '\b<OldName>\b'` across `web/`, `server/`, `common/`, `client/` returns no
  matches. `AuthUser`, `AuthRejection`, and `TagInputState` are unchanged.
- **AC18** All 55 registrar entries in `server/tests/helpers/mod.rs` use the new
  leaf names, still spelled `web::<vertical>::<Leaf>`.
- **AC19** All 55 span names are `web.<vertical>.<new ident>` and
  `server-fn-tracing` passes.

### The wire

- **AC20** All 55 endpoints are `/<vertical>/<ident>`; `server-fn-endpoint`
  passes.
- **AC21** No `server/tests` call site passes a hardcoded server-fn URL; each
  names `<web::…::Type as ServerFn>::PATH`. Checked with
  `rg '"/api/' server/tests`, whose only permitted survivors are the three
  non-call-site mentions: `web_auth.rs:725`'s assert _message_ (prose naming a
  route, not a URL passed to a helper — converting it would mean restructuring
  the assertion), and `router.rs:99,110`, where the multi-segment wildcard test
  must name a literal path to be testing anything.

  _An earlier draft stated this as `rg '"/api/[a-z_]+"' … returns
  nothing`. That check was vacuous: the character class excludes `/`, so it cannot match a new-scheme literal like `"/api/posts/create"`
  — it would have passed even with hardcoded URLs throughout. The looser pattern
  plus a named exemption list is what actually verifies the claim.\_

- **AC22** All 15 `end2end/tests/**` edit points are updated across 7 files: 11
  lines carrying an `/api/…` literal — `media.spec.ts` (2), `feeds.spec.ts` (1),
  `backup.spec.ts` (2), `posts.ts` (1 + doc comment), `audiences.spec.ts` (2),
  `helpers.ts` (route glob + doc comment) — plus 4
  `failServerFn(page, "<endpoint>")` call sites in `audiences.spec.ts` (3) and
  `authed-flash.spec.ts` (1).
- **AC23** A router integration test asserts that a `/api/<vertical>/<op>`
  request reaches its server fn — the regression guard for the multi-segment
  wildcard assumption this design rests on.

### Docs

Rule: everything under `docs/archive/` and `docs/superpowers/` is a historical
record and is **not** edited. Every other doc is live and must be correct.

- **AC24** Live docs naming a changed **URL** are updated:
  `docs/observability.md:326`,
  `docs/adr/0046-test-support-seed-binary.md:10,23`, and
  `docs/adr/0016-dependency-injection-and-appstate.md:272` (which describes the
  wire as `POST /api/{fn}`).
- **AC25** Live docs naming a renamed **ident** are updated:
  `docs/adr/0011-unified-observability.md:195,269,286` and
  `docs/adr/0039-e2e-parallelism-via-per-test-identity-fixtures.md:55`. Verified
  by `rg` for each old ident across `docs/` excluding `archive/` and
  `superpowers/`, plus `CONTEXT.md` and `CONTRIBUTING.md`.
- **AC26** Per D10, no comment or doc comment is edited except where the rename
  made it factually wrong. A reviewer sampling the diff finds no
  reworded-but-equivalent prose.

### Decisions recorded

- **AC27** ADR-0066 is amended: the matching key is `(vertical, leaf)`, and the
  duplicate check is per-vertical. The stale _Consequences_ bullet is corrected
  — it currently describes leaf collision as letting a fn **slip through** (a
  pass) while the code hard-**fails**. The amendment keeps the bullet's
  still-true observation that such a pair also collides at the endpoint level,
  and does **not** claim the new design has no limitation.
- **AC28** A new ADR draft records the `/api/<vertical>/<op>` wire namespace,
  including why `endpoint` stays pinned (the `CARGO_MANIFEST_DIR` hash) and why
  the vertical noun is load-bearing in the _endpoint_ while vestigial in the
  _ident_.

### Green

- **AC29** `cargo xtask validate --no-e2e` is green.
- **AC30** The e2e matrix is green.

## Out of scope

- Changing the `/api` prefix itself, or introducing HTTP-method semantics into
  endpoint names.
- AtomPub routes (`/api/v1/*` in
  `docs/superpowers/specs/2026-06-19-content-visibility-layer-c-design.md` is
  Mastodon's API, not ours).
- `docs/observability.md:158`'s `/api/current_user` — no such server fn exists;
  stale illustrative prose predating this work.
- `auth::AuthUser`, `auth::AuthRejection`, `tags::TagInputState` — carry a
  vertical noun but are not wire types (D9). Filed as **#713**.
- **A gate for the `boundary!("…")` label.** The 42 affected labels _are_
  renamed here (they are a third derived literal — see D11), but enforcing the
  correspondence needs `syn` traversal into the fn body rather than the shared
  attribute-rewrite machinery, so it is its own change. Filed as **#714**.
- A TypeScript-side guard tying the e2e `/api/…` literals to the endpoints.
  Filed as **#712**.
- Renaming generated types beyond what the ident rename implies.

## Risks

| Risk                                                                                                                                                                                                                                                    | Mitigation                                                                                                                                                                                  |
| ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| ~~axum's `{*fn_name}` may not match multi-segment paths.~~ **CLOSED.**                                                                                                                                                                                  | `matchit-0.8.4/src/lib.rs:47-48` is an executed doctest asserting multi-segment capture. AC23 lands a router test as a standing regression guard.                                           |
| The e2e suite keeps hardcoded endpoint strings with **no gate**, while the Rust side becomes gate-enforced. A future rename silently rewrites `endpoint` under `Mode::Fix` and the only detector of the resulting TypeScript drift is a red e2e matrix. | Accepted, and named here rather than left implicit — no constant crosses the language boundary. AC22 fixes today's sites; a TS-side guard is filed as **#712**.                             |
| The `boundary!` label (D11) is renamed by hand with no gate, so a future rename can drift it again.                                                                                                                                                     | Accepted for this issue; the gate is filed as **#714**. AC15b is the sweep that catches a miss _within_ this change. Note the exposure is unchanged from today — no gate exists now either. |
| 42 ident + 12 type renames touch call sites throughout `web/src`.                                                                                                                                                                                       | Compiler-enforced; cannot half-land.                                                                                                                                                        |
| The wire format changes for any external caller of `/api/*`.                                                                                                                                                                                            | Accepted: `/api/*` is the CSR client's private surface. The public API is AtomPub, untouched.                                                                                               |
| Diff size (228 + 55 + 42 + 12 sites) risks becoming unreviewable.                                                                                                                                                                                       | D10 (comment discipline) plus a task ordering that keeps each commit mechanical and single-purpose.                                                                                         |
| `server/tests` `::PATH` conversion (D6) is scope beyond the issue's four stated items.                                                                                                                                                                  | Deliberate, and re-confirmed against the true count of 228 (an earlier estimate of ~30 was wrong).                                                                                          |

## Review note

The re-export collision check (no new ident or PascalCase type collides with an
existing public item in its vertical) was run across all 14 verticals at spec
time. It is deliberately **not** an acceptance criterion: it asserts past
activity a conformance review cannot re-run, and its falsifiable content is
already carried by AC29 — if a collision existed, the crate would not compile.

**It was also incomplete, and implementation proved it.** The check compared new
names against each vertical's `mod.rs` exports only. It did not consider names
_imported into_ the declaring file, nor traits in scope, and three collisions
surfaced during the rename that it had predicted clean:

- `posts::audience_selection` generates `AudienceSelection`, colliding with the
  imported `common::visibility::AudienceSelection` — the domain type is now
  aliased in that file.
- `posts::{get,update}` generate `Get`/`Update`, which shadow the
  `leptos::prelude` traits that 86 `.get()`/`.update()` calls in
  `web/src/posts/component.rs` resolve through. Kept out of that import list and
  spelled `super::Update` at the use sites.
- `AudienceSummary` derives `reactive_stores::Store`, generating
  `AudienceSummaryStoreFields` by concatenation — invisible to a word-boundary
  grep for the old name.

All three were resolved without renaming a generated struct. Recorded because
the lesson generalises: a rename's collision surface includes imported names,
traits in scope, and derive-generated identifiers, and a grep for the old name
catches only the last of those.
