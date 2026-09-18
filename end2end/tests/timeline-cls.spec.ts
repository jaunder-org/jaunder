/**
 * #671 — empirical layout-shift (CLS) assertions for the four projector-painted
 * timeline routes.
 *
 * #671 restructures the CSR side of every timeline page (`TimelineGate`, the paint
 * fold, chrome as a memo-gated sibling region). The projector's own render fns are
 * untouched and `adopt_seed` runs before first render, so a seeded page should paint
 * rows immediately and never flash — but that is an *argument*, and the rest of the
 * suite cannot check it: those tests assert content AFTER the mount settles and would
 * pass straight through a visible flash. #653 was exactly such a flash on the tag
 * pages and the suite missed it.
 *
 * These probes turn the argument into a gate. They were written BEFORE the page sweeps
 * and passed on the pre-sweep tree: green before *and* after is what proves
 * preservation; a test authored afterwards would only document the end state.
 *
 * Deterministic by construction via the shared `expectNoShiftAcrossMount` helper
 * (#202) — it holds the wasm so first paint stays the projector's, then gates on
 * fonts, mount, and consecutive stable post-mount geometry frames, never a timer,
 * so it is safe under `workers>1` (#182).
 *
 * `/app` is deliberately absent: it is served `no-store` and is never
 * projector-painted, so it has nothing to coincide with.
 */
import type { Page } from "@playwright/test";
import { test, expect, slowBrowserTimeoutMs } from "./fixtures";
import { signInAsNewUser } from "./helpers";
import { createPostViaApi } from "./posts";
import { expectNoShiftAcrossMount } from "./layout-shift";
import {
  restoreConfigViaTool,
  snapshotConfigViaTool,
  seedConfigViaTool,
} from "./seed";

async function expectSemanticHero(page: Page): Promise<void> {
  const hero = page.locator('[data-jaunder-part="hero"]');
  await expect(hero).toHaveCount(1);
  await expect(hero.locator('[data-jaunder-part="masthead"]')).toHaveCount(1);
  expect(
    await page
      .locator('[data-jaunder-part="masthead"]')
      .evaluate(
        (masthead) =>
          masthead.parentElement?.getAttribute("data-jaunder-part") === "hero",
      ),
  ).toBe(true);
}

/**
 * The four routes, the chrome element that must not move on each, and whether a post
 * row can also be measured there.
 *
 * Every route paints a `Topbar`; `/` injects its shared masthead through the gate's
 * `children` slot, the ADR-0041 coincidence surface #653 regressed. `url` is built
 * from the per-test username, which doubles as the tag.
 *
 * `measureRow` is false for `/` **on purpose, and it is not a weakened assertion**.
 * The other three routes are scoped to this test's unique username/tag, so the page
 * holds exactly one row — ours — and its position can only move if the layout moved.
 * `/` is the shared site-wide timeline: every other test's posts are on it, new posts
 * are *prepended*, and the projector response is cacheable while the CSR refetch is
 * live. So a row's absolute position legitimately differs between the frozen first
 * paint and the post-mount sample — that is content drift, not layout shift, and the
 * measurement cannot tell the two apart. (Observed under the full suite: the same row
 * moved 137px on one attempt and 216px on the retry.) The masthead sits above the
 * rows, is unaffected by row count, and is the target #671 actually puts at risk.
 */
const ROUTES: {
  name: string;
  url: (user: string) => string;
  chrome: string;
  measureRow: boolean;
}[] = [
  { name: "/", url: () => "/", chrome: ".j-topbar", measureRow: false },
  {
    name: "/tags/:tag",
    url: (u) => `/tags/${u}`,
    chrome: ".j-topbar",
    measureRow: true,
  },
  {
    name: "/~:username",
    url: (u) => `/~${u}`,
    chrome: ".j-topbar",
    measureRow: true,
  },
  {
    name: "/~:username/tags/:tag",
    url: (u) => `/~${u}/tags/${u}`,
    chrome: ".j-topbar",
    measureRow: true,
  },
];

