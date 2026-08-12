/**
 * Shared post helpers for the e2e suite — creation (#262), plus navigation to
 * and assertions over a saved post (#873). One place for both creation styles
 * (a `page.request` API call and a UI-composer flow), with a contextful
 * assertion and typed result.
 */

import { expect, type Locator, type Page } from "@playwright/test";
import { withTimedAction } from "./actions";
import { BASE_URL, click, waitForSelector } from "./helpers";
import { navigateInApp } from "./navigate";
import { SEL } from "./selectors";

/** The two user-selectable post formats, each with the `.j-seg` button label
 *  that selects it and the HTML element its emphasis markup renders to.
 *
 *  `*emphasis*` is deliberately ambiguous between the two: Markdown reads it as
 *  italic and Org as bold, so the rendered element reveals which format a post
 *  was actually *saved* with (`common/src/render.rs:1647` and `:1696`). That is
 *  what makes a format round-trip observable end-to-end — the toggle's
 *  `is-selected` class only proves the buttons behave as a radio group. */
const FORMAT = {
  markdown: { label: "Markdown", tag: "em" },
  org: { label: "Org", tag: "b" },
} as const;

/** A post format, named as the tests name it. */
export type PostFormatName = keyof typeof FORMAT;

/** The body to compose when a test intends to assert on the rendered format.
 *  Paired with [`expectRenderedFormat`], which asserts on its rendering — keep
 *  them together, since that assertion has no way to check the caller filled
 *  this. */
export const FORMAT_PROBE_BODY = "*emphasis*";

/** What `FORMAT_PROBE_BODY` renders to as text, once the markup is consumed. */
const FORMAT_PROBE_TEXT = "emphasis";

/** Create a post via `POST /api/posts/create`. Wraps the request in
 *  `withTimedAction` so it appears in the OTEL trace, asserts success with a
 *  contextful message, and returns the typed JSON. `publish` defaults to `true`;
 *  `slug` maps to the `slug_override` wire field; `tags` is sent only when
 *  provided (matching the current no-tag call sites). The fields are nested under
 *  a `post` wrapper (#299): the endpoint takes a single typed input struct, and
 *  the wire key is the parameter's name. */
export async function createPostViaApi(
  page: Page,
  opts: {
    body: string;
    tags?: string[];
    publish?: boolean;
    slug?: string | null;
  },
): Promise<{ post_id: number; permalink: string }> {
  const res = await withTimedAction(page, "api.posts.create", () =>
    page.request.post(`${BASE_URL}/api/posts/create`, {
      data: {
        post: {
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
    `posts::create failed (${res.status()}): ${await res.text()}`,
  ).toBeTruthy();
  return (await res.json()) as { post_id: number; permalink: string };
}

/** Compose and submit a post through the `/posts/new` UI: fill the body (and the
 *  summary / slug inputs when provided), click publish/save, and wait for the
 *  save-summary panel. Returns the `.j-save-summary` locator for follow-up
 *  assertions. The home-page `.j-composer` flow is a separate path this does not
 *  cover.
 *
 *  **The caller must already be on `/posts/new`** — normally by entering there,
 *  `const page = await registeredPage("/posts/new")`. A `goto` here would cost
 *  a second document load on a page whose entry was already the composer
 *  (#867). Nothing in the app links to `/posts/new` (#896), so reaching it is
 *  always an entry, never an in-app move. */
export async function composePost(
  page: Page,
  opts: {
    body: string;
    summary?: string;
    slug?: string;
    publish: boolean;
    format?: PostFormatName;
  },
): Promise<Locator> {
  return withTimedAction(page, "flow.compose_post", async () => {
    await waitForSelector(page, SEL.postBody);
    await page.fill(SEL.postBody, opts.body);
    if (opts.format !== undefined) {
      await click(page, SEL.formatButton(FORMAT[opts.format].label));
    }
    if (opts.summary !== undefined) {
      await page.fill(SEL.postSummary, opts.summary);
    }
    if (opts.slug !== undefined) {
      await page.fill('input[name="slug_override"]', opts.slug);
    }
    await click(page, SEL.publishButton(opts.publish ? "true" : "false"));
    await waitForSelector(page, SEL.saveSummary);
    return page.locator(SEL.saveSummary);
  });
}

/** Follow a save-summary's "View post" link to the saved post, in-app.
 *
 *  Returns the permalink it navigated to. The href is read off the link the save
 *  just produced rather than reconstructed: it is the authoritative source, and
 *  re-reading is cheaper than reasoning about when a post's slug could change
 *  (#873).
 *
 *  In-app, not a `goto`: the permalink is one router push away from the summary,
 *  and a page boots once (ADR-0111, #867). */
export async function followPermalink(
  page: Page,
  summary: Locator,
): Promise<string> {
  const link = summary.locator(SEL.permalinkLink);
  const href = await link.getAttribute("href");
  expect(href, "save summary rendered no permalink link").toBeTruthy();
  await navigateInApp(page, () => link.click(), {
    url: href!,
    ready: "article.j-post",
  });
  return href!;
}

/** Move from a post's permalink to its edit page, in-app; returns the post id.
 *
 *  Neither the save summary nor the permalink exposes a post id, so it is read
 *  off the PostCard's Edit affordance — the established route. Assumes the page
 *  is already showing the post (see [`followPermalink`]). */
export async function openEditor(page: Page): Promise<string> {
  const editLink = page.locator('.j-post-acts a:has-text("Edit")');
  await editLink.waitFor();
  const postId = (await editLink.getAttribute("href"))!.match(
    /\/posts\/(\d+)\/edit/,
  )![1];
  await navigateInApp(page, () => editLink.click(), {
    url: `/posts/${postId}/edit`,
    ready: SEL.postBody,
  });
  return postId;
}

/** Assert the post currently displayed rendered in `format`.
 *
 *  Assumes the page is showing a post whose body is [`FORMAT_PROBE_BODY`] —
 *  this asserts on that body's rendering and cannot check the caller composed
 *  it, nor that the caller navigated here.
 *
 *  The assertion is two-sided on purpose: checking only that the expected
 *  element exists would pass on a page that rendered both, or neither (#873). */
export async function expectRenderedFormat(
  page: Page,
  format: PostFormatName,
): Promise<void> {
  const body = page.locator(".j-post-body");
  const other: PostFormatName = format === "org" ? "markdown" : "org";
  await expect(body.locator(FORMAT[format].tag)).toHaveText(FORMAT_PROBE_TEXT);
  await expect(body.locator(FORMAT[other].tag)).toHaveCount(0);
}
