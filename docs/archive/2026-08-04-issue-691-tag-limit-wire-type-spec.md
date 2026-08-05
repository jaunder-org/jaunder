# Issue #691 — type the `list_tags` wire limit

**Issue:** [#691](https://github.com/jaunder-org/jaunder/issues/691) — _types:
TagLimit NumNewtype — retire the hand-rolled clamp in list_tags_ **Milestone:**
#13 Domain-value type safety (newtypes) **Branch:**
`worktree-issue-691-tag-limit`

## The issue as filed is partly stale

#691 was written against a `list_tags` body that read:

```rust
let resolved_limit = limit.unwrap_or(DEFAULT_TAG_LIMIT).clamp(1, MAX_TAG_LIMIT);
```

**#696 already deleted that hand-rolled clamp.** `web/src/tags/api.rs` today
reads:

```rust
pub const DEFAULT_TAG_LIMIT: u32 = 10;              // :21
pub const MAX_TAG_LIMIT: u32 = PageSize::MAX;       // :28

pub async fn list(prefix: Option<String>, limit: Option<u32>) -> WebResult<Vec<TagSummary>> {
    let resolved_limit = PageSize::clamped(limit.unwrap_or(DEFAULT_TAG_LIMIT)).exact_limit(); // :44
```

So the clamp is no longer hand-rolled — it already runs through `PageSize`'s
validated door. **The residual defect is narrower and real:** `limit` still
crosses the `#[server]` boundary as a bare `Option<u32>`, and the bound is
applied _after_ deserialization rather than _by_ it.

## The decision: reuse `PageSize`, do not add a `TagLimit`

#691 proposed a new `TagLimit` `NumNewtype` with
`min = 1, max = 50, default = 10, clamp`. **This spec rejects that** and types
the wire arg `Option<PageSize>` instead.

Three reasons, all verified against the tree:

1. **The bound is already `PageSize`'s.** `common/src/pagination.rs:20-29`
   declares `1..=50`, and `api.rs:28` already defines `MAX_TAG_LIMIT` _as_
   `PageSize::MAX`. A `TagLimit` would restate the same two numbers in a second
   place, where they can drift. ADR-0063 §5 says to adopt an existing newtype
   everywhere its value appears.

2. **`TagLimit`'s `clamp` flag would be dead.** The `NumNewtype` serde bridge
   re-runs the bound check on deserialize (`macros/src/num_newtype.rs:325-330`);
   it never coerces. `clamped` is a Rust-side constructor only. Once the wire
   arg is typed, nothing would call `TagLimit::clamped` — the type would carry a
   generated affordance with no caller. `common/src/pagination.rs:16-19` states
   the policy explicitly: the `clamp` affordance is "used by the public
   `AtomPub` `?limit=` param; the web `#[server]` args instead **reject**
   out-of-range on the wire via the serde bridge."

3. **`Option<PageSize>` is the established shape; tags is the outlier.** It is
   already the wire type at `web/src/posts/api.rs:403`,
   `web/src/timeline/api.rs:42,60,71,100,113`, and `web/src/media/api.rs:73`.
   Only `tags` still takes a bare integer.

The only genuine tags-specific quantity is the **default of 10** (the dropdown
shows fewer rows than a listing page). That is call-site policy, not a type
invariant — recorded exactly the way AtomPub's default of 25 already is, as
`PageSize::clamped(10)`.

## Behavior change, stated so it is not mistaken for a regression

Today `limit: 1000` is **coerced to 50** and returns 50 rows. After this change
it is **rejected on the wire** (non-`OK`), because the typed arg fails
deserialization. This is intentional and is the documented policy for web
`#[server]` args. The sole client caller never sends an out-of-range limit.

`list_my_media_rejects_out_of_range_limit` (`server/tests/web/web_media.rs:96`)
is the shape this mirrors, with one caveat recorded so it is not over-read:
`list_my_media` is **form-urlencoded** (`post_form`, `"limit=999"`) while
`tags::list` is `input = Json` (`post_json`). The two codecs need not produce
the same status for a rejected argument, so A7 below asserts only "not `OK`" —
the same assertion the media test makes — rather than pinning a specific code
the Json codec has not been observed to produce.

## Scope

### In

- `web/src/tags/api.rs`
  - `limit: Option<u32>` → `limit: Option<PageSize>`.
  - `DEFAULT_TAG_LIMIT: pub const … u32 = 10` → **private**
    `const DEFAULT_TAG_LIMIT: PageSize = PageSize::clamped(10);`. It has no
    consumer outside this file once the client passes `None`, and leaving `pub`
    surface with no caller is the same objection this spec levels at
    `TagLimit`'s `clamp` flag.
  - **Delete `MAX_TAG_LIMIT`** — it is `PageSize::MAX` and the type now carries
    it.
  - **Rewrite both stale doc comments.** `:14-20` (on `DEFAULT_TAG_LIMIT`)
    currently says `PageSize::clamped` "is the coerce-rather-than-reject policy
    a public `limit=` param wants" — after this change that is the opposite of
    what the endpoint does. `:30-38` (on `list`) says `limit` "is clamped at
    [`MAX_TAG_LIMIT`]" — both a false behavioral claim and an intra-doc link to
    a deleted item, which the doc-links gate would reject. Neither may survive
    as written, and neither may intra-doc-link the now-private
    `DEFAULT_TAG_LIMIT`.
  - Body becomes `limit.unwrap_or(DEFAULT_TAG_LIMIT).exact_limit()` — the
    `PageSize::clamped(...)` call disappears with the untyped arg.
  - Doc comment records why there is no `TagLimit` (see "Reject record" below).
- `web/src/tags/mod.rs:17` — `pub use api::{list, List};`. Both consts leave the
  re-export. Check the module doc (`:1`) for a stale mention of either name.
- `web/src/tags/component.rs:157` — `list(Some(prefix), Some(10))` →
  `list(Some(prefix), None)`, deleting the caller-side duplicate of the default.
- `web/src/tags/api.rs:176` (unit test) — `Some(5)` →
  `Some(PageSize::clamped(5))`.
- `server/tests/web/web_tags.rs` — `list_tags_clamps_limit_to_max` (`:104`)
  becomes `list_tags_rejects_out_of_range_limit`. Its `MAX_TAG_LIMIT` mentions
  in the comment at `:107` and the assert message at `:123` go with it; these
  are the only other live references to the name in the tree.

### Out — verified rejects, do not re-flag

- **`prefix: Option<String>`** stays a `String`. Already justified in-place at
  `api.rs:36-38`: a partial `LIKE prefix%` search fragment, not a complete tag
  value (ADR-0063 §4; #409 Decision 7).
- **No new `TagLimit` type** — see the decision above.
- **No `NumNewtype` macro change.** A clamping `Deserialize` was considered and
  rejected: it would make one newtype's wire contract differ from every other.
- **`RowLimit`/`exact_limit` semantics** are untouched — `exact_limit`, not
  `fetch_limit`, remains correct (the dropdown has no "load more";
  `api.rs:42-43`).
- **The other `Option<PageSize>` sites** (posts, timeline, media, and
  `timeline/server.rs:63,88,113,143`) are already typed and are not touched.

## Reject record

The rationale lives **in the code, at the site** — the same way `prefix`'s
`String` justification does — so the next reader of `api.rs` does not re-propose
`TagLimit`. Substance to capture: the bound is `PageSize`'s and belongs in one
place; a distinct `TagLimit` would restate it and carry a dead `clamp` flag,
because the serde bridge rejects rather than coerces (the coercing door is for
public params like AtomPub's). A one-line summary also goes in the #691 close
comment.

(#697 will add a newtype-adoption gate with a verified-rejects allowlist and
could harvest this. That gate does **not** exist in the tree yet — no `xtask`
step, ADR, or `CONTRIBUTING.md` text mentions it — so it is a possible future
consumer, not a justification this cycle depends on. The next-reader reason
stands alone.)

No ADR. The governing decisions already exist (ADR-0063 §§2, 4, 5; the
`clamp`-vs-reject policy recorded at `common/src/pagination.rs:16-19` under
#696); this cycle applies them rather than deciding anything novel.

## Acceptance criteria

Each is observable — a reviewer can check it against the tree.

- **A1** `web/src/tags/api.rs`'s `list` signature is
  `(prefix: Option<String>, limit: Option<PageSize>)`.
  `rg -n 'limit: Option<u32>' web/src` returns no hit in `tags/`.
- **A2** `DEFAULT_TAG_LIMIT` is declared exactly
  `const DEFAULT_TAG_LIMIT: PageSize = PageSize::clamped(10);` — **no `pub`** —
  and `web/src/tags/mod.rs:17` reads `pub use api::{list, List};`.
- **A3** `rg -n 'MAX_TAG_LIMIT' --glob '!docs/archive/**'` returns **zero**
  hits: no definition, no re-export, no doc-comment reference, and none of the
  `server/tests/web/web_tags.rs` comment or assert-message text.
- **A4** `list`'s body contains no `PageSize::clamped` call; the limit is
  resolved as `limit.unwrap_or(DEFAULT_TAG_LIMIT).exact_limit()`.
- **A5** `web/src/tags/component.rs` calls
  `crate::tags::list(Some(prefix), None)` — no numeric literal limit remains at
  the call site.
- **A6** A doc comment in `web/src/tags/api.rs` states why `limit` is a
  `PageSize` rather than a tags-specific newtype. It must cite **`#691`** (the
  mechanical hook) and must state **both** reasons in substance: (a) the
  `1..=50` bound is `PageSize`'s and belongs in one place, and (b) a
  `TagLimit`'s `clamp` flag would have no caller because the serde bridge
  rejects out-of-range rather than coercing.
- **A7** `server/tests/web/web_tags.rs` contains
  `list_tags_rejects_out_of_range_limit`, which posts `{"limit": 1000}` to
  `<web::tags::List as ServerFn>::PATH` and asserts the status is **not** `OK`.
  It is `#[apply(backends)]`-parametrized like its neighbors.
  `list_tags_clamps_limit_to_max` no longer exists.
- **A8** `list_tags_uses_default_limit_when_unspecified` still passes with its
  assertions unchanged — seeding 20 tags and posting `{}` still returns
  exactly 10. This is the regression lock on the default surviving its retyping,
  and on `None` still meaning 10 now that the client relies on it. (Its assert
  _message_ may be reworded, since the const it names is now private; the
  asserted value may not.)
- **A9** The three remaining `web_tags.rs` tests
  (`..._returns_empty_when_no_tags`, `..._returns_all_when_prefix_absent`,
  `..._filters_by_prefix_case_insensitive`) pass with **no edit at all**. This
  includes `..._returns_all_when_prefix_absent`, which posts an explicit
  `{"prefix": null, "limit": null}` — so it already covers JSON `null`
  deserializing to `None` for the typed arg, and **no new null-handling test is
  to be added**.
- **A10** `list_emits_its_derived_span_recording_limit_but_not_prefix`
  (`web/src/tags/api.rs:162`) still passes: the span still records a `limit`
  field and still never carries the `prefix` value. Only its argument literal
  changes (A-scope).
- **A11** `devtool run -- cargo xtask validate --no-e2e` is green, including the
  doc-links and coverage gates, and the `wasm-clippy` step
  (`xtask/src/steps/static_checks.rs:58-90`, `-p web --features csr`) which is
  what covers the `#[cfg(target_arch = "wasm32")]` `component.rs` edit in A5.
- **A12** The **full** `devtool run -- cargo xtask validate` (with e2e) is green
  before the PR is opened. This is not redundant with A11:
  `docs/coverage/server-fns-evidence.json` maps `tags::list` to six Playwright
  specs (`end2end/tests/posts.spec.ts:870,912,933`, …), and they are the only
  coverage proving the autocomplete dropdown still populates after the
  `Some(10)` → `None` change. `--no-e2e` cannot show that.
