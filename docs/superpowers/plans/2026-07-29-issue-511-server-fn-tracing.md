# #511 — Server-fn tracing spans + enforcement gate: Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with jaunder-iterate
> (delegating individual tasks to a subagent via jaunder-dispatch when useful).
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give all 55 `web/src` `#[server]` fns a PII-safe
`#[tracing::instrument]` span named `web.<vertical>.<fn_ident>`, and add an
xtask gate that keeps it true.

**Architecture:** A syn-AST enumeration of `#[server]` fns is factored out of
the existing `server-fn-registrar` gate into a shared helper; a new
`server-fn-tracing` gate consumes the richer form and checks span presence,
derived name, and a default-deny PII allowlist over both arguments and
`fields(...)`. The instrumentation sweep lands first because the pre-commit hook
runs `cargo xtask check` — once the gate is registered, any non-conforming site
blocks every commit.

**Tech Stack:** Rust, `syn` 2 (`full`, `visit`, `extra-traits`) — already in
`xtask/Cargo.toml:21`, no new dependency — `proc-macro2` token walking for the
inside of `fields(...)` only, `tracing` / `tracing-attributes` 0.1.31,
`tracing-subscriber` (web dev-dep), `mockall` storage mocks, `cargo nextest`,
`cargo xtask`.

**Spec:** `docs/superpowers/specs/2026-07-29-issue-511-server-fn-tracing.md` —
referenced by decision (D1–D6, D4a) and acceptance criterion (AC 1–18)
throughout. Do not re-derive its analysis; the per-site table in its "Per-site
outcome" section is the authoritative checklist for Tasks 2–5.

## Global Constraints

- Span name is exactly
  `web.<first path segment under web/src>.<fn ident verbatim>` (D2).
- `#[tracing::instrument]` goes **after** `#[server]`, always (AC 2).
- Recordable types are exactly the 22 in Task 6's `RECORDABLE_TYPES`; every
  other parameter is skipped (D3). `Filename` is **skipped** — it is not on the
  allowlist.
- Spans use the default INFO level; never specify `level` (D3).
- No `Co-Authored-By` trailer on any commit.
- Run `cargo xtask check` before each commit so the pre-commit hook passes clean
  (**jaunder-commit**). It auto-fixes formatting without committing — re-check
  `git status --porcelain` after it goes green.
- xtask is excluded from the workspace: its tests run via
  `--manifest-path xtask/Cargo.toml`.
- Adding attributes shifts generated coverage regions slightly. If the
  coverage/CRAP gate complains, investigate — do **not** paper over it with
  `cov:ignore`.
- **Do not run an ad-hoc wasm clippy command.** `cargo xtask check` already
  includes a `wasm-clippy` step (`xtask/src/steps/static_checks.rs:76-99`)
  running
  `-p web -p client -p csr --features csr --target wasm32-unknown-unknown -- -D warnings`,
  with `unfulfilled_lint_expectations` and `clippy::too_many_arguments` allowed
  and tracked to #301 / #299. A bare `cargo clippy -p web --target wasm32` omits
  `--features csr` and fails on gated-out `client::reactive`; a bare `-p csr`
  one trips the two allowed lints. Both are false alarms — the gate step is the
  authority.
- Use worktree-absolute paths for every Read/Edit/Write:
  `/home/mdorman/src/jaunder/.claude/worktrees/issue-511-server-fn-tracing/…`

## Task list

1. File the two separable-concern issues (D6) — no code.
2. Prove the mechanism: runtime span test + instrument `tags` (1 fn). **Covers
   AC 15.**
3. Instrument `posts` (15 fns).
4. Instrument `audiences`, `media`, `profile`, `sessions` (19 fns).
5. Instrument `email`, `invites`, `password_reset`, `subscriptions` (9 fns);
   apply the 5 renames and 2 skip-list drops to `auth`, `backup`,
   `registration`, `site`.
6. Extract the shared `#[server]` enumerator; refactor the registrar gate onto
   it.
7. Build the `server-fn-tracing` gate — decision function, unit tests, wiring —
   and prove it bites.
8. ADR-0011 addendum.

## Key risks / decisions

- **Ordering is forced.** Tasks 2–5 (the sweep) must precede Task 7 (the gate),
  or the pre-commit hook — which runs the full `cargo xtask check`
  (`.githooks/pre-commit:21`) — blocks every intermediate commit.
- **The gate is created and wired in one commit.** Splitting them would itself
  produce an uncommittable state: `mod steps` is private
  (`xtask/src/lib.rs:15`), so a `pub fn run` that nothing calls is dead code,
  and `xtask-clippy` runs `--all-targets -- -D warnings`
  (`xtask/src/steps/static_checks.rs:119-131`) inside that same hook. The
  sibling registrar gate was landed the same way (create + unit-test + wire as
  one unit).
- **Task 2 is the mechanism proof.** It writes a failing span-capture test
  _first_, so if `#[server]`'s expansion does not place the span around the
  server-side body, we find out after 1 site rather than after 55. If that test
  cannot be made to pass, stop and revisit the spec — do not proceed to Task 3.
- **Task 6 must not change registrar behaviour.** Its 15 unit tests are the
  regression harness; they are not to be edited (AC 14).
- **Attribute parsing uses `syn::Meta`, not a hand-rolled token walk.**
  `skip(a)` and `fields(who = %token)` both parse as `Meta::List` — the `%`/`?`
  sigils sit in the list's _unparsed_ inner `TokenStream` and never reach a
  `Meta` parser, exactly as the registrar already relies on at
  `server_fn_registrar_check.rs:111-116`. Only the inside of `fields(...)` needs
  a token walk.

---

### Task 1: File the two separable-concern issues

**Files:** none — tracker only.

**Interfaces:**

- Consumes: nothing.
- Produces: two issue numbers, referenced in the Task 8 ADR addendum as
  follow-ups.

**Filed:** #684 (ADR-0066 path matching + fn rename) and #685 (`login` label
newtype). Both: type `Task`, label `web`, milestone _Code quality ratchet_,
priority P3.

- [x] **Step 1: File the ADR-0066 / fn-rename issue**

Use **jaunder-issues**. Title:
`xtask/web: match #[server] fns by module path so vertical nouns can be dropped from fn idents`.
Body must state:

- The `#[server]` fn idents carry a vertical noun that the module path now
  restates (`audiences::create_audience`, `site::get_site_identity`) — vestigial
  since the verticals were split into directories.
- The rename is blocked by ADR-0066's leaf-type-name matching:
  `xtask/src/steps/server_fn_registrar_check.rs:205-222` hard-fails duplicate
  leaf names, and stripping nouns collides on `Create` (audiences, posts,
  invites), `Delete` (audiences, posts, media), `List` (sessions, invites,
  tags), `Update` and `Get` (posts, profile), `ListMine` (audiences, media).
