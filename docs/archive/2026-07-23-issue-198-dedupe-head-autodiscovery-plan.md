# #198 dedupe `<head>` autodiscovery links — Implementation Plan

> **For agentic workers:** Execute task-by-task with `jaunder-iterate`
> (delegating to a subagent via `jaunder-dispatch` when useful). Steps use
> checkbox (`- [ ]`) syntax.

**Spec:**
`docs/superpowers/specs/2026-07-23-issue-198-dedupe-head-autodiscovery.md` (the
"what/why"). **Issue:** jaunder-org/jaunder#198.

**Goal:** Stop the post-boot `<head>` from carrying two identical sets of
feed/RSD autodiscovery `<link>`s: mark the projector-painted copies and drop
them at CSR boot, so the reactive `FeedDiscovery`/`RsdDiscovery` components own
the single set.

**Architecture:** `web::render::render_discovery` tags each discovery `<link>`
with a `data-jaunder-discovery` marker (shared `const`). A new generic
`client::dom::remove_elements_by_selector(&str)` removes matching nodes;
`csr::mount()` calls it right after the existing `#app` drop and before
`mount_to_body`, so the reactive components then produce the only set.
Crawlers/no-JS never run the removal.

**Tech Stack:** Rust, Leptos 0.8 / leptos_meta (CSR), `web_sys`, `cargo xtask`,
Playwright.

## Global Constraints

- **No `Co-Authored-By` trailer** on commits (user preference).
- **Wasm-only code must pass wasm clippy** before commit:
  `cargo clippy -p web --target wasm32-unknown-unknown --features csr -- -D warnings`,
  and for the `csr`/`client` crates
  `cargo clippy -p csr --target wasm32-unknown-unknown -- -D warnings`.
- **Per-commit gate:** the pre-commit hook runs full `cargo xtask check`; run it
  first so it passes clean (`jaunder-commit`). No editing during a gated commit.
- **`issue-198` token** stays in the plan/spec filenames.
- Backend parity N/A (no storage/dialect code touched).

---

## Review header

**Scope — in:**

- `web/src/render/mod.rs`: a `DISCOVERY_MARKER_ATTR` const + mark
  `render_discovery`'s `<link>`s; a host test on `render_discovery`.
- `client/src/dom.rs`: new `remove_elements_by_selector(&str)`.
- `csr/src/lib.rs`: call the remover at boot before `mount_to_body`.
- `end2end/tests/feeds.spec.ts`: tighten to one-set; add
  client-side-nav-updates + a no-wasm-fetch crawler assertion.

**Scope — out:** the reactive components (unchanged); feed/RSD endpoints;
authed/cockpit surfaces (no projector head). **No ADR** — assessed per the
spec's decision record: this is a small local convention (server head for
crawlers; reactive owns post-boot), captured by the spec + boot-code comment,
not a cross-cutting invariant. _Reviewer: flag if you'd rather have one._ No
separable concerns to file.

**Tasks:**

1. Mark the projector discovery links (const + `render_discovery` + host test).
2. `client::dom::remove_elements_by_selector` + call it in `csr::mount()`.
3. E2E: one-set assertion, client-side-nav-updates (tag chip), crawler
   no-wasm-fetch.
4. Full gate + wasm-clippy + local e2e sweep.

**Key risks / decisions:**

- **Removal must be marker-scoped** — the projector head also has two
  `rel="stylesheet"` links that must survive (spec §Parity). The
  `data-jaunder-discovery` marker guarantees only discovery links are removed.
- **Parity is verified** (spec table): every projector surface has a matching
  reactive mount, so boot-time removal never leaves a surface linkless.
- **Wasm-only bits** (`remove_elements_by_selector`, the boot call) have no host
  test — they are verified by the Task 3 e2e + wasm clippy, matching the
  existing `client::dom` primitives (coverage-exempt, wasm-only).

---

## Task 1: Mark the projector discovery links

**Files:**

- Modify: `web/src/render/mod.rs` — add `DISCOVERY_MARKER_ATTR` const; mark the
  two `write!`s in `render_discovery` (~lines 174, 186).
- Test: in-file `#[cfg(test)] mod tests` in `web/src/render/mod.rs` (calls the
  private `render_discovery` via `super::`).

**Interfaces:**

