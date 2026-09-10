/**
 * #202 — empirical layout-shift (CLS) assertion for the authed-owner flash.
 * The trusted owner Actions disclosure is outside the custom-theme surface:
 * ownership is unknown at the anonymous projector paint, and public custom CSS
 * must not control author actions. Its trigger fills the viewer-independent
 * slot without moving the post content after mount.
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

test("authed owner: Actions trigger is additive (no content shift)", async ({
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
      await expect(p.locator(".j-post-action-trigger")).toBeVisible({
        timeout: slowBrowserTimeoutMs(testInfo, 10_000),
      });
    },
    tolerancePx: 0, // exact; loosen per-axis only on documented evidence (validate matrix)
  });
});