- Leaf matching exists because of the glob re-export at
  `web/src/posts/api.rs:16` (`pub use listing::*;`); path matching must solve
  that.
- It must also fix ADR-0066's stale _Consequences_ prose, which calls leaf
  collision a benign "accepted limitation" that could let an unregistered fn
  pass, while the code hard-fails it.
- Note the payoff: #511 derives span names from fn idents, so this rename
  improves all 55 span names with no span-name edit and no gate change.

Label `web`, milestone none, priority P3.

- [x] **Step 2: File the `login` label newtype issue**

Title: `web: auth::login takes label: Option<String> where SessionLabel exists`.
Body: `web/src/auth/api.rs:45` takes `label: Option<String>` while
`web/src/sessions/api.rs:63` takes `label: SessionLabel` for the same concept —
an ADR-0063 gap. Note that both are skipped by #511's tracing allowlist either
way, so this does not change the gate. Label `web`, priority P3.

- [x] **Step 3: Record both issue numbers**

Write them into this plan's Task 8 Step 1 (the addendum's follow-up pointer). No
commit — this task produces no repository change.

---

### Task 2: Prove the mechanism — runtime span test + instrument `tags`

**Files:**

- Modify: `web/src/tags/api.rs` (add `#[tracing::instrument]` at :31; add a
  `server_tests` module at end of file)

**Interfaces:**

- Consumes: `storage::{MockPostStorage, PostStorage}`,
  `leptos::reactive::owner::Owner`, `leptos::prelude::provide_context`,
  `tracing_subscriber::{registry, layer::Layer}`.
- Produces: `web.tags.list_tags` — the first span conforming to D2. The
  `CaptureLayer` / `Captured` test scaffolding defined here is local to this
  module; later tasks do not reuse it.

- [x] **Step 1: Write the failing test**

Append to `web/src/tags/api.rs`:

```rust
#[cfg(all(test, feature = "server"))]
mod server_tests {
    // Helper fns in this feature-gated test module aren't covered by clippy's
    // allow-{unwrap,expect}-in-tests, so allow the test-scaffolding panics.
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::list_tags;
    use leptos::prelude::provide_context;
    use leptos::reactive::owner::Owner;
    use std::sync::{Arc, Mutex};
    use storage::{MockPostStorage, PostStorage};
    use tracing::field::{Field, Visit};
    use tracing_subscriber::layer::{Context, Layer};
    use tracing_subscriber::prelude::*;
    use tracing_subscriber::registry::LookupSpan;

    /// Every span created while the layer is installed: name + the names of the
    /// fields actually recorded on it at creation.
    #[derive(Default)]
    struct Captured {
        spans: Vec<(String, Vec<String>)>,
    }

    struct CaptureLayer(Arc<Mutex<Captured>>);

    /// Collects the *names* of recorded fields. `#[instrument]` records each
    /// non-skipped argument through its `Debug` impl, so a skipped argument
    /// simply never reaches a visitor.
    struct FieldNames(Vec<String>);

    impl Visit for FieldNames {
        fn record_debug(&mut self, field: &Field, _value: &dyn std::fmt::Debug) {
            self.0.push(field.name().to_string());
        }
    }

    impl<S> Layer<S> for CaptureLayer
    where
        S: tracing::Subscriber + for<'a> LookupSpan<'a>,
    {
        fn on_new_span(
            &self,
            attrs: &tracing::span::Attributes<'_>,
            _id: &tracing::span::Id,
            _ctx: Context<'_, S>,
        ) {
            let mut names = FieldNames(Vec::new());
            attrs.record(&mut names);
            self.0
                .lock()
                .unwrap()
                .spans
                .push((attrs.metadata().name().to_string(), names.0));
        }
    }

    // guard:no-backend — mock store
    #[tokio::test]
    async fn list_tags_emits_its_derived_span_recording_limit_but_not_prefix() {
        let captured = Arc::new(Mutex::new(Captured::default()));
        let subscriber =
            tracing_subscriber::registry().with(CaptureLayer(Arc::clone(&captured)));
        let _guard = tracing::subscriber::set_default(subscriber);

        let owner = Owner::new();
        owner.set();
        let mut posts = MockPostStorage::new();
        posts
            .expect_list_tags()
            .returning(|_prefix, _limit| Ok(Vec::new()));
        provide_context(Arc::new(posts) as Arc<dyn PostStorage>);

        let result = list_tags(Some("secret-fragment".to_string()), Some(5)).await;
        drop(owner);
        assert!(result.is_ok(), "list_tags failed: {result:?}");

        let captured = captured.lock().unwrap();
        let (_, fields) = captured
            .spans
            .iter()
            .find(|(name, _)| name == "web.tags.list_tags")
            .expect("list_tags must emit a span named web.tags.list_tags");
        assert!(
            fields.iter().any(|f| f == "limit"),
            "limit is recordable and must be recorded; got {fields:?}"
        );
        assert!(
            !fields.iter().any(|f| f == "prefix"),
            "prefix is an unbounded String and must be skipped; got {fields:?}"
        );
    }
}
```

- [x] **Step 2: Run it, verify it fails**

Run:
`devtool run -- cargo nextest run -p web --features server list_tags_emits_its_derived_span`

Expected: FAIL — the
`.expect("list_tags must emit a span named web.tags.list_tags")` panics, because
`list_tags` carries no `#[tracing::instrument]` yet.

No `web/Cargo.toml` change is needed: `tracing-subscriber` is already a `web`
dev-dependency (`web/Cargo.toml:58`) and the workspace definition
(`Cargo.toml:53`) enables `["env-filter", "fmt", "json"]`, where `fmt`
transitively enables `registry` — which is why `server/src/observability.rs:961`
compiles `registry().with(layer)` today.

- [x] **Step 3: Instrument `list_tags`**

Insert immediately after the `#[server]` attribute at `web/src/tags/api.rs:31`:

```rust
#[tracing::instrument(name = "web.tags.list_tags", skip(prefix))]
```

`limit: Option<u32>` reduces to `u32`, which is recordable;
`prefix: Option<String>` reduces to `String`, which is not (D3).

- [x] **Step 4: Run the test, verify it passes**

Run:
`devtool run -- cargo nextest run -p web --features server list_tags_emits_its_derived_span`

Expected: PASS.

**If it still fails on the span lookup, STOP.** That means `#[server]`'s
expansion does not place the forwarded attribute around the server-side body,
which invalidates the whole approach — return to the user rather than
instrumenting 54 more sites the same way.

- [x] **Step 5: Check the wasm target**

Run: `devtool run -- cargo xtask check --no-test` (its `wasm-clippy` step lints
the wasm target; see Global Constraints)