- Produces: `pub const DISCOVERY_MARKER_ATTR: &str = "data-jaunder-discovery";`
  in `web::render` (consumed by Task 2's `csr::mount`). `render_discovery`
  output gains the marker on each discovery `<link>`; stylesheet/meta lines
  unchanged.

- [ ] **Step 1: Write the failing host test** in `render/mod.rs`'s test module

```rust
#[test]
fn discovery_links_carry_the_marker_per_surface() {
    use super::{render_discovery, DISCOVERY_MARKER_ATTR};
    // Site: three feed links, all marked, no RSD.
    let site = render_discovery(&PageSeed::SiteTimeline(one_post_page()));
    assert_eq!(site.matches(DISCOVERY_MARKER_ATTR).count(), 3, "{site}");
    assert_eq!(site.matches("rel=\"alternate\"").count(), 3, "{site}");
    assert!(!site.contains("EditURI"), "{site}");
    // Profile: three feed links + one RSD, all four marked.
    let profile = render_discovery(&PageSeed::Profile {
        username: parse_username("bob"),
        page: one_post_page(),
    });
    assert_eq!(profile.matches(DISCOVERY_MARKER_ATTR).count(), 4, "{profile}");
    assert!(profile.contains("rel=\"EditURI\""), "{profile}");
    // Permalink: none.
    assert_eq!(render_discovery(&PageSeed::Permalink(sample_post())), "");
}
```

- [ ] **Step 2: Run, verify it fails**

Run: `cargo nextest run -p web discovery_links_carry_the_marker` Expected: FAIL
— `DISCOVERY_MARKER_ATTR` undefined / marker absent (count 0 ≠ 3).

- [ ] **Step 3: Implement the marker**

Add above `render_discovery` (module scope):

```rust
/// Marker attribute on each projector-painted autodiscovery `<link>`. The CSR boot
/// (`csr::mount`) removes `link[{DISCOVERY_MARKER_ATTR}]` before mounting so the reactive
/// `FeedDiscovery`/`RsdDiscovery` own the single post-boot set (#198). Shared here so the
/// emitter below and the boot-time remover cannot drift.
pub const DISCOVERY_MARKER_ATTR: &str = "data-jaunder-discovery";
```

Add the marker to both `write!`s in `render_discovery` — the feed-link one and
the RSD one, e.g.:

```rust
"<link {marker} rel=\"alternate\" type=\"{mime}\" title=\"{title}\" href=\"{href}\" />",
marker = DISCOVERY_MARKER_ATTR,
```

and likewise `"<link {marker} rel=\"EditURI\" …"` with
`marker = DISCOVERY_MARKER_ATTR`.

- [ ] **Step 4: Run, verify pass**

Run: `cargo nextest run -p web discovery_links_carry_the_marker` Expected: PASS.
Also `cargo nextest run -p web render` stays green (existing head tests
unaffected — the marker is additive).

- [ ] **Step 5: Commit**

```bash
git add web/src/render/mod.rs
git commit -m "feat(web): mark projector autodiscovery links for boot-time dedupe (#198)"
```

Run `cargo xtask check` first (`jaunder-commit`).

---

## Task 2: `remove_elements_by_selector` + drop the links at boot

**Files:**

- Modify: `client/Cargo.toml` — enable the web-sys `"NodeList"` feature (see
  Step 0).
- Modify: `client/src/dom.rs` — add `remove_elements_by_selector`.
- Modify: `csr/src/lib.rs` — call it in `mount()` after
  `remove_element_by_id("app")`, before `mount_to_body`.
- No host test (wasm-only `web_sys`, like the existing `client::dom` primitives;
  verified by Task 3 e2e + wasm clippy).

**Interfaces:**

- Consumes: `web::render::DISCOVERY_MARKER_ATTR` (Task 1).
- Produces: `pub fn remove_elements_by_selector(selector: &str)` in
  `client::dom`.

- [ ] **Step 0: Enable the `NodeList` web-sys feature** in `client/Cargo.toml`

`document.query_selector_all()` returns `web_sys::NodeList`, and its
`.length()`/`.item()` are gated behind the `"NodeList"` feature — which the
`client` crate does **not** currently enable (its web-sys features are
`Window, Storage, Location, Document, Element, Node`, and `rg` finds no other
`query_selector_all`/`NodeList` user to pull it in). Without this the crate
fails to compile. Add `"NodeList"` to the `web-sys` `features = [...]` list.
Verify: `cargo check -p client --target wasm32-unknown-unknown` builds after the
addition.

- [ ] **Step 1: Add the generic primitive** to `client/src/dom.rs`

Mirror `remove_element_by_id`'s shape (raw `web_sys`, no domain types, no-op
off-DOM):

