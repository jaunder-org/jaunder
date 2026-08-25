/**
 * #202 — empirical layout-shift (CLS) assertion for the authed-owner flash.
 *
 * The strict follow-up to #181/ADR-0044's structural guards. The own-post action
 * column (`.j-post-acts`) is the one owner affordance deliberately NOT pre-reserved
 * (ownership is unknown at the anonymous projector paint; a per-post gutter can't be
 * pre-painted — see `server/assets/jaunder.css:1282-1286` and
 * `web/src/posts/render.rs:172-179`). It is added client-side at mount as a flex
 * sibling to the right of the post content, and the design's claim is that this is
 * "purely additive — never a content change". This test empirically confirms that
 * bounded case: the owner's own-post content does not move across the
 * projector-paint → wasm-mount transition.
 *
 * Deterministic by construction via the shared `expectNoShiftAcrossMount` helper
 * (holds the wasm to freeze first paint; gates on fonts, mount, and consecutive
 * stable post-mount geometry frames, never a timer) — safe under `workers>1`
 * (#182).
 */
import { test, expect, slowBrowserTimeoutMs } from "./fixtures";
import { signInAsNewUser } from "./helpers";
import { createPostViaApi } from "./posts";
import { expectNoShiftAcrossMount } from "./layout-shift";

test("authed owner: own-post action column is additive (no content shift)", async ({
  page,
}, testInfo) => {
  // signInAsNewUser (not the registeredPage fixture) so we get the username to
  // probe the owner's own author page instead of the shared `/` timeline.
  const username = await signInAsNewUser(page);
  await createPostViaApi(page, { body: "cls probe" }); // short → no wrap/reflow

  // The owner's own post, scoped by author handle (`@username`, rendered at
  // `posts/render.rs:208`). The handle is in the anonymous projector paint, so this
  // scope is stable across BOTH phases and safe under `workers>1`. The author page
  // holds only this test user's posts, so concurrent tests cannot prepend rows above
  // the measured post.
  const ownPost = (p: typeof page) =>
    p.locator(".j-post", {
      has: p.locator(".j-post-handle", { hasText: `@${username}` }),
    });

  await expectNoShiftAcrossMount(page, {
    url: `/~${username}`,
    targets: (p) => [
      { name: "post-head", locator: ownPost(p).locator(".j-post-head") },
      // The RENDERED body div (`posts/render.rs:212`) — not SEL.postBody, which is
      // the composer textarea.
      { name: "post-body", locator: ownPost(p).locator(".j-post-body") },
    ],
    afterMount: async (p) => {
      await expect(ownPost(p).locator(".j-post-acts")).toBeVisible({
        timeout: slowBrowserTimeoutMs(testInfo, 10_000),
      });
    },
    tolerancePx: 0, // exact; loosen per-axis only on documented evidence (validate matrix)
  });
});