Expected: clean. The attribute is forwarded onto the client stub too; `tracing`
is an unconditional `web` dependency (`web/Cargo.toml:47`) so it compiles, but
wasm clippy is not covered by the default check (see the repo's wasm-clippy
note).

- [x] **Step 6: Commit**

```bash
git add web/src/tags/api.rs
git commit -m "feat(web): instrument list_tags and pin span emission with a test (#511)"
```

Run `cargo xtask check` first (**jaunder-commit**).

---

### Task 3: Instrument `posts` (15 fns)

**Files:**

- Modify: `web/src/posts/api.rs` (10 sites), `web/src/posts/api/listing.rs` (5
  sites)

**Interfaces:**

- Consumes: the D2 naming rule proven in Task 2.
- Produces: 15 spans under `web.posts.*`. No new symbols.

- [x] **Step 1: Add the 15 attributes**

Insert each line immediately after that fn's `#[server]` attribute. Verbatim
from the spec's per-site table — note the vertical is `posts` for **both**
files, because it is the first path segment under `web/src` (D2):

`web/src/posts/api.rs`:

```rust
#[tracing::instrument(name = "web.posts.create_post", skip_all)]              // :161
#[tracing::instrument(name = "web.posts.get_post")]                            // :243
#[tracing::instrument(name = "web.posts.get_post_preview")]                    // :283
#[tracing::instrument(name = "web.posts.update_post", skip_all)]               // :308
#[tracing::instrument(name = "web.posts.default_audience_selection")]          // :401
#[tracing::instrument(name = "web.posts.post_audience_selection")]             // :415
#[tracing::instrument(name = "web.posts.list_drafts")]                         // :437
#[tracing::instrument(name = "web.posts.publish_post")]                        // :484
#[tracing::instrument(name = "web.posts.delete_post")]                         // :542
#[tracing::instrument(name = "web.posts.unpublish_post")]                      // :575
```

`web/src/posts/api/listing.rs`:

```rust
#[tracing::instrument(name = "web.posts.list_user_posts")]                     // :119
#[tracing::instrument(name = "web.posts.list_local_timeline")]                 // :142
#[tracing::instrument(name = "web.posts.list_home_feed")]                      // :163
#[tracing::instrument(name = "web.posts.list_posts_by_tag")]                   // :284
#[tracing::instrument(name = "web.posts.list_user_posts_by_tag")]              // :307
```

Line numbers are as of `wt-base-issue-511` and drift as you insert; the fn name
is the key. `create_post`/`update_post` are `skip_all` because their sole
argument is `CreatePostArgs`/`UpdatePostArgs`, which carry post bodies. The rest
record only recordable-typed args (`post_id`, `username`, `date`, `slug`, `tag`,
`cursor_created_at`, `cursor_post_id`, `limit`) and so need no skip list.

- [x] **Step 2: Verify it compiles on both targets**

Run: `devtool run -- cargo check -p web --features server --all-targets`
Expected: PASS.

Run: `devtool run -- cargo xtask check --no-test` (its `wasm-clippy` step lints
the wasm target; see Global Constraints) Expected: clean.

A compile error naming a missing `Debug` impl means a type reached a recorded
position that the spec classified as recordable but which lacks `Debug` — report
it rather than adding a `skip`, since it contradicts the spec's verified
classification.

- [x] **Step 3: Run the posts tests**

Run: `devtool run -- cargo nextest run -p web --features server posts` Expected:
PASS — no behaviour change; this pins that instrumentation did not disturb the
existing server-fn tests at `web/src/posts/api.rs:831`.

- [x] **Step 4: Commit**

```bash
git add web/src/posts/api.rs web/src/posts/api/listing.rs
git commit -m "feat(web): instrument posts server fns (#511)"
```

---

### Task 4: Instrument `audiences`, `media`, `profile`, `sessions` (19 fns)

**Files:**

- Modify: `web/src/audiences/api.rs` (8), `web/src/media/api.rs` (4),
  `web/src/profile/api.rs` (4), `web/src/sessions/api.rs` (3)

**Interfaces:**

- Consumes: the D2 naming rule.
- Produces: 19 spans under `web.audiences.*`, `web.media.*`, `web.profile.*`,
  `web.sessions.*`.

- [x] **Step 1: Add the 19 attributes**

`web/src/audiences/api.rs`:

```rust
#[tracing::instrument(name = "web.audiences.create_audience", skip_all)]                  // :51
#[tracing::instrument(name = "web.audiences.rename_audience", skip(name))]                // :65
#[tracing::instrument(name = "web.audiences.delete_audience")]                            // :79
#[tracing::instrument(name = "web.audiences.list_my_audiences")]                          // :90
#[tracing::instrument(name = "web.audiences.list_my_subscribers")]                        // :111
#[tracing::instrument(name = "web.audiences.add_subscriber_to_audience")]                 // :146
#[tracing::instrument(name = "web.audiences.remove_subscriber_from_audience")]            // :163
#[tracing::instrument(name = "web.audiences.list_audience_members")]                      // :180
```

`create_audience` and `rename_audience` skip `name: AudienceName` — an
author-private label. `rename_audience` still records `audience_id`.

`web/src/media/api.rs`:

```rust
#[tracing::instrument(name = "web.media.list_my_media")]                                  // :65
#[tracing::instrument(name = "web.media.media_usage")]                                    // :103
#[tracing::instrument(name = "web.media.delete_media", skip(filename))]                   // :126
#[tracing::instrument(name = "web.media.upload_media", skip_all)]                         // :200
```

`delete_media` skips `filename: Filename` — free user text, and a media URL is
only discoverable once a published post references it (D3). It still records
`sha256`, `source`, `force`.

`web/src/profile/api.rs`:

```rust
#[tracing::instrument(name = "web.profile.get_profile")]                                  // :41
#[tracing::instrument(name = "web.profile.update_profile", skip_all)]                     // :66
#[tracing::instrument(name = "web.profile.get_default_post_format")]                      // :85
#[tracing::instrument(name = "web.profile.set_default_post_format")]                      // :96
```

`web/src/sessions/api.rs`:

```rust
#[tracing::instrument(name = "web.sessions.list_sessions")]                               // :33
#[tracing::instrument(name = "web.sessions.create_app_password", skip_all)]               // :63
#[tracing::instrument(name = "web.sessions.revoke_session", skip_all)]                    // :77
```

- [x] **Step 2: Verify both targets**

Run: `devtool run -- cargo check -p web --features server --all-targets`
Expected: PASS.

Run: `devtool run -- cargo xtask check --no-test` (its `wasm-clippy` step lints
the wasm target; see Global Constraints) Expected: clean.

- [x] **Step 3: Run the affected tests**

