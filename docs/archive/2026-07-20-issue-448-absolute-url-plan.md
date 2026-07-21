# `AbsoluteUrl` newtype (#448) Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with
> **jaunder-iterate** (delegating an individual task to a subagent via
> **jaunder-dispatch** when useful — Task 5 is the prime candidate). Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** Introduce a `url`-crate-backed `AbsoluteUrl` newtype, type the
always-absolute feed/site values (`base_url`, hub URLs) with it, and route all
base-URL composition through it — retiring the ad-hoc `format!` +
`percent_encode_path` composition.

**Architecture:** `AbsoluteUrl(String)` via `#[derive(StrNewtype)]` (ADR-0063)
with a hand-written `FromStr` that parses/normalizes through `url::Url` (mirrors
`FeedPath`). Composition moves to `AbsoluteUrl::join` behind a
`compose(base, path)` helper that preserves today's relative fallback when
`base_url` is unset. The composed _absolute-or-relative_ URL fields stay
`String` and are deferred to a follow-up (they are sum types, not
`AbsoluteUrl`).

**Tech Stack:** Rust, `url` crate (new `common` dep), `macros::StrNewtype`,
sqlx, Leptos (`ValidatedInput`), Playwright e2e.

**Spec:**
[`docs/superpowers/specs/2026-07-20-issue-448-absolute-url.md`](../specs/2026-07-20-issue-448-absolute-url.md)
— "how" is here; "what/why" is the spec, referenced by decision id (D1–D11) and
acceptance criterion (AC#n).

## Global Constraints

- **ADR-0063 trailer** — string newtypes use `#[derive(StrNewtype)]`; only
  `FromStr` + std derives are hand-written. Backing type must be
  `struct X(String)`.
- **ADR-0063 §5 pervasiveness** — a typed value is the newtype on _every_
  surface; the composed-URL fields are `String` here only because they are
  genuinely absolute-or-relative (D8), not for convenience.
- **Backend parity** (`CONTRIBUTING.md`) — storage tests are dual-backend
  (`#[apply]`), never a bare `#[tokio::test]`; never edit ADR-0019 per-backend
  dialect files.
- **Coverage policy** — new host-reachable code is covered; `#[server]` bodies /
  wasm-only helpers per the repo's existing patterns.
- **Gate** — the pre-commit hook runs `cargo xtask check`; run it clean before
  each commit (**jaunder-commit**). **No `Co-Authored-By` trailer.**
- **Server crate** — package name is `jaunder` (`-p jaunder`), not `-p server`.
- **New dep cost** — adding `url` triggers a shared-vendor cold rebuild (~5–8
  min); version-match the existing vendor where possible.

---

## Review header

**Scope in:** the `AbsoluteUrl` type + `join`/`compose`; typing
`SiteIdentity.base_url`, `FeedMetadata.hub_url`, `FeedsConfig.websub_hub_url`;
migrating every base-URL composition site off `format!`; the `base_url` settings
input (ADR-0065 typed arg + client validation); a `url`-dependency ADR draft; a
spun-out follow-up issue.

**Scope out:** typing the composed absolute-or-relative URL fields (feed
`self_url`/`canonical_url`, atompub `FeedMeta.*`/`id`/`edit_uri`/`content_src`,
post `permalink`/`preview_url`) — deferred to the Task 1 follow-up; making
`base_url` mandatory; any `FeedPath` change.

**Tasks:**

1. File the follow-up issue for the absolute-or-relative composed-URL fields
   (separable concern).
2. Write the `url`-in-`common`/wasm dependency ADR draft (D10).
3. `AbsoluteUrl` type + `join` + `compose` helper, with unit tests (D1–D4).
4. Type the hub URLs (`FeedsConfig.websub_hub_url`, `FeedMetadata.hub_url`) +
   storage getter/setter + renderers (D11).
5. Type `base_url` + migrate all composition sites to `compose`, delete
   `percent_encode_path`, drop the manual slash-strip (D3, D5, D6).
6. `base_url` settings input: ADR-0065 typed wire arg + client validation +
   clear-to-None e2e (D9).

**Key risks/decisions:** the unset-`base_url` **relative fallback is preserved**
(AC#5) — Task 5 is one atomic behavior-preserving change because a typed
`base_url` silently double-slashes any un-migrated `format!` site (D5). `?`/`#`
encoding semantics shift when `percent_encode_path` goes (no-op for feed paths;
correct for atompub queries — pinned by a Task 3 test).

---

## Task 1: File the follow-up issue (separable concern)

Per the spec D8 / `jaunder-plan` scope check — the composed absolute-or-relative
URL fields are not this issue; capture them up front so they can be picked up
concurrently.

- [x] **Step 1: File the issue** via **jaunder-issues** (GitHub
      `jaunder-org/jaunder`, milestone #13 "Domain-value type safety
      (newtypes)", label `type-safety`, blocked-by #448). → **Filed as
      [#560](https://github.com/jaunder-org/jaunder/issues/560)**, in project
      #1.

  Title:
  `types: relative/absolute-or-relative URL type for composed feed & post URLs`

  Body (essentials): these fields are absolute when `site.base_url` is
  configured and **root-relative** otherwise, so they are a sum type, not
  `AbsoluteUrl` (which #448 introduced). Model them as a relative-path newtype
  or an absolute-or-relative enum and type:
  `FeedMetadata.self_url`/`canonical_url` (`common/src/feed/metadata.rs`);
  `FeedMeta.self_url`/`first`/`next`/`previous`/`id`
  (`common/src/atompub/entry.rs`); the atompub entry
  `edit_uri`/`edit_media_uri`/`content_src`; post `permalink`/`preview_url`
  (`web/src/posts/*`). Note the RFC-4287 constraint that `atom:id` must be an
  absolute IRI — the type should make the unset-`base_url` case an explicit
  decision, not a silent relative fallback.

- [x] **Step 2: Record the issue number** — #560, cited above. No commit (issue
      lives in the tracker).

---

## Task 2: `url`-dependency ADR draft (D10)

**Files:**

- Create: `docs/adr/0073-url-crate-for-absolute-url-normalization.md`
  (numberless draft; `cargo xtask adr promote` numbers it at ship —
  **jaunder-adr**).

- [x] **Step 1: Write the ADR draft.** →
      `docs/adr/0073-url-crate-for-absolute-url-normalization.md`. Follow
      **jaunder-adr** (numberless draft format). Content:
  - **Context:** absolute feed/site URLs need
    scheme/host-case/percent-encoding/trailing-slash normalization (ADR-0063
    invariant axis); `url` was previously `xtask`-only.
  - **Decision:** `url` is the sanctioned URL parser/normalizer; it becomes a
    `common` dependency and is therefore compiled to wasm and **reachable** in
    the client binary (via the `SiteIdentity` settings-page deserialize).
    Rejected: hand-rolling normalization (error-prone) and reusing `urlencoding`
    (query-param encoder only, not a parser).
  - **Consequences:** client wasm bundle grows by `idna`'s tables atop the
    unicode tables `common` already ships; accepted as the cost of a correct,
    single-chokepoint normalization; one-time shared-vendor cold rebuild.
  - Reference ADR-0063 and issue #448.

- [x] **Step 2: No commit.** `docs/adr/drafts/` is gitignored (confirmed via
      `git check-ignore`) — the draft lives out of git until
      `cargo xtask adr promote` numbers it and stages it at **ship**
      (jaunder-ship). It stays in this worktree until then.

---

## Task 3: `AbsoluteUrl` type + `join` + `compose` (D1–D4)

**Files:**

- Modify: `Cargo.toml` (workspace root — add `url` to
  `[workspace.dependencies]`)
- Modify: `common/Cargo.toml` (add `url = { workspace = true }`)
- Create: `common/src/absolute_url.rs`
- Modify: `common/src/lib.rs` (add `pub mod absolute_url;`)
- Test: in-file `#[cfg(test)] mod tests` in `common/src/absolute_url.rs` (the
  `FeedPath` convention)

**Interfaces:**

- Consumes: `macros::StrNewtype`; the `url` crate.
- Produces (later tasks rely on these exact names/signatures):
  - `common::absolute_url::AbsoluteUrl` — `struct AbsoluteUrl(String)`, derives
    `Clone, Debug, PartialEq, Eq, Hash, StrNewtype`; full ADR-0063 trailer
    (incl. serde + default-on sqlx).
  - `common::absolute_url::InvalidAbsoluteUrl` —
    `#[derive(Debug, thiserror::Error)]` error,
    `impl FromStr for AbsoluteUrl { type Err = InvalidAbsoluteUrl; }`.
  - `impl AbsoluteUrl { pub fn join(&self, path: &str) -> Result<AbsoluteUrl, InvalidAbsoluteUrl>; }`
  - `common::absolute_url::compose(base: Option<&AbsoluteUrl>, path: &str) -> Result<String, InvalidAbsoluteUrl>`

- [ ] **Step 1: Add the dependency.** Root `Cargo.toml`
      `[workspace.dependencies]`: `url = "2.5"` (match the version already
      declared in `xtask/Cargo.toml` — `2.5.8` — to reuse the vendor).
      `common/Cargo.toml` `[dependencies]`: `url = { workspace = true }`.

- [ ] **Step 2: Write the failing tests** in `common/src/absolute_url.rs`:

  ```rust
  #[cfg(test)]
  mod tests {
      use super::*;
      use std::str::FromStr;

      // -- FromStr: invariant + normalization (D4, D6) --
      #[test] fn rejects_non_http_schemes() {
          for bad in ["file:///etc/passwd", "ftp://h/x", "javascript:alert(1)", "mailto:a@b.c"] {
              assert!(bad.parse::<AbsoluteUrl>().is_err(), "should reject {bad}");
          }
      }
      #[test] fn rejects_hostless_or_unparseable() {
          for bad in ["http:foo", "https://", "/feed.rss", "not a url", ""] {
              assert!(bad.parse::<AbsoluteUrl>().is_err(), "should reject {bad}");
          }
      }
      #[test] fn normalizes_host_case_and_default_port() {
          assert_eq!("https://Example.COM:443/".parse::<AbsoluteUrl>().unwrap(), *"https://example.com/");
          assert_eq!("http://H:80/".parse::<AbsoluteUrl>().unwrap(), *"http://h/");
      }
      #[test] fn adds_canonical_root_slash() {
          assert_eq!("https://example.com".parse::<AbsoluteUrl>().unwrap(), *"https://example.com/");
      }
      #[test] fn from_str_is_idempotent() {
          let once = "https://Example.com/Path".parse::<AbsoluteUrl>().unwrap();
          assert_eq!(once.as_ref().parse::<AbsoluteUrl>().unwrap(), once);
      }
      #[test] fn trailer_derefs_and_displays() {
          let u = "https://example.com/".parse::<AbsoluteUrl>().unwrap();
          let s: &str = &u;                       // Deref<str>
          assert_eq!(s, "https://example.com/");
          assert_eq!(u.to_string(), "https://example.com/");   // Display
      }

      // -- join (D3) --
      #[test] fn join_composes_without_double_slash() {
          let base = "https://example.com/".parse::<AbsoluteUrl>().unwrap();
          assert_eq!(base.join("/feed.rss").unwrap(), *"https://example.com/feed.rss");
          assert_eq!(base.join("/tags/rust/").unwrap(), *"https://example.com/tags/rust/");
      }
      #[test] fn join_preserves_query() {
          let base = "https://example.com/".parse::<AbsoluteUrl>().unwrap();
          assert_eq!(
              base.join("/atompub/alice/posts?updated_before=x&id_before=1").unwrap(),
              *"https://example.com/atompub/alice/posts?updated_before=x&id_before=1"
          );
      }
      #[test] fn join_of_canonical_feed_path_is_unchanged_path() {
          // percent_encode_path removal regression (AC#2): canonical feed paths need no escaping.
          let base = "https://example.com/".parse::<AbsoluteUrl>().unwrap();
          assert_eq!(base.join("/~alice/tags/rust/feed.atom").unwrap(),
                     *"https://example.com/~alice/tags/rust/feed.atom");
      }
      #[test] fn join_rejects_non_http_result() {
          // An absolute path with a non-http(s) scheme replaces the base entirely; the
          // re-validation through FromStr then rejects it. Pins join's Err branch.
          let base = "https://example.com/".parse::<AbsoluteUrl>().unwrap();
          assert!(base.join("mailto:foo@bar.example").is_err());
          assert!(base.join("ftp://other.example/x").is_err());
      }
      #[test] fn join_does_not_guarantee_same_origin() {
          // Documented limitation: an absolute *http* path resolves to the other host and
          // is accepted. All real call sites pass server-built "/…" literals, never user
          // input, so this cannot fire in-tree; the D8 follow-up must add an origin check
          // if it ever feeds untrusted paths to join.
          let base = "https://example.com/".parse::<AbsoluteUrl>().unwrap();
          assert_eq!(base.join("http://other.example/evil").unwrap(), *"http://other.example/evil");
      }

      // -- compose: relative fallback (D3, AC#5) --
      #[test] fn compose_uses_base_when_present() {
          let base = "https://example.com/".parse::<AbsoluteUrl>().unwrap();
          assert_eq!(compose(Some(&base), "/feed.rss").unwrap(), "https://example.com/feed.rss");
      }
      #[test] fn compose_falls_back_to_relative_when_no_base() {
          assert_eq!(compose(None, "/feed.rss").unwrap(), "/feed.rss");
          assert_eq!(compose(None, "/").unwrap(), "/");
      }
  }
  ```

  Then write the type/impl skeleton (unimplemented bodies) so the module
  compiles and the tests link-and-fail.

- [ ] **Step 3: Run the tests, verify they fail.** Run:
      `cargo nextest run -p common absolute_url` Expected: FAIL (unimplemented
      bodies).

- [ ] **Step 4: Implement against the tests.**
  - Type + error, to the Produces signatures above:

    ```rust
    #[derive(Clone, Debug, PartialEq, Eq, Hash, StrNewtype)]
    pub struct AbsoluteUrl(String);

    #[derive(Debug, thiserror::Error)]
    #[error("not a valid absolute http(s) URL")]
    pub struct InvalidAbsoluteUrl;
    ```

  - `FromStr`: `url::Url::parse(s.trim())` → `map_err(|_| InvalidAbsoluteUrl)`;
    reject unless
    `matches!(url.scheme(), "http" | "https") && url.host().is_some()`; store
    `url.to_string()`. Every branch (bad scheme, host-less, unparseable,
    normalize, idempotent) is pinned by Step 2 tests.
  - `join`: parse `&self.0` (infallible — already canonical),
    `.join(path).map_err(|_| InvalidAbsoluteUrl)`, then re-validate the result
    through `AbsoluteUrl::from_str`. Re-validation ensures the joined result is
    still a valid `http(s)` URL — it catches a `path` that introduces a non-http
    scheme (the `mailto:`/`ftp:` tests); it does **not** enforce same-origin,
    which is fine because every call site passes a server-built `/…` literal
    (documented by `join_does_not_guarantee_same_origin`). Branches pinned by
    the join tests.
  - `compose`: the two-arm match from the Interfaces block; both arms pinned by
    the compose tests.

- [ ] **Step 5: Run the tests, verify they pass.** Run:
      `cargo nextest run -p common absolute_url` → PASS. Also confirm the wasm
      target still builds: `cargo check -p web --target wasm32-unknown-unknown`
      (or the repo's wasm check) — `url` must compile for wasm (AC/D2).

- [ ] **Step 6: Commit** (run `cargo xtask check` clean first).
  ```bash
  git add Cargo.toml common/Cargo.toml common/src/absolute_url.rs common/src/lib.rs
  git commit -m "feat(#448): AbsoluteUrl newtype with url-backed normalization and join"
  ```

---

## Task 4: Type the hub URLs (D11)

`FeedsConfig.websub_hub_url` and `FeedMetadata.hub_url` are
always-absolute-or-`None` — type them independently of composition.
`websub_hub_url` is config/CLI-only (no web form), so the boundary is the
storage getter.

**Files:**

- Modify: `common/src/feed/mod.rs`
  (`FeedsConfig.websub_hub_url: Option<AbsoluteUrl>`)
- Modify: `common/src/feed/metadata.rs`
  (`FeedMetadata.hub_url: Option<AbsoluteUrl>`)
- Modify: `storage/src/site_config.rs` (`get_feeds_websub_hub_url`,
  `get_feeds_config`, `set_feeds_config`)
- Modify: `server/src/feed/regenerate.rs` (moves `feeds.websub_hub_url` into
  `FeedMetadata.hub_url` — now both `Option<AbsoluteUrl>`, no conversion)
- Modify: `common/src/feed/{json,rss}.rs` (read hub via `AsRef`/`Display` at the
  `serde_json`/`rss` boundary — §5 external-crate carve-out)
- Test: dual-backend storage round-trip in `storage/src/site_config.rs`
  `#[cfg(test)]` (follow the existing `set_feeds_config`/`get_feeds_config` test
  at ~424 and the `#[apply]` dual-backend template)

**Interfaces:**

- Consumes: `AbsoluteUrl` (Task 3).
- Produces:
  `SiteConfig::get_feeds_websub_hub_url(&self) -> Result<Option<AbsoluteUrl>, _>`;
  `FeedsConfig { websub_hub_url: Option<AbsoluteUrl>, .. }`;
  `FeedMetadata { hub_url: Option<AbsoluteUrl>, .. }`.

- [ ] **Step 1: Write the failing test** — extend the existing dual-backend
      `set_feeds_config`/`get_feeds_config` round-trip so it asserts a typed hub
      URL survives, and a bad stored value is rejected on read:

  ```rust
  // in the existing #[apply(dual_backend)] feeds-config round-trip test
  let cfg = FeedsConfig { websub_hub_url: Some("https://hub.example.com/".parse().unwrap()), ..base };
  env.site_config.set_feeds_config(&cfg).await.unwrap();
  assert_eq!(env.site_config.get_feeds_config().await.unwrap().websub_hub_url,
             Some("https://hub.example.com/".parse().unwrap()));
  // empty stored value → None
  env.base.pool()./* set FEEDS_WEBSUB_HUB_URL_KEY = "" */;
  assert_eq!(env.site_config.get_feeds_websub_hub_url().await.unwrap(), None);
  ```

  (Use `common::test_support::parse_*`-style construction where a helper exists;
  otherwise `"…".parse().unwrap()` in `cfg(test)` is fine here.)

- [ ] **Step 2: Run, verify fail.** `cargo nextest run -p storage feeds_config`
      → FAIL (type mismatch).

- [ ] **Step 3: Implement.**
  - `FeedsConfig.websub_hub_url` / `FeedMetadata.hub_url` →
    `Option<AbsoluteUrl>`.
  - `get_feeds_websub_hub_url`: parse the non-empty stored string into
    `AbsoluteUrl` (empty → `None`; invalid → surface a read error, mirroring the
    `?`-on-`FromStr` pattern). `get_feeds_config` populates from it.
    `set_feeds_config`: write `config.websub_hub_url.as_deref().unwrap_or("")`
    (Deref<str> — unchanged shape).
  - `regenerate.rs`: `hub_url: feeds.websub_hub_url` compiles unchanged (both
    `Option<AbsoluteUrl>`).
  - Feed renderers: `if let Some(hub) = &meta.hub_url { …hub.as_ref()… }` at the
    `rss`/`serde_json` boundary (`json.rs:41` → serde_json `Value`; `rss.rs:43`
    → `String` href). Their in-file test fixtures currently build hub via
    `hub.map(str::to_string)` (`json.rs:61`, `rss.rs:76`) — retype those to
    `hub.map(|s| s.parse().unwrap())`.

- [ ] **Step 4: Run, verify pass.** `cargo nextest run -p storage feeds_config`
      and `cargo nextest run -p common feed` → PASS.

- [ ] **Step 5: Commit** (`cargo xtask check` clean first).
  ```bash
  git add common/src/feed/mod.rs common/src/feed/metadata.rs common/src/feed/json.rs common/src/feed/rss.rs storage/src/site_config.rs server/src/feed/regenerate.rs
  git commit -m "feat(#448): type websub_hub_url and FeedMetadata.hub_url as AbsoluteUrl"
  ```

---

## Task 5: Type `base_url` + migrate all composition to `compose` (D3, D5, D6)

**One atomic change** (D5 forcing function): flipping `base_url` to
`AbsoluteUrl` makes every un-migrated `format!("{base}{path}")` silently
double-slash, so the type flip and every composition-site migration land in one
commit. Prime **jaunder-dispatch** candidate. Verify against the spec's
_Migrated composition sites_ list.

**Files:**

- Modify: `common/src/site.rs` (`SiteIdentity.base_url: Option<AbsoluteUrl>`)
- Modify: `storage/src/site_config.rs` (`get_identity`/`set_identity` — drop the
  manual `trim_end_matches('/')` at `:148`/`:166`; `get_identity` now parses
  into `Option<AbsoluteUrl>`)
- Modify: `server/src/feed/regenerate.rs` (self_url + per-surface canonical_url
  via `compose`; **delete `percent_encode_path`** at `:118` and its
  `..._encodes_query_marker` test at `:179`)
- Modify: `server/src/feed/worker.rs` (`ping_websub` topic URL via `compose`)
- **Modify: `server/src/atompub/mod.rs` — the shared
  `pub(crate) async fn base_url(...)` funnel (`:119-126`).** All five atompub
  composition sites route through this helper, not `identity.base_url` directly.
  Change it from `-> String` (`.unwrap_or_default()`) to
  `-> Option<AbsoluteUrl>` (returning `identity.base_url`), and retype its
  consumers' `base_url: &str` params — notably
  `post_to_entry(post: &PostRecord, base_url: Option<&AbsoluteUrl>)`
  (`mapping.rs:128`) — so each site can call `compose(base, path)`.
- Modify: `server/src/atompub/{posts,mapping,service,media,rsd}.rs` (all
  base-composition sites → `compose`, fed by the retyped helper)
- Modify: `web/src/invites/mod.rs` and `server/src/commands.rs` (the
  register-link twins → `compose`)
- Modify: existing `Eq`/round-trip tests asserting the un-slashed base
  (`storage/src/site_config.rs` ~674–721; `server/tests/web/web_site.rs` ~60/89)
  → normalized form
- Test: a new "unset base_url ⇒ relative URLs preserved" test (AC#5) and a
  "stored un-slashed base still parses" test

**Interfaces:**

- Consumes: `AbsoluteUrl`, `compose` (Task 3).
- Produces: `SiteIdentity { base_url: Option<AbsoluteUrl>, .. }`; every base-URL
  composition goes through `compose(identity.base_url.as_ref(), path)`.

- [ ] **Step 1: Write/adjust the failing tests.**
  - **Preserve-relative (new, AC#5)** — regenerate a feed metadata with
    `base_url = None` and assert the composed strings are still root-relative
    (e.g. `self_url == "/feed.rss"`, site `canonical_url == "/"`). Put it where
    `regenerate.rs`'s existing tests live.
  - **Normalized base round-trip** — a
    `SiteIdentity { base_url: Some("https://example.com".parse().unwrap()) }`
    stored and re-read equals `Some("https://example.com/".parse().unwrap())`
    (trailing slash), and a raw stored `"https://example.com"` (no slash) parses
    on read.
  - Update the existing un-slashed-base assertions to the normalized (`…/`)
    form, and **rename** the now-lying
    `identity_returns_some_base_url_when_set_with_trailing_slash_stripped`
    (`storage/src/site_config.rs:690`) to reflect that the type normalizes to
    the slashed form (the manual strip is gone).

- [ ] **Step 2: Run, verify fail.**
      `cargo nextest run -p jaunder feed::regenerate` and
      `cargo nextest run -p storage site_identity` → FAIL.

- [ ] **Step 3: Implement.**
  - `SiteIdentity.base_url` → `Option<AbsoluteUrl>`.
  - `storage/site_config.rs`: `get_identity` parses the stored base into
    `Option<AbsoluteUrl>` (empty → `None`; invalid → read error), **no** manual
    slash strip; `set_identity` writes via `Deref`/`AsRef`, no slash strip.
  - `server/src/atompub/mod.rs`: retype `base_url(...)` → `Option<AbsoluteUrl>`
    and `post_to_entry`'s `base_url` param → `Option<&AbsoluteUrl>` (see Files),
    so the atompub sites compose off the typed base.
  - Replace every base-composition `format!` (feed/worker/invites) and the
    atompub sites' calls with `compose(base, path)?` (the `path` built as today,
    retaining per-segment/query encoding). Delete `percent_encode_path` + its
    test (the canonical-path regression now lives in Task 3's
    `join_of_canonical_feed_path_is_unchanged_path`).
  - Cross-check every site in the spec's _Migrated composition sites_ list is
    converted (grep AC#4).

- [ ] **Step 4: Run, verify pass.** `cargo nextest run -p jaunder` (feed +
      atompub), `cargo nextest run -p storage`,
      `cargo nextest run -p web invites` → PASS. Then the grep check: Run:
      `rg -n '(format|println)!\("\{base' server/src web/src` → **no matches**
      (AC#4 — the pattern covers both the `format!` sites and the `println!` CLI
      invite twin at `commands.rs:307`).

- [ ] **Step 5: Commit** (`cargo xtask check` clean first — this rebuilds
      coverage; run foreground).
  ```bash
  git add common/src/site.rs storage/src/site_config.rs server/src/feed/regenerate.rs server/src/feed/worker.rs server/src/atompub/ web/src/invites/mod.rs server/src/commands.rs server/tests/web/web_site.rs
  git commit -m "feat(#448): type SiteIdentity.base_url and route composition through AbsoluteUrl::join"
  ```

---

## Task 6: `base_url` settings input — ADR-0065 typed arg + client validation (D9)

**Files:**

- Modify: `web/src/site/mod.rs` (`update_site_identity` wire arg →
  `Option<AbsoluteUrl>`; remove the manual scheme + trailing-slash check)
- Modify: `web/src/pages/site.rs` (settings form: `ValidatedInput<AbsoluteUrl>`
  / `Field<AbsoluteUrl>` for `base_url`, inline error)
- Test: server-fn test asserting a bad base_url is rejected (non-OK, per
  ADR-0065 — not a message); e2e in `end2end/` for the clear-to-None path
- Reference: `docs/adr/0065-*` and the `ValidatedInput`/`Field` gotchas (project
  memory `web_client_validation_leptos_gotchas`)

**Interfaces:**

- Consumes: `AbsoluteUrl` (Task 3), the typed `SiteIdentity.base_url` (Task 5).
- Produces:
  `update_site_identity(title: String, base_url: Option<AbsoluteUrl>) -> WebResult<()>`.

- [ ] **Step 1: Write the failing tests.**
  - Server-fn test: submitting a malformed base_url yields `Err` (assert
    `.is_err()`, not the message — ADR-0065). Submitting an empty base_url
    clears to `None` (assert `get_identity().base_url == None`), per the
    ADR-0065 omit/None clear pattern.
  - e2e (`end2end/`): open site settings, clear the base URL field, save, reload
    → the field is empty and no error; set a valid URL, save → it round-trips
    with the trailing slash. (Model on the existing site-settings e2e; preserve
    the clear path — memory `adr0065_optional_field_clearing`.)

- [ ] **Step 2: Run, verify fail.** `cargo nextest run -p web site` and the e2e
      spec → FAIL.

- [ ] **Step 3: Implement.**
  - `update_site_identity` takes `base_url: Option<AbsoluteUrl>`; the validating
    serde bridge rejects malformed input on the wire; delete the manual
    `http(s)`/slash logic (the type owns it). Keep empty→clear as omit/None.
  - Settings form: swap the raw text input for `ValidatedInput<AbsoluteUrl>`
    (bare inner for the optional field — memory
    `web_client_validation_leptos_gotchas`), surfacing an inline error before
    submit; `prop:value` reads `identity.base_url` via `Display`.

- [ ] **Step 4: Run, verify pass.** `cargo nextest run -p web site` → PASS; e2e
      green via `cargo xtask e2e <backend> <browser>` for one combo locally.

- [ ] **Step 5: Commit** (`cargo xtask check` clean first).
  ```bash
  git add web/src/site/mod.rs web/src/pages/site.rs end2end/
  git commit -m "feat(#448): type base_url settings input with ADR-0065 client validation"
  ```

---

## Self-review notes

- **Spec coverage:** AC#1 → Task 3; AC#2 → Task 3 (join tests) + Task 5
  (`percent_encode_path` deletion); AC#3 (hub) + AC#9 → Task 4; AC#3
  (base_url) + AC#4 + AC#5 + AC#6 → Task 5; AC#7 + AC#8 → Task 6; AC#10 → Task
  1; AC#11 → Task 2; AC#12 → the final gate before ship.
- **Type consistency:** `AbsoluteUrl` / `InvalidAbsoluteUrl` / `join` /
  `compose` names are fixed in Task 3's Produces block and consumed verbatim in
  Tasks 4–6.
- **Deferred-field discipline:** the composed absolute-or-relative fields are
  untouched here (they stay `String`); the Task 1 follow-up owns them — no
  partial typing that would flatten a `join` result back to `String` mid-struct.
