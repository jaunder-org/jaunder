/**
 * #671 — empirical layout-shift (CLS) assertions for the four projector-painted
 * timeline routes.
 *
 * #671 restructures the CSR side of every timeline page (`TimelineGate`, the paint
 * fold, chrome as a memo-gated sibling region). The projector's own render fns are
 * untouched and `adopt_seed` runs before first render, so a seeded page should paint
 * rows immediately and never flash — but that is an *argument*, and the rest of the
 * suite cannot check it: those tests assert content AFTER hydration settles and would
 * pass straight through a visible flash. #653 was exactly such a flash on the tag
 * pages and the suite missed it.
 *
 * These probes turn the argument into a gate. They are written BEFORE the page sweeps
 * and pass on the pre-sweep tree: green before *and* after is what proves
 * preservation; a test authored afterwards would only document the end state.
 *
 * Deterministic by construction via the shared `expectNoShiftAcrossMount` helper
 * (#202) — it holds the wasm so first paint stays the projector's, and gates on
 * `document.fonts.ready` + `body[data-hydrated]`, never a timer, so it is safe under
 * `workers>1` (#182).
 *
 * `/app` is deliberately absent: it is served `no-store` and is never
 * projector-painted, so it has nothing to coincide with.
 */
import { test, expect, slowBrowserTimeoutMs } from "./fixtures";
import { register } from "./helpers";
import { createPostViaApi } from "./posts";
import { expectNoShiftAcrossMount } from "./layout-shift";

/**
 * Register a fresh user and publish one short post tagged with their own username.
 *
 * The username is unique per run, so it doubles as a collision-free tag — which makes
 * every measured element scopeable to THIS test's post even on the shared `/`
 * timeline, and gives `/tags/:tag` a page whose only row is ours. Short body: no wrap,
 * so a reflow cannot masquerade as a shift.
 */
async function seedTaggedPost(
  page: Parameters<typeof createPostViaApi>[0],
  firstNav: number,
): Promise<string> {
  const username = await register(page, firstNav);
  await createPostViaApi(page, { body: "cls probe", tags: [username] });
  return username;
}

/** This test's own post, scoped by the author handle rendered at `render.rs:203`. */
function ownPost(
  page: Parameters<typeof createPostViaApi>[0],
  username: string,
) {
  return page.locator(".j-post", {
    has: page.locator(".j-post-handle", { hasText: `@${username}` }),
  });
}

test("/ : masthead and first row do not shift across mount", async ({
  page,
  firstNav,
}, testInfo) => {
  const username = await seedTaggedPost(page, firstNav);

  await expectNoShiftAcrossMount(page, {
    url: "/",
    targets: (p) => [
      // The masthead hero — the `inner_html` subtree #671 moves into the gate's
      // `children` slot, and the ADR-0041 coincidence surface #653 regressed.
      { name: "masthead hero", locator: p.locator(".j-hero") },
      {
        name: "own post head",
        locator: ownPost(p, username).locator(".j-post-head"),
      },
    ],
    afterMount: async (p) => {
      // Proves the reactive tree really mounted, so a zero-shift result cannot be a
      // frozen-projector no-op. `.j-scroll` is emitted only by `TimelineRows`.
      await expect(p.locator(".j-scroll").first()).toBeVisible({
        timeout: slowBrowserTimeoutMs(testInfo, 10_000),
      });
    },
    tolerancePx: 0,
  });
});

test("/tags/:tag : topbar and first row do not shift across mount", async ({
  page,
  firstNav,
}, testInfo) => {
  const username = await seedTaggedPost(page, firstNav);

  await expectNoShiftAcrossMount(page, {
    url: `/tags/${username}`,
    targets: (p) => [
      { name: "topbar", locator: p.locator(".j-topbar") },
      {
        name: "own post head",
        locator: ownPost(p, username).locator(".j-post-head"),
      },
    ],
    afterMount: async (p) => {
      await expect(p.locator(".j-scroll").first()).toBeVisible({
        timeout: slowBrowserTimeoutMs(testInfo, 10_000),
      });
    },
    tolerancePx: 0,
  });
});

test("/~:username : topbar and first row do not shift across mount", async ({
  page,
  firstNav,
}, testInfo) => {
  const username = await seedTaggedPost(page, firstNav);

  await expectNoShiftAcrossMount(page, {
    url: `/~${username}`,
    targets: (p) => [
      { name: "topbar", locator: p.locator(".j-topbar") },
      {
        name: "own post head",
        locator: ownPost(p, username).locator(".j-post-head"),
      },
    ],
    afterMount: async (p) => {
      await expect(p.locator(".j-scroll").first()).toBeVisible({
        timeout: slowBrowserTimeoutMs(testInfo, 10_000),
      });
    },
    tolerancePx: 0,
  });
});

test("/~:username/tags/:tag : topbar and first row do not shift across mount", async ({
  page,
  firstNav,
}, testInfo) => {
  const username = await seedTaggedPost(page, firstNav);

  await expectNoShiftAcrossMount(page, {
    url: `/~${username}/tags/${username}`,
    targets: (p) => [
      { name: "topbar", locator: p.locator(".j-topbar") },
      {
        name: "own post head",
        locator: ownPost(p, username).locator(".j-post-head"),
      },
    ],
    afterMount: async (p) => {
      await expect(p.locator(".j-scroll").first()).toBeVisible({
        timeout: slowBrowserTimeoutMs(testInfo, 10_000),
      });
    },
    tolerancePx: 0,
  });
});