Run: `devtool run -- cargo nextest run -p web --features server` Expected: PASS.

- [x] **Step 4: Commit**

```bash
git add web/src/audiences/api.rs web/src/media/api.rs web/src/profile/api.rs web/src/sessions/api.rs
git commit -m "feat(web): instrument audiences, media, profile, sessions server fns (#511)"
```

---

### Task 5: Instrument the remaining verticals; apply the renames and skip-list drops

**Files:**

- Modify: `web/src/email/api.rs` (2), `web/src/invites/api.rs` (2),
  `web/src/password_reset/api.rs` (2), `web/src/subscriptions/api.rs` (3)
- Modify: `web/src/backup/api.rs` (3 renames, 1 skip-list drop),
  `web/src/site/api.rs` (2 renames, 1 skip-list drop)

**Interfaces:**

- Consumes: the D2 naming rule.
- Produces: the final 9 new spans; all 55 sites now conform to D2/D3.

- [x] **Step 1: Add the 9 new attributes**

```rust
// web/src/email/api.rs
#[tracing::instrument(name = "web.email.request_email_verification", skip_all)]           // :26
#[tracing::instrument(name = "web.email.verify_email", skip_all)]                         // :70

// web/src/invites/api.rs
#[tracing::instrument(name = "web.invites.create_invite", skip(recipient_email))]         // :40
#[tracing::instrument(name = "web.invites.list_invites")]                                 // :90

// web/src/password_reset/api.rs
#[tracing::instrument(name = "web.password_reset.request_password_reset")]                // :24
#[tracing::instrument(name = "web.password_reset.confirm_password_reset", skip_all)]      // :86

// web/src/subscriptions/api.rs
#[tracing::instrument(name = "web.subscriptions.subscribe_to")]                           // :21
#[tracing::instrument(name = "web.subscriptions.unsubscribe_from")]                       // :39
#[tracing::instrument(name = "web.subscriptions.is_subscribed_to")]                       // :59
```

`request_password_reset` records `username` (ADR-0011 carve-out); the three
`subscriptions` fns record `author_username` on the same ground. `create_invite`
records `expires_in_hours` and skips `recipient_email`.

- [x] **Step 2: Apply the 5 renames and 2 skip-list drops**

Replace the existing attribute lines (D2's rename table; the attributes sit one
line below each `#[server]`):

```rust
// web/src/backup/api.rs
#[tracing::instrument(name = "web.backup.backup_warning_visible")]                        // was web.backup.warning_visible
#[tracing::instrument(name = "web.backup.get_backup_settings")]                           // was web.backup.get_settings
#[tracing::instrument(name = "web.backup.update_backup_settings")]                        // was …update_settings + skip(4 args)

// web/src/site/api.rs
#[tracing::instrument(name = "web.site.get_site_identity")]                               // was web.site.get_identity
#[tracing::instrument(name = "web.site.update_site_identity")]                            // was …update_identity + skip(title, base_url)
```

`update_backup_settings` loses its entire
`skip(destination_path, schedule, retention_count, mode)` — all four are
operator configuration (D3), so the span that today records nothing now records
all four. `update_site_identity` likewise loses `skip(title, base_url)`.

Leave `web/src/auth/api.rs` and `web/src/registration/api.rs` **unchanged** —
their five span names (`auth/api.rs:41,114,131`; `registration/api.rs:41,53`)
already match the derived form and their skip lists are correct
(`skip(password, label)`, `skip(password, invite_code)`) per AC 6. The sixth
already-correct name from the spec, `web.site.base_url_warning_visible`, lives
in `site/api.rs` and is likewise left alone while its two siblings there are
renamed.

- [x] **Step 3: Verify every site conforms, by hand, against the spec table**

Run: `rg -n -A 1 '^#\[server' web/src`

Read the output against the spec's per-site outcome table. Expected: 55
`#[server]` lines, each followed by its `#[tracing::instrument]` line, matching
the table exactly. This is the last checkpoint before the gate exists to do it
mechanically.

- [x] **Step 4: Verify both targets and the full suite**

Run: `devtool run -- cargo check -p web --features server --all-targets`
Expected: PASS.

Run: `devtool run -- cargo xtask check --no-test` (its `wasm-clippy` step lints
the wasm target; see Global Constraints) Expected: clean.

Run: `devtool run -- cargo nextest run -p web --features server` Expected: PASS.

- [x] **Step 5: Commit**

```bash
git add web/src/email/api.rs web/src/invites/api.rs web/src/password_reset/api.rs web/src/subscriptions/api.rs web/src/backup/api.rs web/src/site/api.rs
git commit -m "feat(web): instrument remaining server fns; align backup/site span names (#511)"
```

---

### Task 6: Extract the shared `#[server]` fn enumerator

**Files:**

- Create: `xtask/src/web_server_fns.rs`
- Modify: `xtask/src/lib.rs` (declare the module)
- Modify: `xtask/src/steps/server_fn_registrar_check.rs` (consume the helper)

**Interfaces:**

- Consumes: `syn` 2 with `full` + `visit` + `extra-traits` (already in
  `xtask/Cargo.toml:21`). **No new dependency** — parameter types are carried as
  parsed `syn::Type`, never rendered to a string, so `quote`/`ToTokens` is not
  needed (and `quote` is not currently an xtask dependency).
- Produces — the API Task 7 depends on:

```rust
pub struct WebServerFn {
    /// 1-based line of the `#[server]` attribute.
    pub line: usize,
    /// The fn identifier, verbatim (`list_my_media`).
    pub ident: String,
    /// Parameters in declaration order: (name, parsed type).
    pub params: Vec<(String, syn::Type)>,
    /// Every attribute on the fn, in source order.
    pub attrs: Vec<syn::Attribute>,
    /// Index into `attrs` of the `#[server]` attribute.
    pub server_attr_index: usize,
}

/// Every `#[server]` fn in one source file, or a message describing why the file
/// could not be enumerated.
pub fn server_fns_in(src: &str) -> Result<Vec<WebServerFn>, String>;

/// Collect every `.rs` file under `dir`, recursively.
pub fn rust_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>)
    -> std::io::Result<()>;