```rust
/// Remove every element matching `selector` from the document; no-op off-DOM or on a
/// selector that matches nothing (or is invalid — `query_selector_all` errs, swallowed).
pub fn remove_elements_by_selector(selector: &str) {
    if let Some(document) = web_sys::window().and_then(|w| w.document()) {
        if let Ok(nodes) = document.query_selector_all(selector) {
            for i in 0..nodes.length() {
                if let Some(node) = nodes.item(i) {
                    if let Ok(el) = node.dyn_into::<web_sys::Element>() {
                        el.remove();
                    }
                }
            }
        }
    }
}
```

Add `use wasm_bindgen::JsCast;` if not already imported (needed for `dyn_into`).

- [ ] **Step 2: Call it at boot** in `csr/src/lib.rs` `mount()`

After `client::dom::remove_element_by_id("app");` and before `mount_to_body`:

```rust
// Drop the projector-painted discovery <link>s so the reactive FeedDiscovery/
// RsdDiscovery mounted below produce the ONLY set (no invisible duplicate). Crawlers/
// no-JS never run this, so their head is unchanged (#198).
client::dom::remove_elements_by_selector(&format!(
    "link[{}]",
    web::render::DISCOVERY_MARKER_ATTR
));
```

- [ ] **Step 3: Build + wasm clippy**

