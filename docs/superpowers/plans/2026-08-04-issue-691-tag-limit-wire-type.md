# Type the `list_tags` wire limit — Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with `jaunder-iterate`
> (delegating individual tasks to a subagent via `jaunder-dispatch` when
> useful). Steps use checkbox (`- [ ]`) syntax for tracking.

**Spec:**
[`docs/superpowers/specs/2026-08-04-issue-691-tag-limit-wire-type.md`](../specs/2026-08-04-issue-691-tag-limit-wire-type.md)
**Issue:** [#691](https://github.com/jaunder-org/jaunder/issues/691) ·
**Milestone** #13

**Goal:** Type `list_tags`' `limit` argument as `Option<PageSize>` so the
`1..=50` bound is enforced _by_ deserialization instead of applied after it.

**Architecture:** Reuse the existing `common::pagination::PageSize` rather than
adding the `TagLimit` newtype #691 proposed — see the spec's "The decision"
section. The change is compiler-coupled: the signature, its two callers, and the
in-file unit test cannot compile apart, so they land as one task. The only
behavior change (out-of-range coerced → rejected) is driven by a test that is
red before the change and green after.

**Tech Stack:** Rust; `leptos` `#[server]` via `macros::server`, `input = Json`;
`serde`; `rstest`/`rstest_reuse` + `nextest`; `cargo xtask` as the gate.

## Global Constraints

- **No `Co-Authored-By` trailer** on any commit.
- **Backend parity (ADR-0019):** new HTTP integration tests are
  `#[apply(backends)]`-parametrized, matching their neighbors in `web_tags.rs`.
- **The bound lives once.** No new numeric literal for 1, 50, or the tag default
  may appear **in code** outside `DEFAULT_TAG_LIMIT` and `PageSize`'s own
  declaration. (Doc _prose_ may state the numbers — it must, since the doc-links
  gate forbids linking the now-private const.)
- **Verified rejects stay recorded in the code**, at the site — `prefix`'s
  existing `String` justification is the pattern to match.
- The pre-commit hook runs the full `cargo xtask check`; run it first so it
  passes clean (**`jaunder-commit`**).

## Review header

**Scope in:** `web/src/tags/api.rs`, `web/src/tags/mod.rs`,
`web/src/tags/component.rs`, `server/tests/web/web_tags.rs`. **Scope out:**
`prefix: Option<String>` (documented reject); any `NumNewtype` macro change; the
already-typed `Option<PageSize>` sites in posts/timeline/media; any new
`TagLimit` type.

**Tasks:**

1. Type the wire arg, retire `MAX_TAG_LIMIT`, and flip the out-of-range test
   from clamp to reject.
2. Full-gate verification with e2e (no code changes) — spec A12.

**Key risks / decisions:**

- **`DEFAULT_TAG_LIMIT` must be `#[cfg(feature = "server")]`-gated.** Making it
  private (spec A2) while `#[macros::server]` strips the body from the client
  build would leave it unreferenced there — a `dead_code` warning, and the
  `wasm-clippy` step runs with `-D warnings`. `api.rs:6-7` already gates its
  server-only imports this way; follow it.
- **No intra-doc link to `DEFAULT_TAG_LIMIT` may survive.** It becomes private
  and `cfg`-gated; the doc-links gate rejects a public doc linking a private
  item. `list`'s doc states the number in prose instead.
- **The red test is genuinely red today**, not merely uncompilable:
  `{"limit": 1000}` currently returns `OK`. That is what makes step 2 a real TDD
  check.

---

### Task 1: Type `limit` as `Option<PageSize>`

**Files:**

- Modify: `web/src/tags/api.rs:9-53` (imports, both consts, both doc comments,
  signature, body) and `web/src/tags/api.rs:176` (in-file unit test)
- Modify: `web/src/tags/mod.rs:1,17` (module doc, re-export)
- Modify: `web/src/tags/component.rs:157` (the sole client caller)
- Test: `server/tests/web/web_tags.rs:102-124,146`

**Interfaces:**

- Consumes: `common::pagination::PageSize` — `const fn clamped(u32) -> Self`,
  `const fn exact_limit(self) -> RowLimit`, `PageSize::MAX == 50`. Already
  imported at `api.rs:9`.
- Produces:
  `web::tags::list(prefix: Option<String>, limit: Option<PageSize>) -> WebResult<Vec<TagSummary>>`,
  re-exported as `web::tags::list` alongside the generated `web::tags::List`.
  Neither `DEFAULT_TAG_LIMIT` nor `MAX_TAG_LIMIT` is part of the module's public
  surface any more.

- [ ] **Step 1: Rewrite the out-of-range test from clamp to reject**

Replace `list_tags_clamps_limit_to_max` at
`server/tests/web/web_tags.rs:102-124` entirely with:

```rust
#[apply(backends)]
#[tokio::test]
async fn list_tags_rejects_out_of_range_limit(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;

    // `limit=1000` is outside `PageSize`'s `1..=50`; the typed wire arg rejects it on
    // deserialization instead of coercing it down to the cap (#691). Mirrors
    // `list_my_media_rejects_out_of_range_limit`; the status is asserted only as
    // "not OK" because this endpoint is `input = Json` and that one is form-encoded.
    let (status, _body) = post_json(
        &state,
        <web::tags::List as ServerFn>::PATH,
        serde_json::json!({ "limit": 1000 }),
        None,
    )
    .await;

    assert_ne!(
        status,
        StatusCode::OK,
        "out-of-range tag limit must be rejected"
    );
}
```

The 60-tag seed and the `seed_user_and_tagged_post` call go with the old test —
the request never reaches storage now. In the same file, reword the assert
message at `:146` from `"DEFAULT_TAG_LIMIT is 10"` to
`"the default limit is 10"`, because the const it names becomes private in
step 3. **Do not touch that test's asserted value, and do not touch the other
three tests in the file at all** (spec A8, A9).

- [ ] **Step 2: Run it, verify it fails**

Run:
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-691-tag-limit -- devtool pg run -- cargo nextest run -p jaunder web::web_tags`

Expected: **FAIL** — `list_tags_rejects_out_of_range_limit` fails on both
backends with
`assertion left != right failed: out-of-range tag limit must be rejected`,
because today `PageSize::clamped(1000)` coerces to 50 and the endpoint answers
`200 OK`. The other four `web_tags` tests pass.

- [ ] **Step 3: Type the argument**

`web/src/tags/api.rs` — replace the two consts (`:14-28`) with one private,
server-gated const, and rewrite `list`'s doc comment and signature (`:30-44`).
Written out in full because the doc comments are themselves deliverables (spec
A6) and no test can pin their content:

```rust
/// Suggestions returned to the autocomplete dropdown when the caller doesn't
/// specify a limit — deliberately fewer than a listing page's 50.
///
/// Private, and gated to the server build: `list`'s `None` branch is its only
/// consumer now that the dropdown relies on the default rather than restating
/// the number at the call site, and `#[macros::server]` strips that body from
/// the client build (which would leave this dead there).
#[cfg(feature = "server")]
const DEFAULT_TAG_LIMIT: PageSize = PageSize::clamped(10);

/// Returns tag suggestions for the autocomplete dropdown.
///
/// `prefix` is a case-insensitive prefix match against the canonical slug;
/// `None` or whitespace-only returns the alphabetically-first tags. `limit`
/// defaults to 10 suggestions and is bounded `1..=50` by [`PageSize`]; an
/// out-of-range value is **rejected on the wire** by the serde bridge, not
/// coerced down to the cap.
///
/// `limit` is a [`PageSize`] rather than a tags-specific `TagLimit` newtype
/// (#691), for two reasons. The `1..=50` bound is `PageSize`'s own and belongs
/// in one place — the deleted `MAX_TAG_LIMIT` was literally defined *as*
/// `PageSize::MAX`, so a `TagLimit` would restate two numbers that can then
/// drift. And a `TagLimit` carrying `clamp` would carry a door nothing calls:
/// the `NumNewtype` serde bridge re-runs the bound and rejects, it never
/// coerces, so `clamped` is reachable only from Rust. That coercing door is for
/// public params like `AtomPub`'s `?limit=`, not for a `#[server]` wire arg.
///
/// `prefix` stays `String` (not `Tag`): it is a partial search fragment matched
/// with SQL `LIKE prefix%`, not a complete tag value — typing it `Tag` would
/// reject valid partials (ADR-0063 §4 boundary policy; #409 Decision 7).
#[macros::server(input = Json, skip(prefix))]
pub async fn list(
    prefix: Option<String>,
    limit: Option<PageSize>,
) -> WebResult<Vec<TagSummary>> {
    let posts = expect_context::<Arc<dyn PostStorage>>();
    // `exact_limit`, not `fetch_limit`: the dropdown shows what it gets and has no
    // "load more", so an extra probing row would just be fetched and discarded.
    let resolved_limit = limit.unwrap_or(DEFAULT_TAG_LIMIT).exact_limit();
    let records = posts.list_tags(prefix.as_deref(), resolved_limit).await?;
    Ok(records
        .into_iter()
        .map(|rec| TagSummary {
            slug: rec.tag_slug.clone(),
            display: TagLabel::from(rec.tag_slug),
        })
        .collect())
}
```

Then the three mechanical follow-ons the compiler forces:

- `web/src/tags/api.rs` — in the nested `mod server` test module (`:88-97`), add
  `use common::pagination::PageSize;` and change the call at `:176` to
  `list(Some(SECRET_PREFIX.to_string()), Some(PageSize::clamped(5))).await`.
- `web/src/tags/mod.rs:17` — `pub use api::{list, List};`. Read the module doc
  at `:1-16` and drop any mention of `DEFAULT_TAG_LIMIT` / `MAX_TAG_LIMIT` if
  present.
- `web/src/tags/component.rs:157` —
  `if let Ok(results) = crate::tags::list(Some(prefix), None).await {`.

Leave `use common::pagination::PageSize;` at `api.rs:9` ungated — the signature
names `PageSize` in both builds.

- [ ] **Step 4: Run the tests, verify they pass**

Run:
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-691-tag-limit -- devtool pg run -- cargo nextest run -p jaunder web::web_tags`
Expected: **PASS** — all five tests, both backends. In particular
`list_tags_uses_default_limit_when_unspecified` still returns exactly 10 (spec
A8) and `list_tags_returns_all_when_prefix_absent` still passes with its
explicit `{"prefix": null, "limit": null}` body (spec A9).

Run:
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-691-tag-limit -- cargo nextest run -p web --features server list_emits_its_derived_span`
Expected: **PASS** — the span still records `limit` and still never carries
`prefix` (spec A10).

- [ ] **Step 5: Check the criteria that no test covers**

Run:
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-691-tag-limit -- rg -n MAX_TAG_LIMIT --glob '!docs/archive/**' --glob '!docs/superpowers/**'`
Expected: **no matches** (spec A3). Note two things a mechanical runner gets
wrong here: no-match is `rg` exit code **1**, which `devtool run` reports as
`ok:false` — that is the _passing_ signal for this step. And the
`--glob '!docs/superpowers/**'` widens A3's stated exclusion beyond
`docs/archive/`, because the spec and this plan both discuss the name; say so in
the PR body rather than letting it read as a silent deviation.

Run:
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-691-tag-limit -- rg -n 'Option<u32>' web/src/tags`
Expected: **no matches** (spec A1).

- [ ] **Step 6: Commit**

Run the gate first —
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-691-tag-limit -- cargo xtask check`
— and fix anything it reports. It covers the two criteria most at risk here: the
doc-links gate (no link to the now-private const) and `wasm-clippy`
(`-p web --features csr`), which is what proves the `#[cfg(feature = "server")]`
gate on `DEFAULT_TAG_LIMIT` is correct and the `component.rs` edit compiles.

```bash
git add web/src/tags/api.rs web/src/tags/mod.rs web/src/tags/component.rs server/tests/web/web_tags.rs
git commit -m "refactor(web): type the list_tags wire limit as PageSize (#691)"
```

---

### Task 2: Full-gate verification with e2e

**Files:** none — verification only. No commit.

**Interfaces:** consumes Task 1's committed change; produces the green signal
`jaunder-ship` needs before opening the PR.

- [ ] **Step 1: Run the verify-only gate first (spec A11)**

Run:
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-691-tag-limit -- cargo xtask validate --no-e2e`

Expected: **PASS**. This is not redundant with Task 1's `cargo xtask check`:
`check` **auto-fixes** formatting, so a green `check` can leave the tree mutated
_after_ the commit. `validate --no-e2e` is verify-only and never mutates, so it
is the gate that actually proves the committed tree is clean. If it fails on
formatting, the commit needs amending before Step 2.

- [ ] **Step 2: Run the full local gate**

Run (Bash background mode — this is long and cold):
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-691-tag-limit -- cargo xtask validate`

Expected: **PASS**, `ok=true`, across all four
`{sqlite,postgres}×{chromium,firefox}` e2e combos.

- [ ] **Step 3: Confirm the e2e specs that cover this endpoint actually ran**

`docs/coverage/server-fns-evidence.json` maps `tags::list` to Playwright specs
in `end2end/tests/posts.spec.ts` (`:870`, `:912`, `:933`, …). They are the only
coverage proving the autocomplete dropdown still populates after `Some(10)` →
`None`, which is why `--no-e2e` is not sufficient here (spec A12). If `validate`
reports the server-fn coverage step green, `tags::list` is still covered; if it
flags `tags::list` as uncovered, that is a real regression from the caller
change, not a flake — investigate before shipping.

- [ ] **Step 4: Read the sidecar on any failure**

`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-691-tag-limit -- jq '.steps[] | select(.ok == false)' .xtask/last-result.json`