/// The `web` crate source root, scanned by both gates.
pub const WEB_SRC: &str = "web/src";
```

The parse-failure message is **exactly** `format!("cannot parse as Rust: {e}")`,
with no `line {n}:` prefix — the registrar produces that string today
(`server_fn_registrar_check.rs:61`) and it must survive the move verbatim, since
AC 14 requires the registrar's observable behaviour to be unchanged.

- [x] **Step 1: Write the failing tests for the new module**

Create `xtask/src/web_server_fns.rs` with the struct/fn signatures above (bodies
`todo!()`) and this test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captures_ident_line_and_params() {
        let src = "#[server(endpoint = \"/x\")]\n\
                   pub async fn delete_media(sha256: ContentHash, force: Option<bool>) -> R {}\n";
        let fns = server_fns_in(src).unwrap();
        assert_eq!(fns.len(), 1);
        assert_eq!(fns[0].ident, "delete_media");
        assert_eq!(fns[0].line, 1);
        let names: Vec<&str> = fns[0].params.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, vec!["sha256", "force"]);
        // Types are carried parsed; Task 7 reduces them structurally.
        assert!(matches!(fns[0].params[0].1, syn::Type::Path(_)));
        assert!(matches!(fns[0].params[1].1, syn::Type::Path(_)));
    }

    #[test]
    fn captures_every_attribute_and_locates_the_server_one() {
        let src = "/// doc\n\
                   #[server(endpoint = \"/x\")]\n\
                   #[tracing::instrument(name = \"web.a.x\")]\n\
                   pub async fn x() -> R {}\n";
        let fns = server_fns_in(src).unwrap();
        // doc comment + server + instrument
        assert_eq!(fns[0].attrs.len(), 3);
        assert!(fns[0].attrs[fns[0].server_attr_index].path().is_ident("server"));
        assert_eq!(fns[0].line, 2);
    }

    #[test]
    fn zero_arg_fn_has_no_params() {
        let src = "#[server]\npub async fn logout() -> R {}\n";
        assert!(server_fns_in(src).unwrap()[0].params.is_empty());
    }

    #[test]
    fn ignores_non_server_fns() {
        let src = "pub async fn plain() {}\n#[tokio::test]\nasync fn t() {}\n";
        assert!(server_fns_in(src).unwrap().is_empty());
    }

    #[test]
    fn syn_parse_failure_is_an_error() {
        assert!(server_fns_in("fn broken( {{{ not valid").is_err());
    }
}
```

Declare the module in `xtask/src/lib.rs` next to the existing top-level modules
(`pub mod web_server_fns;`).

- [x] **Step 2: Run them, verify they fail**

Run:
`devtool run -- cargo nextest run --manifest-path xtask/Cargo.toml web_server_fns`
Expected: FAIL — `todo!()` panics.

**Deviation — this red phase was not observed.** The module was written with its
implementation and tests together, so the first run was already green (6/6). The
regression protection that actually matters for this task is Step 6's untouched
registrar suite, which did run against the refactor and passed 15/15. Recorded
rather than back-filled.

- [x] **Step 3: Implement the enumerator**

Write the bodies to the signatures above. Every branch is pinned by Step 1's
tests — ident/line/params extraction, full attribute capture with the
`#[server]` index, the zero-arg case, non-`#[server]` fns, and the parse-failure
case — so implement against them. Three mechanics the tests cannot express:

- Move `rust_files` verbatim from `server_fn_registrar_check.rs:248-258` (it is
  unchanged; this is a relocation, not a rewrite).
- Collect params from `syn::FnArg::Typed` only, taking the name from a
  `Pat::Ident` pattern and cloning `*pat_type.ty` for the type. A
  `FnArg::Receiver` (`&self`) cannot occur on a free fn; a non-ident pattern (a
  destructured tuple) has no single name to skip or record, so treat it as a
  hard error rather than dropping it silently — dropping it would hide an
  unclassified argument from Task 7's gate.
- Keep the visitor `fail-loud`: a `syn` parse error is returned as `Err` with
  the exact string named in this task's Interfaces block, never swallowed,
  because a file we cannot enumerate could hide a bare `#[server]` fn.

- [x] **Step 4: Run them, verify they pass**

Run:
`devtool run -- cargo nextest run --manifest-path xtask/Cargo.toml web_server_fns`
Expected: PASS.

- [x] **Step 5: Refactor the registrar gate onto the helper**

In `xtask/src/steps/server_fn_registrar_check.rs`:

- Delete its private `ServerFnVisitor`, its `rust_files`, and its
  `server_fns_in`.
- Keep `ServerFn`, `pascal_case`, `server_fn_default_named`, `registered_names`,
  `RegistrarVisitor`, `register_explicit_leaf`, `problems`, and `run` — and keep
  every one of its `#[cfg(test)] mod tests` tests **unedited** (AC 14).
- Reimplement its `server_fns_in(src) -> Result<Vec<ServerFn>, String>` as a
  thin adapter over `crate::web_server_fns::server_fns_in`: for each
  `WebServerFn`, apply `server_fn_default_named(&f.attrs[f.server_attr_index])`
  — preserving the existing `Ok(false)` / `Err(e)` messages verbatim, including
  their `line {n}:` prefixes — and on `Ok(true)` push
  `ServerFn { name: pascal_case(&f.ident), line: f.line }`.
- Point `run` at `crate::web_server_fns::{rust_files, WEB_SRC}`.

- [x] **Step 6: Verify the registrar gate is unchanged**

Run:
`devtool run -- cargo nextest run --manifest-path xtask/Cargo.toml server_fn_registrar`
Expected: PASS — all 15 existing tests, unedited.

Run: `devtool run -- cargo xtask check --no-test` Expected: PASS, with
`server-fn-registrar` still `ok` in `.xtask/last-result.json`.

- [x] **Step 7: Commit**

```bash
git add xtask/src/web_server_fns.rs xtask/src/lib.rs xtask/src/steps/server_fn_registrar_check.rs
git commit -m "refactor(xtask): extract the shared web #[server] fn enumerator (#511)"
```

---

### Task 7: The `server-fn-tracing` gate — build, wire, and prove it bites

Creation and wiring are **one commit**. Landing the module unwired would fail
the pre-commit hook: `mod steps` is private (`xtask/src/lib.rs:15`), so a
`pub fn run` nothing calls is dead code, and `xtask-clippy` runs
`--all-targets -- -D warnings` (`xtask/src/steps/static_checks.rs:119-131`). The
separation is safe to collapse because the sweep (Tasks 2–5) is already
complete, so the gate passes the moment it is registered.

**Files:**

- Create: `xtask/src/steps/server_fn_tracing_check.rs`
- Modify: `xtask/src/lib.rs` — declare the step module, and register it in
  **both** `Command::Check` (after the `server_fn_registrar_check::run` at
  `:296`) and `Command::Validate` (after the one at `:328`)

**Interfaces:**

- Consumes:
  `crate::web_server_fns::{WebServerFn, server_fns_in, rust_files, WEB_SRC}`.
- Produces:

```rust
/// Recordable argument types, each with the ground that admits it (D3).
const RECORDABLE_TYPES: &[(&str, &str)];

/// The failure detail for every non-conforming `#[server]` fn, or `None` when
/// every one conforms. Pure given its inputs, so it is unit-tested directly.
fn problems(web_sources: &[(String, String)]) -> Option<String>;