Run: `cargo check -p csr --target wasm32-unknown-unknown` Expected: builds. Run:
`cargo clippy -p csr --target wasm32-unknown-unknown -- -D warnings` Run:
`cargo clippy -p client --target wasm32-unknown-unknown -- -D warnings`
Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add client/src/dom.rs csr/src/lib.rs
git commit -m "feat(csr): drop projector discovery links at boot; add remove_elements_by_selector (#198)"
```

Run `cargo xtask check` first.

---

## Task 3: E2E — one set, client-side-nav update, crawler path

**Files:**

- Modify: `end2end/tests/feeds.spec.ts`.

**Interfaces:**

- Consumes: `register`, `goto`, `click`, `waitForHydration`, `BASE_URL`,
  `createPostViaApi` (supports `tags?: string[]`), `SEL`, `FORMATS` — all
  already imported in `feeds.spec.ts`.

- [ ] **Step 1: Tighten the existing test to assert exactly one set**
      (feeds.spec.ts ~46-90)

In "auto-discovery links are present…", after collecting
`homeLinks`/`userLinks`, add a count assertion so duplicates would fail:

```ts
// #198: exactly one set post-boot (three feed links), not the pre-dedupe six.
expect(homeLinks.length).toBe(3);
```

and for the user timeline add, additionally, the single RSD link:

```ts
expect(userLinks.length).toBe(3);
const rsd = await page.$$eval('head link[rel="EditURI"]', (els) => els.length);
expect(rsd).toBe(1);
```

Also assert the stylesheet links survive the boot-time removal (AC4 —
marker-scoped removal must not touch them):

```ts
// AC4: the two projector stylesheet <link>s must survive the discovery-link removal.
const styles = await page.$$eval(
  'head link[rel="stylesheet"]',
  (els) => els.length,
);
expect(styles).toBe(2);
```

(Keep the existing per-format `.find` + resolve checks.)

- [ ] **Step 2: Add the client-side-nav-updates test**

```ts
test("head discovery links update across a client-side nav, staying a single set", async ({
  page,
}, info) => {
  setTestBudget(60_000);
  await register(page, slowBrowserFirstNavigationTimeoutMs(info, 30_000));
  // Seed a public post carrying a tag so its footer renders a clickable tag chip.
  await createPostViaApi(page, {
    body: "# Tagged\n\nbody",
    tags: ["disco198"],
  });

  await goto(page, "/");
  await waitForHydration(page);
  const siteHrefs = await page.$$eval('head link[rel="alternate"]', (els) =>
    els.map((e) => (e as HTMLLinkElement).href),
  );
  expect(siteHrefs.length).toBe(3); // one set on the Site feed

  // Client-side nav: click the post's tag chip → /tags/disco198 (leptos_router
  // intercepts the same-origin <a>, no full load).
  await click(page, 'a.j-tag[href="/tags/disco198"]');
  await page.waitForURL(`${BASE_URL}/tags/disco198`);

  // The reactive head rewrite (old FeedDiscovery unmounts, SiteTag one mounts) lands in
  // the batch following the route change — poll until it settles rather than read once
  // and race: exactly three alternate links, all now the SiteTag feed.
  await expect
    .poll(async () =>
      page.$$eval(
        'head link[rel="alternate"]',
        (els) =>
          (els as HTMLLinkElement[]).filter((e) => e.href.includes("disco198"))
            .length,
      ),
    )
    .toBe(3);
  const tagHrefs = await page.$$eval('head link[rel="alternate"]', (els) =>
    els.map((e) => (e as HTMLLinkElement).href),
  );
  expect(tagHrefs.length).toBe(3); // exactly one set (no leftover Site links)
  expect(tagHrefs).not.toEqual(siteHrefs); // the SiteTag feed, not the Site feed
});
```

(Confirm the chip selector against the rendered DOM during implementation — spec
cites `a.j-tag[href="/tags/{slug}"]` from `taglist/markup.rs:20`; adjust the
exact class/href if the site-timeline `TagContext` differs.)

- [ ] **Step 3: Add the crawler (no-wasm) assertion**

A raw HTTP fetch never boots wasm — the projector head is served intact:

```ts
test("crawler path keeps the projector discovery links (no wasm)", async ({
  page,
}, info) => {
  // Public content so `/` renders the projector site-timeline head (an empty site falls
  // back to the link-less SPA shell). register() establishes the session for the post.
  await register(page, slowBrowserFirstNavigationTimeoutMs(info, 30_000));
  await createPostViaApi(page, { body: "# Crawlable\n\nbody" });
  // A raw HTTP fetch never boots wasm — the projector head is served intact.
  const res = await page.request.get(`${BASE_URL}/`);
  const html = await res.text();
  expect(html).toContain("data-jaunder-discovery");
  expect((html.match(/rel="alternate"/g) ?? []).length).toBe(3);
});
```

- [ ] **Step 4: Run the affected spec locally**

Run: `cargo xtask e2e-local feeds` Expected: PASS (chromium+firefox × local
backend). Full matrix runs in CI.

- [ ] **Step 5: Commit**

```bash
git add end2end/tests/feeds.spec.ts
git commit -m "test(e2e): assert single discovery-link set, client-side-nav update, crawler path (#198)"
```

Run `cargo xtask check` first.

---

## Task 4: Full gate sweep

**Files:** none (verification).

- [ ] **Step 1: Wasm clippy** —
      `cargo clippy -p web --target wasm32-unknown-unknown --features csr -- -D warnings`,
      `cargo clippy -p csr --target wasm32-unknown-unknown -- -D warnings` →
      clean.
- [ ] **Step 2: Full validate (no e2e)** — `cargo xtask validate --no-e2e` →
      green (static + clippy + coverage; the new host test covers the
      `render_discovery` change; the wasm-only bits are coverage-exempt).
      Foreground, generous timeout.
- [ ] **Step 3: E2E** — `cargo xtask e2e-local feeds` green locally; CI runs the
      full four-combo matrix at PR time.
- [ ] **Step 4:** No commit (verification only); fold any fix back into the
      owning task's commit.

---

## Self-review

- **Spec coverage:** AC1 → Task 1 host test + Task 3 Step 3 crawler fetch; AC2 →
  Task 3 Step 1; AC3 → Task 3 Step 2; AC4 (stylesheets survive) → marker-scoped
  removal (Task 2) + explicit `rel="stylesheet"` count assertion (Task 3 Step
  1); AC5 → Task 3 Step 1 tightening; AC6 → Task 4. All covered.
- **Type consistency:** `DISCOVERY_MARKER_ATTR` (web::render) is the single
  source used by `render_discovery` and `csr::mount`'s selector;
  `remove_elements_by_selector(&str)` names match across Task 2 and its caller.
- **No placeholders:** every step has the actual test/code/command.
