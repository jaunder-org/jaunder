# Spec — issue #448: `AbsoluteUrl` newtype for composed feed/site URLs

- Issue: [#448](https://github.com/jaunder-org/jaunder/issues/448)
- Milestone: Domain-value type safety (newtypes) (#13)
- Date: 2026-07-20
- Predecessor: #399 (`FeedPath` — the site-relative identity _path_; this is the
  _absolute_-URL sibling, deliberately split out)
- Governing decision record:
  [ADR-0063](../../adr/0063-domain-value-newtype-convention.md)

## Problem

Several feed/site URLs cross the codebase as bare `String`/`Option<String>` and
are absolute URLs (scheme + host + path), composed server-side by
`format!("{base_url}{path}")` with an ad-hoc `percent_encode_path` helper. There
is no type carrying the scheme / host-case / trailing-slash / percent-encoding
normalization these values need, and the scheme check exists in exactly one
place (the web settings form) while every other entry point (CLI, config,
storage) has none. Two spellings of the same URL are freely representable, and
composition is hand-rolled string surgery.

Per ADR-0063 (invariant axis: normalization), this earns a `url`-crate-backed
newtype whose constructor parses + normalizes. This PR introduces the type,
threads it through the values that are **always absolute** (the site origin and
the hub URLs), and — critically — makes the type own URL _composition_ so the
string-concatenation sites are retired.

### The absolute-or-relative wrinkle (why the composed fields are _not_ typed here)

When `base_url` is unset (`None`), feed/atompub composition today emits
**root-relative** URLs (`self_url = "/feed.rss"`, `canonical_url = "/"`, atompub
`id`/`href` = `/atompub/…`). `SiteIdentity`'s doc comment states this explicitly
("when absent, callers emit root-relative URLs"), and clearing `base_url` is a
supported UI action (see D9). An `AbsoluteUrl` **cannot** hold a host-less
relative string (its `FromStr` rejects it). So the _composed_ feed/atompub URL
fields (`self_url`, `canonical_url`, the paging hrefs, the atompub
`id`/`edit_uri`/`content_src`, and the post `permalink`/`preview_url`) are
genuinely **absolute-or-relative** values — a sum type, not an `AbsoluteUrl`.
Typing them correctly requires the relative-path type, which is **spun out**
(D8). This PR types only the values that are _always_ absolute-or-`None`, and
defers the union-typed fields.

## Decisions (resolved in design interview)

- **D1 — String-backed newtype.** `AbsoluteUrl` is `struct AbsoluteUrl(String)`
  using `#[derive(StrNewtype)]` (the full ADR-0063 trailer: `Display`,
  `AsRef`/`Borrow`/ `Deref<str>`, `TryFrom<String>`, `From<Self> for String`,
  `PartialEq<str>`/`<&str>`, the validating serde bridge, the default-on sqlx
  TEXT bridge). `url::Url` is used **only inside the hand-written `FromStr`** as
  parser/normalizer; the stored value is `url.to_string()` (the canonical
  string). Mirrors `FeedPath`.

- **D2 — `url` enters `common` (and the wasm graph); accepted cost.** `url` is
  added to `[workspace.dependencies]` and `common`'s `[dependencies]`. Because
  `common` compiles to wasm and the client deserializes `SiteIdentity.base_url`
  (the settings page reads it), `url::Url::parse` is **reachable** in the client
  wasm binary. The `idna` bundle cost (atop the unicode tables `common` already
  ships) is accepted and recorded in an ADR (D10).

- **D3 — `AbsoluteUrl` owns composition via a fallible `join`.** Add
  `fn join(&self, path: &str) -> Result<AbsoluteUrl, InvalidAbsoluteUrl>` backed
  by `url::Url::join`. Every `format!("{base}{path}")` composition site migrates
  to compose via `join` (through the `compose` helper below).
  `percent_encode_path` in `regenerate.rs` is **deleted**. `join` is fallible;
  call sites are all `async fn`s already returning results.

- **D4 — `FromStr` requires `http`/`https`.** Any other scheme (`file:`, `ftp:`,
  `mailto:`, `javascript:`, …) is rejected at the chokepoint; `url` lowercases
  the scheme. The manual `http(s)` check in `web/src/site/mod.rs` is **removed**
  — the type is the only scheme guard, now covering CLI/config/storage entry
  too.

- **D5 — Full composition-site sweep, one PR.** Every `base_url` composition
  site migrates. Rationale (the forcing function): once
  `base_url: Option<AbsoluteUrl>`, `identity.base_url.as_deref().unwrap_or("")`
  still _compiles_ (via `Deref<str>`) but yields the url-normalized string
  **with** a trailing slash, so any un-migrated `format!("{base}{path}")`
  **silently** emits a double slash (`https://host//feed.rss`). Leaving a site
  un-migrated ships a regression; the whole sweep goes together.

- **D6 — url's canonical base spelling (trailing slash) is accepted.**
  `https://example.com` → `https://example.com/`. The settings form shows the
  slash; stored `site.base_url` gains it on next save; **old values without it
  still parse** (no data migration). The redundant trailing-slash-strip code in
  `storage/src/site_config.rs` (read _and_ write) and `web/src/site/mod.rs` is
  removed. `Eq` round-trip tests update to the normalized form.

- **D7 — Name: `AbsoluteUrl`.** Names the invariant accurately across every
  carrier, including the _external_ WebSub hub URL (`SiteUrl` would misdescribe
  that). Pairs with `FeedPath`.

- **D8 — Composed absolute-or-relative URLs are spun out.** A follow-up issue
  (the plan's first task, via `jaunder-issues`) introduces the relative-path /
  absolute-or-relative type and applies it to: the feed
  `self_url`/`canonical_url`, the atompub `FeedMeta.self_url` / paging hrefs /
  `id`, the atompub entry `edit_uri`/`edit_media_uri`/`content_src`, and the
  post `permalink`/`preview_url`. All of those stay `String` in this PR.

- **D9 — `base_url` input gets the full ADR-0065 treatment.**
  `update_site_identity`'s wire arg becomes `Option<AbsoluteUrl>` (validating
  serde bridge rejects malformed input on the wire; clearing = omit/None per the
  ADR-0065 optional-field pattern). The settings-form input gets client-side
  pre-validation via `ValidatedInput<AbsoluteUrl>` / `Field<T>` (#414/ADR-0065)
  for inline errors. An e2e preserves the clear-to-None path.

- **D10 — A short ADR** records the `url`-in-`common`/wasm dependency decision
  (why `url`, why the bundle cost is accepted, `url` as the sanctioned
  URL-normalization tool over hand-rolling / `urlencoding`). Drafted via
  `jaunder-adr`. Remaining mechanics stay in this spec (ADR-0063 application,
  like `FeedPath`).

- **D11 — `websub_hub_url` is config/CLI-only.** Set via `server/src/cli.rs`
  (`site-config set feeds.websub_hub_url`) and stored as a raw config string;
  **no web form**. So it gets the typed field + a validating storage _getter_
  (parse on read is the boundary), but **not** the D9 client-validation
  treatment.

## Design

### The type (`common/src/absolute_url.rs`, module `absolute_url`)

Module named `absolute_url` (not `url`) to avoid shadowing the `url` crate.

```rust
#[derive(Clone, Debug, PartialEq, Eq, Hash, StrNewtype)]
pub struct AbsoluteUrl(String);

#[derive(Debug, Error)]
#[error("not a valid absolute http(s) URL")]
pub struct InvalidAbsoluteUrl;

impl FromStr for AbsoluteUrl {
    type Err = InvalidAbsoluteUrl;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let url = url::Url::parse(s.trim()).map_err(|_| InvalidAbsoluteUrl)?;
        if !matches!(url.scheme(), "http" | "https") || url.host().is_none() {
            return Err(InvalidAbsoluteUrl);           // http/https + real host
        }
        Ok(Self(url.to_string()))                     // url-canonical form
    }
}

impl AbsoluteUrl {
    pub fn join(&self, path: &str) -> Result<AbsoluteUrl, InvalidAbsoluteUrl> {
        // url::Url::parse(&self.0) is infallible (self.0 is already url-canonical);
        // .join(path) resolves the (site-absolute) path and re-validates via FromStr.
        …
    }
}
```

Normalization owned by `FromStr` (all via `url`): scheme lowercased + restricted
to `http`/`https`; host lowercased; default port (`:80`/`:443`) stripped;
canonical root path (`/`) present; percent-encoding canonicalized. `Hash`
derived (potential map key); no `Ord`.

### Composition helper (preserves the relative fallback)

The composed feed/atompub URLs stay `String` (D8), but every site moves off
`format!` onto a single helper so composition correctness (slash boundary,
encoding) lives in one place:

```rust
/// Compose `base` + a site-absolute `path` into an absolute URL string when a base is
/// configured, else the relative path unchanged — exactly today's behavior, but the
/// base-set branch now goes through `AbsoluteUrl::join` (correct slash + encoding).
fn compose(base: Option<&AbsoluteUrl>, path: &str) -> Result<String, InvalidAbsoluteUrl> {
    match base {
        Some(b) => Ok(b.join(path)?.into()),
        None => Ok(path.to_owned()),
    }
}
```

- `path` is built exactly as today (e.g. `/feed.rss`, `/tags/{tag}/`,
  `/atompub/{user}/posts?updated_before={ts}&id_before={id}`), retaining the
  existing per-segment/query encoding (`urlencoding::encode`, `utf8_encode`),
  which `url::Url::join` leaves intact (it does not double-encode valid `%XX`).
- `percent_encode_path` is deleted: for the canonical `FeedPath` surfaces the
  path uses only unreserved chars + `~`, so join needs no extra escaping.
  **Behavioral note:** the old `percent_encode_path` escaped `?`/`#` (its
  `..._encodes_query_marker` test); `join` honors `?`/`#` as URL delimiters. For
  canonical feed paths (no `?`/`#`) this is a no-op; the atompub query strings
  _want_ the `?` honored. A unit test pins that composing a canonical `FeedPath`
  yields an unchanged path.
- Site-surface canonical HTML URLs keep their trailing slash (`{base}/`,
  `{base}/tags/{tag}/`, `{base}/~{user}/`) because the joined path carries it.

### Typed carrier fields (this PR — always-absolute-or-`None` only)

| File                          | Struct.field                 | Old → New                                |
| ----------------------------- | ---------------------------- | ---------------------------------------- |
| `common/src/site.rs`          | `SiteIdentity.base_url`      | `Option<String>` → `Option<AbsoluteUrl>` |
| `common/src/feed/metadata.rs` | `FeedMetadata.hub_url`       | `Option<String>` → `Option<AbsoluteUrl>` |
| `common/src/feed/mod.rs`      | `FeedsConfig.websub_hub_url` | `Option<String>` → `Option<AbsoluteUrl>` |

`FeedMetadata.hub_url` is moved in directly from `FeedsConfig.websub_hub_url` in
`regenerate.rs` — both now `Option<AbsoluteUrl>`, no conversion. Renderers read
hub via `Deref`/`AsRef`/`Display` at the `rss`/`atom_syndication`/`serde_json`
boundary (§5 carve-out).

**Explicitly NOT typed this PR (stay `String`, deferred to the D8 follow-up):**
`FeedMetadata.self_url`/`canonical_url`;
`FeedMeta.self_url`/`first`/`next`/`previous`/`id`
(`common/src/atompub/entry.rs`); the atompub entry
`edit_uri`/`edit_media_uri`/`content_src`; post `permalink`/`preview_url`
(`web/src/posts/*`).

### Migrated composition sites (`format!("{base}…")` → `compose(base, …)`)

- `server/src/feed/regenerate.rs` — `self_url`, per-surface `canonical_url`;
  delete `percent_encode_path`.
- `server/src/feed/worker.rs` (`ping_websub`, ~207) — the WebSub ping topic URL.
- `server/src/atompub/posts.rs` — `collection_url` (~165), paging `next` (query,
  ~171), the `Location` header (~441).
- `server/src/atompub/mapping.rs` — `edit_uri` (~130), entry `href` (~144).
- `server/src/atompub/service.rs` (~29, and the service-doc URLs ~42/48),
  `server/src/atompub/media.rs` (~100/140), `server/src/atompub/rsd.rs` (~33-34,
  `service_url`/`homepage_url`).
- `web/src/invites/mod.rs` (~65) and its CLI twin `server/src/commands.rs`
  (~307) — the `/register?invite_code=…` link.

(All consume `base_url`; each must migrate per D5's forcing function even though
the target field stays `String`.)

### Storage / boundary changes

- `storage/src/site_config.rs`: `get_identity` / `set_identity` stop stripping
  the trailing slash by hand (the type normalizes). `get_identity` now parses
  the stored string into `Option<AbsoluteUrl>`; a corrupt/legacy stored value
  that no longer parses is **purged and read as unset** — mirroring the
  `feed_events` unparseable-`feed_url` purge — rather than hard-failing the read
  (a hard fail would 500 every feed and brick the settings page that's the only
  UI to fix it). `get_feeds_websub_hub_url` / `get_feeds_config` parse into
  `Option<AbsoluteUrl>` the same way (empty → `None`; non-empty-invalid →
  purge + `None`). `set_feeds_config` / `set_identity` write via the type's
  `Display`/`AsRef`.
- `web/src/site/mod.rs`:
  `update_site_identity(title, base_url: Option<AbsoluteUrl>)`; remove the
  manual scheme + trailing-slash logic; keep empty→clear semantics as ADR-0065
  omit/None.

## Acceptance criteria

1. `common::absolute_url::AbsoluteUrl` exists as `struct AbsoluteUrl(String)`
   with `#[derive(StrNewtype)]` + hand-written `FromStr`. Unit tests assert:
   rejects non-`http(s)` schemes (`file:`, `ftp:`, `javascript:`), rejects
   host-less / unparseable input, and normalizes — host case lowercased,
   `:80`/`:443` stripped, `https://h` → `https://h/`, and `FromStr` idempotent.
2. `AbsoluteUrl::join` composes `base` + a site-absolute path with **no double
   slash** and correct encoding; a garbage path yields `Err`; composing a
   canonical `FeedPath` yields an unchanged path (the
   `percent_encode_path`-removal regression test). Site-surface canonical URLs
   retain their trailing slash. `percent_encode_path` no longer exists in
   `server/src/feed/regenerate.rs`.
3. `SiteIdentity.base_url`, `FeedMetadata.hub_url`, and
   `FeedsConfig.websub_hub_url` are `Option<AbsoluteUrl>`. (The composed URL
   fields listed under "NOT typed this PR" remain `String`.)
4. No `format!`/`println!` concatenating a base origin with a path remains in
   `server/src` or `web/src` (grep-verifiable — including `rsd.rs`,
   `commands.rs`, all `atompub/*`); each listed site composes via the
   `compose`/`join` helper.
5. **Behavior is preserved for the unset-`base_url` case:** with
   `base_url = None`, feed and atompub URLs are still the same root-relative
   strings as before (asserted by an existing or new test that regenerates a
   feed with no base configured).
6. `SiteIdentity.base_url` serializes/round-trips as url-canonical form
   (trailing slash); a stored value **without** the slash still deserializes;
   `site_config.rs` read/write contains no manual `trim_end_matches('/')` for
   base_url; the JSON wire shape is unchanged (still a nullable plain string).
7. `update_site_identity`'s wire arg is `Option<AbsoluteUrl>`; a malformed URL
   is rejected on the wire; the settings form shows an inline client-side error
   before submit for a malformed URL; submitting an empty base_url clears it to
   `None`. An e2e asserts the clear-to-None path end to end.
8. `web/src/site/mod.rs` contains no manual scheme check; a scheme-less or
   non-http(s) base_url is rejected solely by the type (a server-fn test asserts
   the result is non-OK, per ADR-0065 — not a specific message).
9. `get_feeds_websub_hub_url` (and `get_identity` for `base_url`) return
   `Option<AbsoluteUrl>`, parsing the stored string on read (empty → `None`); an
   unparseable stored value is **purged and read as unset** (self-heal), not a
   hard read error. Tests cover a value round-tripping through the typed
   getter/setter and the purge-on-unparseable path.
10. A follow-up issue is filed (plan's first task) for the absolute-or-relative
    / relative-path type covering the composed feed/atompub URLs +
    `permalink`/`preview_url`; those fields remain `String` in this PR.
11. A numberless ADR draft in `docs/adr/drafts/` records the
    `url`-in-`common`/wasm dependency decision.
12. `cargo xtask validate --no-e2e` is clean; the base_url-clear e2e passes
    under the full gate.

## Out of scope

- Typing the composed absolute-or-relative URL fields or introducing the
  relative-path type (D8 — spun out): feed `self_url`/`canonical_url`, atompub
  `FeedMeta.*`/`id`/`edit_uri`/ `edit_media_uri`/`content_src`, post
  `permalink`/`preview_url`.
- Making `base_url` mandatory / changing the unset-`base_url` behavior (rejected
  resolution; behavior is preserved per AC#5).
- Any change to `FeedPath` (#399) — different grammar; do **not** fold onto it.

## Risks / notes

- **Wasm bundle growth** (`url` + `idna`) — accepted (D2), recorded in the ADR
  (D10). `url` compiles to wasm32.
- **Silent double-slash** if a base_url consumer is missed — mitigated by the
  full sweep (D5), the grep AC (#4), the preserved-relative test (#5), and
  feed/atompub e2e coverage.
- **`?`/`#` encoding semantics** change with `percent_encode_path` removal —
  no-op for canonical feed paths, correct for atompub queries; pinned by AC#2.
- **Shared-vendor cold rebuild** on the new `url` dep (~5–8 min one-time) —
  expected; version-match the existing vendor where possible.
- **base_url visible spelling change** (trailing slash) — approved (D6).