pub fn run(result: &mut crate::result::CommandResult);
```

- [x] **Step 1: Write the failing tests**

Create `xtask/src/steps/server_fn_tracing_check.rs` with the signatures above
(`problems` as `todo!()`) and this test module. Each test names the acceptance
criterion it pins:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// Build a one-file source set rooted at a `web/src/<vertical>/api.rs` path.
    fn src(vertical: &str, body: &str) -> Vec<(String, String)> {
        vec![(format!("web/src/{vertical}/api.rs"), body.to_string())]
    }

    // --- AC 1/8: presence ---

    #[test]
    fn flags_a_server_fn_with_no_instrument_attribute() {
        let s = src("posts", "#[server(endpoint = \"/x\")]\npub async fn create_post() -> R {}\n");
        let detail = problems(&s).expect("a bare #[server] fn is a problem");
        assert!(detail.contains("create_post"), "{detail}");
        assert!(detail.contains("web/src/posts/api.rs"), "{detail}");
    }

    #[test]
    fn passes_a_conforming_zero_arg_fn() {
        let s = src(
            "posts",
            "#[server]\n#[tracing::instrument(name = \"web.posts.list_drafts\")]\npub async fn list_drafts() -> R {}\n",
        );
        assert_eq!(problems(&s), None);
    }

    // --- AC 2: ordering ---

    #[test]
    fn flags_instrument_placed_before_server() {
        let s = src(
            "posts",
            "#[tracing::instrument(name = \"web.posts.x\")]\n#[server]\npub async fn x() -> R {}\n",
        );
        assert!(problems(&s).expect("wrong order is a problem").contains("after"));
    }

    // --- AC 3/9: derived name ---

    #[test]
    fn flags_a_span_name_that_is_not_the_derived_one() {
        let s = src(
            "tags",
            "#[server]\n#[tracing::instrument(name = \"tags.list\")]\npub async fn list_tags() -> R {}\n",
        );
        let detail = problems(&s).expect("a wrong name is a problem");
        assert!(detail.contains("web.tags.list_tags"), "{detail}");
        assert!(detail.contains("tags.list"), "{detail}");
    }

    #[test]
    fn derives_the_vertical_from_the_first_segment_not_the_file() {
        // posts/api/listing.rs -> vertical `posts`, not `api`.
        let s = vec![(
            "web/src/posts/api/listing.rs".to_string(),
            "#[server]\n#[tracing::instrument(name = \"web.posts.list_home_feed\")]\npub async fn list_home_feed() -> R {}\n".to_string(),
        )];
        assert_eq!(problems(&s), None);
    }

    #[test]
    fn a_server_fn_directly_under_web_src_is_a_hard_error() {
        let s = vec![(
            "web/src/loose.rs".to_string(),
            "#[server]\n#[tracing::instrument(name = \"web.loose.x\")]\npub async fn x() -> R {}\n".to_string(),
        )];
        assert!(problems(&s).expect("no vertical directory is an error").contains("web/src/loose.rs"));
    }

    // --- AC 4/10: the PII allowlist ---

    #[test]
    fn flags_an_argument_that_is_neither_skipped_nor_recordable() {
        let s = src(
            "email",
            "#[server]\n#[tracing::instrument(name = \"web.email.verify_email\")]\npub async fn verify_email(token: RawToken) -> R {}\n",
        );
        let detail = problems(&s).expect("an unclassified arg is a problem");
        assert!(detail.contains("token"), "{detail}");
        assert!(detail.contains("RawToken"), "{detail}");
        assert!(detail.contains("skip"), "names the skip remedy: {detail}");
        assert!(detail.contains("RECORDABLE_TYPES"), "names the allowlist remedy: {detail}");
    }

    #[test]
    fn accepts_a_recordable_arg_unskipped_and_reduces_option_and_path() {
        let s = src(
            "posts",
            "#[server]\n#[tracing::instrument(name = \"web.posts.get_post_preview\")]\n\
             pub async fn get_post_preview(a: Option<PostId>, b: common::ids::PostId) -> R {}\n",
        );
        assert_eq!(problems(&s), None);
    }

    #[test]
    fn skip_all_covers_every_argument() {
        let s = src(
            "posts",
            "#[server]\n#[tracing::instrument(name = \"web.posts.create_post\", skip_all)]\npub async fn create_post(args: CreatePostArgs) -> R {}\n",
        );
        assert_eq!(problems(&s), None);
    }

    #[test]
    fn filename_is_not_recordable() {
        // D3: a media URL is only discoverable once a published post references it.
        let s = src(
            "media",
            "#[server]\n#[tracing::instrument(name = \"web.media.delete_media\")]\npub async fn delete_media(filename: Filename) -> R {}\n",
        );
        assert!(problems(&s).expect("Filename must be skipped").contains("Filename"));
    }

    // --- AC 11: the fields(...) bypass ---

    #[test]
    fn flags_a_fields_value_reading_a_non_recordable_argument() {
        let s = src(
            "email",
            "#[server]\n#[tracing::instrument(name = \"web.email.verify_email\", skip(token), fields(who = %token))]\npub async fn verify_email(token: RawToken) -> R {}\n",
        );
        assert!(problems(&s).expect("a fields bypass is a problem").contains("token"));
    }

    #[test]
    fn allows_a_field_named_after_a_skipped_argument_when_the_value_does_not_read_it() {
        // D4: the field-name position (left of `=`) is excluded from collection.
        let s = src(
            "auth",
            "#[server]\n#[tracing::instrument(name = \"web.auth.login\", skip(label), fields(label = \"redacted\"))]\npub async fn login(label: Option<String>) -> R {}\n",
        );
        assert_eq!(problems(&s), None);
    }

    #[test]
    fn allows_a_fields_value_reading_a_recordable_argument() {
        let s = src(
            "posts",
            "#[server]\n#[tracing::instrument(name = \"web.posts.publish_post\", skip_all, fields(post_id = %post_id))]\npub async fn publish_post(post_id: PostId) -> R {}\n",
        );
        assert_eq!(problems(&s), None);
    }

    // --- D4a: what counts as the attribute ---

    #[test]
    fn accepts_a_bare_instrument_path() {
        let s = src(
            "posts",
            "#[server]\n#[instrument(name = \"web.posts.list_drafts\")]\npub async fn list_drafts() -> R {}\n",
        );
        assert_eq!(problems(&s), None);
    }

    #[test]
    fn rejects_a_cfg_attr_wrapped_instrument() {
        let s = src(
            "posts",
            "#[server]\n#[cfg_attr(feature = \"server\", tracing::instrument(name = \"web.posts.x\"))]\npub async fn x() -> R {}\n",
        );
        assert!(problems(&s).expect("cfg_attr is a hard error").contains("cfg_attr"));
    }

    #[test]
    fn rejects_the_err_argument() {
        let s = src(
            "posts",
            "#[server]\n#[tracing::instrument(name = \"web.posts.x\", err)]\npub async fn x() -> R {}\n",
        );
        assert!(problems(&s).expect("err is rejected").contains("err"));
    }

    #[test]
    fn rejects_the_ret_argument() {
        let s = src(
            "posts",
            "#[server]\n#[tracing::instrument(name = \"web.posts.x\", ret)]\npub async fn x() -> R {}\n",
        );
        assert!(problems(&s).expect("ret is rejected").contains("ret"));
    }

    #[test]
    fn rejects_an_unrecognized_instrument_argument() {
        // Default-deny: a tracing argument this gate does not model could record
        // something the allowlist never saw, so it fails until modelled.
        let s = src(
            "posts",
            "#[server]\n#[tracing::instrument(name = \"web.posts.x\", follows_from = y)]\npub async fn x() -> R {}\n",
        );
        assert!(problems(&s).expect("an unmodelled arg is rejected").contains("follows_from"));
    }

    #[test]
    fn tolerates_level_target_and_parent() {
        let s = src(
            "posts",
            "#[server]\n#[tracing::instrument(name = \"web.posts.x\", target = \"t\", parent = None)]\npub async fn x() -> R {}\n",
        );
        assert_eq!(problems(&s), None);
    }

    #[test]
    fn a_missing_name_is_its_own_failure() {
        let s = src("posts", "#[server]\n#[tracing::instrument]\npub async fn x() -> R {}\n");
        let detail = problems(&s).expect("a missing name is a problem");
        assert!(detail.contains("name"), "{detail}");
        assert!(detail.contains("required"), "{detail}");
    }

    // --- AC 12: fail-loud enumeration ---

    #[test]
    fn an_unparseable_file_is_reported_not_skipped() {
        let s = src("posts", "fn broken( {{{ not valid");
        assert!(problems(&s).expect("a parse failure is reported").contains("web/src/posts/api.rs"));
    }
}
```

