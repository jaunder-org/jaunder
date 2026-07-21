/**
 * Shared post-creation helpers for the e2e suite (#262).
 *
 * "Create a post" was inlined dozens of times in two styles — a `page.request`
 * API call and a UI-composer flow — plus a half-extracted local `publishPost` in
 * feeds.spec.ts. These promote both into one place with a contextful assertion
 * and typed result.
 */

import { expect, type Locator, type Page } from "@playwright/test";
import { withTimedAction } from "./actions";
import { BASE_URL, click, goto, waitForSelector } from "./helpers";
import { SEL } from "./selectors";

/** Create a post via `POST /api/create_post`. Wraps the request in
 *  `withTimedAction` so it appears in the OTEL trace, asserts success with a
 *  contextful message, and returns the typed JSON. `publish` defaults to `true`;
 *  `slug` maps to the `slug_override` wire field; `tags` is sent only when
 *  provided (matching the current no-tag call sites). The fields are nested under
 *  an `args` wrapper (#299): the endpoint takes a single typed arg-struct. */
export async function createPostViaApi(
  page: Page,
  opts: {
    body: string;
    tags?: string[];
    publish?: boolean;
    slug?: string | null;
  },
): Promise<{ post_id: number; permalink: string }> {
  const res = await withTimedAction(page, "api.create_post", () =>
    page.request.post(`${BASE_URL}/api/create_post`, {
      data: {
        args: {
          body: opts.body,
          format: "markdown",
          slug_override: opts.slug ?? null,
          publish: opts.publish ?? true,
          ...(opts.tags ? { tags: opts.tags } : {}),
        },
      },
    }),
  );
  expect(
    res.ok(),
    `create_post failed (${res.status()}): ${await res.text()}`,
  ).toBeTruthy();
  return (await res.json()) as { post_id: number; permalink: string };
}

/** Compose and submit a post through the `/posts/new` UI: navigate, fill the
 *  body (and the summary / slug inputs when provided), click publish/save, and
 *  wait for the save-summary panel. Returns the `.j-save-summary` locator for
 *  follow-up assertions. Serves the plain `goto("/posts/new")` composer sites;
 *  the home-page `.j-composer` flow is a separate path this does not cover. */
export async function composePost(
  page: Page,
  opts: { body: string; summary?: string; slug?: string; publish: boolean },
): Promise<Locator> {
  await goto(page, "/posts/new");
  await page.fill(SEL.postBody, opts.body);
  if (opts.summary !== undefined) {
    await page.fill("#compose-summary", opts.summary);
  }
  if (opts.slug !== undefined) {
    await page.fill('input[name="slug_override"]', opts.slug);
  }
  await click(page, SEL.publishButton(opts.publish ? "true" : "false"));
  await waitForSelector(page, SEL.saveSummary);
  return page.locator(SEL.saveSummary);
}