for (const route of ROUTES) {
  test(`${route.name} : projector paint does not shift across mount`, async ({
    page,
    tracedContext,
  }, testInfo) => {
    // Seed a fresh user and publish one short post tagged with their own
    // username. The username is unique per run, so it doubles as a collision-free
    // tag — which gives the three scoped routes a page whose only row is ours, and
    // scopes the row locator to THIS test's post. Short body: no wrap, so a reflow
    // cannot masquerade as a shift.
    const username = await signInAsNewUser(page);
    await createPostViaApi(page, { body: "cls probe", tags: [username] });

    // The Local projector caches its anonymous document for five minutes, so configure
    // identity before the guest context's direct document load. This proves the
    // configured masthead—not only the default one—coincides with CSR on first paint.
    const configuredLocalIdentity = route.name === "/";
    let guestContext: Awaited<ReturnType<typeof tracedContext>> | undefined;
    let priorTitle:
      Awaited<ReturnType<typeof snapshotConfigViaTool>> | undefined;
    let priorTagline:
      Awaited<ReturnType<typeof snapshotConfigViaTool>> | undefined;
    try {
      if (configuredLocalIdentity) {
        priorTitle = await snapshotConfigViaTool("site.title");
        priorTagline = await snapshotConfigViaTool("site.tagline");
        await seedConfigViaTool("site.title", "CLS <Site>");
        await seedConfigViaTool("site.tagline", "A <configured> & tagline");
      }
      guestContext = configuredLocalIdentity
        ? await tracedContext()
        : undefined;
      const probePage =
        guestContext === undefined ? page : await guestContext.newPage();
      await expectNoShiftAcrossMount(probePage, {
        url: route.url(username),
        beforeMount: async (p) => {
          await expectSemanticHero(p);
          if (configuredLocalIdentity) {
            await expect(
              p.locator('[data-jaunder-part="site-title"]'),
            ).toHaveText("CLS <Site>");
            await expect(p.locator(".j-topbar .j-sub")).toHaveText(
              "A <configured> & tagline",
            );
            await expect(
              p.locator("[data-jaunder-projected-local-metadata]"),
            ).toHaveCount(4);
            const metadata = [
              ["head > title", "CLS <Site>"],
              ['head > meta[name="description"]', "A <configured> & tagline"],
              ['head > meta[property="og:title"]', "CLS <Site>"],
              [
                'head > meta[property="og:description"]',
                "A <configured> & tagline",
              ],
            ];
            for (const [selector, value] of metadata) {
              const node = p.locator(selector);
              await expect(node).toHaveCount(1);
              if (selector === "head > title") {
                await expect.poll(() => p.title()).toBe(value);
              } else {
                await expect(node).toHaveAttribute("content", value);
              }
            }
          }
        },
        targets: (p) => [
          { name: "chrome", locator: p.locator(route.chrome) },
          // Scoped by the author handle rendered at `posts/render.rs:203`, so a
          // concurrent worker's post cannot be measured by mistake.
          ...(route.measureRow
            ? [
                {
                  name: "own post head",
                  locator: p
                    .locator(".j-post", {
                      has: p.locator(".j-post-handle", {
                        hasText: `@${username}`,
                      }),
                    })
                    .locator(".j-post-head"),
                },
              ]
            : []),
        ],
        afterMount: async (p) => {
          // Proves the reactive tree really mounted, so a zero-shift result cannot be
          // a frozen-projector no-op. `.j-scroll` is emitted only by `TimelineRows`.
          await expect(p.locator(".j-scroll").first()).toBeVisible({
            timeout: slowBrowserTimeoutMs(testInfo, 10_000),
          });
          await expectSemanticHero(p);
          if (configuredLocalIdentity) {
            // `toHaveText` proves text semantics after CSR replaced the projector:
            // markup-looking config remains decoded text, never DOM markup.
            await expect(
              p.locator('[data-jaunder-part="site-title"]'),
            ).toHaveText("CLS <Site>");
            await expect(p.locator(".j-topbar .j-sub")).toHaveText(
              "A <configured> & tagline",
            );
            await expect(
              p.locator("[data-jaunder-projected-local-metadata]"),
            ).toHaveCount(0);
            await expect(p.locator("head > title")).toHaveCount(1);
            await expect.poll(() => p.title()).toBe("CLS <Site>");
            for (const [selector, content] of [
              ['head > meta[name="description"]', "A <configured> & tagline"],
              ['head > meta[property="og:title"]', "CLS <Site>"],
              [
                'head > meta[property="og:description"]',
                "A <configured> & tagline",
              ],
            ]) {
              const metadata = p.locator(selector);
              await expect(metadata).toHaveCount(1);
              await expect(metadata).toHaveAttribute("content", content);
            }
          }
        },
        tolerancePx: 0,
      });
    } finally {
      await guestContext?.close();
      if (configuredLocalIdentity) {
        // This spec writes process-global config; restore the exact prior rows even
        // when the first restoration command fails.
        try {
          if (priorTagline !== undefined) {
            await restoreConfigViaTool("site.tagline", priorTagline);
          }
        } finally {
          if (priorTitle !== undefined) {
            await restoreConfigViaTool("site.title", priorTitle);
          }
        }
      }
    }
  });
}