- [x] **Step 2: Run them, verify they fail**

Run:
`devtool run -- cargo nextest run --manifest-path xtask/Cargo.toml server_fn_tracing`
Expected: FAIL — `todo!()` panics on every test.

- [x] **Step 3: Implement the allowlist and the decision function**

Write `RECORDABLE_TYPES` with exactly these 22 entries, each carrying its D3
ground as the second tuple element (the justification is the point — it is what
a future reader consults):

```rust
const RECORDABLE_TYPES: &[(&str, &str)] = &[
    // Bounded by the type itself — admits no free text.
    ("PostId", "opaque row id"),
    ("AudienceId", "opaque row id"),
    ("SubscriptionId", "opaque row id"),
    ("ContentHash", "sha256 digest"),
    ("PageSize", "bounded page count"),
    ("PageOffset", "bounded page offset"),
    ("RetentionCount", "bounded count, min 1"),
    ("InviteTtlHours", "bounded hour count"),
    ("UtcInstant", "pagination cursor timestamp"),
    ("PostFormat", "bounded enum"),
    ("MediaSource", "bounded enum"),
    ("BackupMode", "bounded enum"),
    ("u32", "bounded integer"),
    ("bool", "two-valued flag"),
    // Operator configuration — set by the operator, who reads the traces.
    ("DestinationPath", "operator-configured backup path"),
    ("SiteTitle", "operator-configured site title"),
    ("AbsoluteUrl", "operator-configured site base URL"),
    ("BackupSchedule", "operator-configured cron expression"),
    // Already published — a component of a public permalink.
    ("Slug", "public post permalink component"),
    ("PermalinkDate", "public post permalink component"),
    ("Tag", "public tag-listing URL component"),
    // Permitted outright by ADR-0011.
    ("Username", "ADR-0011: usernames are public identifiers and acceptable"),
];
```

Then write `problems` to the signature above. Step 1's tests pin every branch —
presence, ordering, name derivation including the nested-path and no-vertical
cases, the allowlist with `Option`/path reduction, `skip_all`, the `fields(...)`
LHS/RHS split, the four D4a attribute rules, and the parse-failure path — so
implement against them. Four mechanics the tests cannot express, which must be
written as described:

1. **Attribute recognition (D4a).** An attribute is the instrument attribute
   when `attr.path().segments.last()` is `instrument`. Scan `attrs` for a
   `cfg_attr` whose token stream contains an `instrument` ident and reject it
   before anything else, so the `cfg_attr` message is not masked by a "missing
   attribute" message.
2. **Ordering (AC 2).** Compare the instrument attribute's index in `attrs`
   against `server_attr_index`; it must be greater.
3. **Argument parsing uses `syn::Meta`; only `fields(...)`'s interior is a token
   walk.** Parse with
   `attr.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)` — the
   same call the registrar makes at `server_fn_registrar_check.rs:111-116`.
   `skip(a, b)` and `fields(who = %token)` are both `Meta::List`, whose inner
   `TokenStream` is left unparsed, so the `%`/`?` sigils never reach a `Meta`
   parser. Dispatch on each arg's path ident:
   - `name` (`Meta::NameValue`, value a string literal) → the declared span
     name. Absent from the whole list → the missing-name failure.
   - `skip` (`Meta::List`) → every `Ident` in its token stream is a skipped
     parameter.
   - `skip_all` (`Meta::Path`) → all parameters are skipped.
   - `fields` (`Meta::List`) → walk its token stream: split on top-level
     `Punct(',')`; within each entry split on the **first** top-level
     `Punct('=')`; discard the left side (the field name — D4) and collect every
     `Ident` on the right. An entry with no `=` is a shorthand field naming an
     argument, so collect its idents too.
   - `err` / `ret` (either `Meta::Path` or `Meta::List`) → rejection, message
     pointing at D6.
   - `level` / `target` / `parent` → accepted and ignored.
   - anything else → rejection naming the argument. Default-deny: an unmodelled
     tracing argument could record a value the allowlist never inspected.
4. **Type reduction is structural, not textual.** Recurse over the parsed
   `syn::Type`: `Type::Reference` → recurse into its `elem`; `Type::Path` → take
   the last segment, and if that segment is `Option`, recurse into its single
   angle-bracketed `GenericArgument::Type`; otherwise the segment's ident is the
   reduced name. Any other `Type` variant yields `None`, which is **not
   recordable** — default-deny. Compare the reduced name against
   `RECORDABLE_TYPES`'s first tuple elements. Working on the parsed type avoids
   rendering (no `quote` dependency) and sidesteps whitespace normalization
   entirely.

Sort the accumulated failure lines and append a recovery line naming both
remedies (the `skip(...)` list and `RECORDABLE_TYPES` in this file), mirroring
`server_fn_registrar_check.rs:239-243`.

