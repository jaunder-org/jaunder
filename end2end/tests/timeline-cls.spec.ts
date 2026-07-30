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
 * These probes turn the argument into a gate. They were written BEFORE the page sweeps
 * and passed on the pre-sweep tree: green before *and* after is what proves
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
 * The four routes, and the chrome element that must not move on each.
 *
 * `/` paints the masthead hero (the `inner_html` subtree #671 moves into the gate's
 * `children` slot, and the ADR-0041 coincidence surface #653 regressed); the other
 * three paint a `Topbar`. `url` is built from the per-test username, which doubles as
 * the tag — see `seedTaggedPost`.
 */
const ROUTES: {
  name: string;
  url: (user: string) => string;
  chrome: string;
}[] = [
  { name: "/", url: () => "/", chrome: ".j-hero" },
  { name: "/tags/:tag", url: (u) => `/tags/${u}`, chrome: ".j-topbar" },
  { name: "/~:username", url: (u) => `/~${u}`, chrome: ".j-topbar" },
  {
    name: "/~:username/tags/:tag",
    url: (u) => `/~${u}/tags/${u}`,
    chrome: ".j-topbar",
  },
];

for (const route of ROUTES) {
  test(`${route.name} : chrome and first row do not shift across mount`, async ({
    page,
    firstNav,
  }, testInfo) => {
    // Register a fresh user and publish one short post tagged with their own
    // username. The username is unique per run, so it doubles as a collision-free
    // tag — which makes the measured row scopeable to THIS test's post even on the
    // shared `/` timeline, and gives `/tags/:tag` a page whose only row is ours.
    // Short body: no wrap, so a reflow cannot masquerade as a shift.
    const username = await register(page, firstNav);
    await createPostViaApi(page, { body: "cls probe", tags: [username] });

    await expectNoShiftAcrossMount(page, {
      url: route.url(username),
      targets: (p) => [
        { name: "chrome", locator: p.locator(route.chrome) },
        {
          name: "own post head",
          // Scoped by the author handle rendered at `posts/render.rs:203`, so a
          // concurrent worker's post cannot be measured by mistake.
          locator: p
            .locator(".j-post", {
              has: p.locator(".j-post-handle", { hasText: `@${username}` }),
            })
            .locator(".j-post-head"),
        },
      ],
      afterMount: async (p) => {
        // Proves the reactive tree really mounted, so a zero-shift result cannot be
        // a frozen-projector no-op. `.j-scroll` is emitted only by `TimelineRows`.
        await expect(p.locator(".j-scroll").first()).toBeVisible({
          timeout: slowBrowserTimeoutMs(testInfo, 10_000),
        });
      },
      tolerancePx: 0,
    });
  });
}