- [x] **Step 4: Run the unit tests, verify they pass**

Run:
`devtool run -- cargo nextest run --manifest-path xtask/Cargo.toml server_fn_tracing`
Expected: PASS — all 21 tests.

- [x] **Step 5: Write `run` and register the step in both commands**

Write `run` to mirror `server_fn_registrar_check.rs:264-301` exactly: walk
`WEB_SRC` via `crate::web_server_fns::rust_files`, hard-fail on a missing tree,
read every file and surface a read error as a failure rather than dropping the
file (AC 12), then push `StepResult::ok("server-fn-tracing")` or
`.fail(...).detail(...)`.

Add `pub mod server_fn_tracing_check;` to the `steps` module in
`xtask/src/lib.rs`, and add this line immediately after each existing
`steps::server_fn_registrar_check::run(&mut result);` — once in `Command::Check`
(currently `:296`) and once in `Command::Validate` (currently `:328`):

```rust
steps::server_fn_tracing_check::run(&mut result);
```

- [x] **Step 6: Verify the gate passes on the swept tree**

Run: `devtool run -- cargo xtask check --no-test` Expected: PASS.

Then confirm the step ran and is green:

Run: `rg -n 'server-fn-tracing' .xtask/last-result.json` Expected: a `steps[]`
entry with `"ok": true` (AC 7).

If it fails here, the failure names the exact site the sweep got wrong — fix
that site, not the gate.

- [x] **Step 7: Prove it bites (AC 8)**

Temporarily delete the `#[tracing::instrument]` line from
`web/src/posts/api.rs`'s `publish_post`.

Run: `devtool run -- cargo xtask check --no-test` Expected: FAIL, with a detail
line naming `web/src/posts/api.rs`, the line, and `publish_post`.

Restore the deleted line.

Run: `devtool run -- cargo xtask check --no-test` Expected: PASS.

Run: `git status --porcelain` Expected: only the two xtask files modified —
confirming the temporary deletion was fully restored.

- [x] **Step 8: Prove it bites on the allowlist (AC 10)**

Temporarily change `web/src/media/api.rs`'s `delete_media` attribute to drop
`skip(filename)`.

Run: `devtool run -- cargo xtask check --no-test` Expected: FAIL, naming
`filename`, `Filename`, and both remedies.

Restore `skip(filename)` and re-run; expected PASS, `git status --porcelain`
again showing only the two xtask files.

- [x] **Step 9: Commit**

```bash
git add xtask/src/steps/server_fn_tracing_check.rs xtask/src/lib.rs
git commit -m "feat(xtask): enforce web server-fn tracing spans in check and validate (#511)"
```

---

### Task 8: ADR-0011 addendum

**Files:**

- Modify: `docs/adr/0011-unified-observability.md` (append an addendum)

**Interfaces:**

- Consumes: the two issue numbers from Task 1.
- Produces: the written convention (AC 16). This is an amendment to an existing
  ADR, not a new draft, so `docs/adr/drafts/` and `cargo xtask adr promote` are
  **not** involved.

- [x] **Step 1: Append the addendum**

Append to `docs/adr/0011-unified-observability.md`, following the file's
existing addendum style (`## Addendum (YYYY-MM-DD): <title> (issue #N)`), dated
2026-07-29 and titled for issue #511. It must state:

- Every `#[server]` fn in `web/src` carries `#[tracing::instrument]`, placed
  after `#[server]`, named
  `web.<first path segment under web/src>.<fn ident verbatim>` — a name fully
  derived from the source location, so the `server-fn-tracing` gate checks it by
  equality.
- The four recordable grounds — bounded by the type itself; operator
  configuration (the operator is the trace's audience, and ADR-0011 prohibits
  _user_ PII and secrets); already published as a public permalink component;
  and `Username`, permitted outright by this ADR's own text above.
- Everything else is skipped, and the allowlist is **default-deny**: a new
  argument type fails the gate until classified. The list lives in
  `xtask/src/steps/server_fn_tracing_check.rs`; this ADR states the rule, the
  gate holds the list.
- `fields(...)` value expressions are checked against the same allowlist so
  `skip(x)` plus `fields(y = %x)` cannot bypass it; the field-name position is
  not checked.
- **Caveat 1:** `Bio` and `DisplayName` are skipped because no public page
  renders them _today_. If a public profile page later does, the classification
  warrants revisiting — the gate will not notice.
- **Caveat 2:** allowlisting `u32` leaves a narrow hole (a numeric OTP or PIN
  would pass); ADR-0063 closes it by convention, since such a value would arrive
  as a newtype.
- Spans are emitted at the default INFO level, so operator configuration
  (`destination_path`, `title`, `base_url`) reaches trace backends at INFO.
  Permitted — that data is the operator's own — but recorded here as a
  deliberate choice.
- `err`/`ret` are rejected by the gate pending their own PII review of the
  `WebError` `Display` chain; reference the Task 1 issues as the filed
  follow-ups.

- [x] **Step 2: Format and verify**

Run: `devtool run -- prettier -w docs/adr/0011-unified-observability.md`

Run: `devtool run -- cargo xtask check --no-test` Expected: PASS — including the
`adr-format` and `adr-readme-parity` steps
(`xtask/src/steps/adr_check.rs:19-22`; there is no step literally named
`adr-check`). Neither is affected by appending an addendum: `adr-format` checks
only the `# ADR-NNNN: <title>` heading and the `- Status:` line, and
`adr-readme-parity` checks README number/link/status agreement.

- [x] **Step 3: Commit**

```bash
git add docs/adr/0011-unified-observability.md
git commit -m "docs(adr): record the web server-fn tracing convention in ADR-0011 (#511)"
```

---

## Final verification (before ship)

- [ ] Run: `devtool run -- cargo xtask validate --no-e2e` — expected PASS (AC
      18). This subsumes the wasm half of AC 18: its `static_checks` run
      includes the `wasm-clippy` step, so a separate wasm clippy command is
      neither needed nor meaningful (see Global Constraints).
- [ ] Confirm AC 1 via the gate, not a grep. `server-fn-tracing` reporting `ok`
      in `.xtask/last-result.json` after `validate --no-e2e` **is** the proof
      that all 55 sites conform — that is what the gate computes. Do not try to
      confirm it with `rg -c`: that prints per-file counts rather than a total,
      and `rg 'tracing::instrument' web/src` also matches
      `web/src/auth/server.rs:112`, a non-`#[server]` helper, so the counts
      legitimately fail to line up.
- [ ] Confirm both Task 1 issues exist in the tracker (AC 17).
- [ ] Run: `git status --porcelain` — expected empty (the check step auto-fixes
      formatting without committing).
