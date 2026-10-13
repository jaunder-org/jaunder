import { createHash } from "node:crypto";

import { expect, type APIRequestContext, type Page } from "@playwright/test";
import { withTimedAction } from "./actions";
import { allowSecondBoot } from "./bootBudget";

import { fetchFeedContaining } from "./feeds";
import type { NewTracedContext } from "./fixtures";
import {
  BASE_URL,
  click,
  goto,
  login,
  subscribeTo,
  TEST_PASSWORD,
} from "./helpers";
import {
  applySeededSession,
  createSessionViaTool,
  seedUserViaTool,
  SEEDED_MEDIA_SHA256,
  parseOrdinaryFixture,
  parseOrdinarySeedManifest,
  type OrdinarySeedManifest,
  type OrdinaryFixture,
  type OrdinarySeedState,
  bindOrdinarySeedState,
  type SeedRecord,
} from "./seed";
import {
  audienceForWire,
  composePost,
  followPermalink,
  openComposerFromSidebar,
} from "./posts";
import { mintAppPassword } from "./sessions";
import { SEL } from "./selectors";
import { navigateInApp } from "./navigate";

export const OPERATION_MANIFEST = {
  version: 1,
  browserMarkdown: {
    slug: "baseline-browser-markdown",
    body: "# Baseline browser Markdown\n\nCanonical browser-created Markdown.",
  },
  browserOrg: {
    slug: "baseline-browser-org",
    body: "* Baseline browser Org\n\nCanonical browser-created Org.",
  },
  webHtml: {
    slug: "baseline-web-html",
    body: "<p>Canonical web <strong>HTML</strong> body.</p>",
  },
  scheduledMarkdown: {
    slug: "baseline-web-scheduled",
    body: "# Baseline scheduled Markdown\n\nCanonical scheduled web Post.",
  },
  subscribers: {
    slug: "baseline-web-subscribers",
    body: "# Baseline subscribers\n\nNot public.",
  },
  atomHtml: {
    slug: "baseline-atom-html",
    body: "<p>Canonical AtomPub <strong>HTML</strong> body.</p>",
  },
  atomOrg: {
    slug: "baseline-atom-org",
    body: "Canonical AtomPub Org paragraph.\n\nCreated through the Collection.",
  },
  media: {
    filename: "baseline-atompub.png",
    sha256: "ebf4f635a17d10d6eb46ba680b70142419aa3220f228001a036d311a22ee9d2a",
  },
} as const;

const PNG = Buffer.from([
  0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49,
  0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06,
  0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4, 0x89, 0x00, 0x00, 0x00, 0x0a, 0x49, 0x44,
  0x41, 0x54, 0x78, 0x9c, 0x63, 0x00, 0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0d,
  0x0a, 0x2d, 0xb4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42,
  0x60, 0x82,
]);

type VerifiedBrowserPost = {
  permalink: string;
  slug: string;
  renderedText: string;
};
type AuthorAccess = {
  username: string;
  appPassword: string;
  postsCollectionUrl: string;
};
export type BaselineState = OrdinarySeedState & {
  username: string;
  appPassword: string;
  aliceSession: SeedRecord;
  authorAccess: AuthorAccess[];
  subscriberSession: SeedRecord;
  nonSubscriberSession: SeedRecord;
  serviceUrl: string;
  postsCollectionUrl: string;
  mediaCollectionUrl: string;
  mediaMemberUrl: string;
  mediaContentUrl: string;
  atomMemberUrls: Array<{ url: string; body: string; contentType: string }>;
  browserPosts: {
    markdown: VerifiedBrowserPost;
    org: VerifiedBrowserPost;
    html: VerifiedBrowserPost;
  };
  scheduled: { permalink: string; slug: string };
  subscribers: VerifiedBrowserPost;
};
const auth = (state: BaselineState) =>
  `Basic ${Buffer.from(`${state.username}:${state.appPassword}`).toString("base64")}`;
const onServer = (url: string) => {
  const parsed = new URL(url, BASE_URL);
  return `${BASE_URL}${parsed.pathname}${parsed.search}`;
};
type SavedPost = {
  post_id: number;
  slug: string;
  published_at: string | null;
  permalink: string;
};

/** Each fixture owns exactly one response contract, independent of replies. */
export function parseCreatedPost(
  fixture: OrdinaryFixture,
  raw: unknown,
): SavedPost {
  parseOrdinaryFixture(fixture);
  const object = (value: unknown): Record<string, unknown> => {
    if (!value || typeof value !== "object" || Array.isArray(value))
      throw new Error("invalid Post create object");
    return value as Record<string, unknown>;
  };
  const envelope = object(raw);
  if (Object.keys(envelope).join() !== "Confirmed")
    throw new Error("Post creation was not confirmed");
  const confirmed = object(envelope.Confirmed);
  if (
    fixture === "modern" &&
    (!Object.keys(confirmed).every((key) =>
      ["post", "publication"].includes(key),
    ) ||
      typeof confirmed.publication !== "string" ||
      !["draft", "published", "scheduled"].includes(confirmed.publication))
  )
    throw new Error("invalid modern Post publication");
  const saved = fixture === "legacy1419" ? confirmed : object(confirmed.post);
  if (
    Object.keys(saved).length !== 4 ||
    !["post_id", "slug", "published_at", "permalink"].every(
      (key) => key in saved,
    ) ||
    !Number.isSafeInteger(saved.post_id) ||
    Number(saved.post_id) <= 0 ||
    typeof saved.slug !== "string" ||
    !/^[a-z0-9]+(?:-[a-z0-9]+)*$/.test(saved.slug) ||
    !(
      saved.published_at === null ||
      (typeof saved.published_at === "string" &&
        Number.isFinite(Date.parse(saved.published_at)))
    ) ||
    typeof saved.permalink !== "string" ||
    !saved.permalink.startsWith("/") ||
    saved.permalink.startsWith("//")
  )
    throw new Error("invalid selected SavedPost");
  return saved as SavedPost;
}

export function baselineAudience(
  fixture: OrdinaryFixture,
  audience: "public" | "subscribers" | "private",
) {
  parseOrdinaryFixture(fixture);
  return fixture === "legacy1419"
    ? { base: audience, named: [] }
    : audienceForWire(audience);
}

/** Only the host-selected fixture can bind retained state. */
export function validateBaselineState(
  state: BaselineState,
  fixture: OrdinaryFixture,
): void {
  parseOrdinaryFixture(fixture);
  if (state.fixture !== fixture)
    throw new Error("persisted ordinary fixture changed");
  parseOrdinarySeedManifest(fixture, state.seededManifest);
}

type BaselinePostInputs = {
  body: string;
  format?: "markdown" | "org" | "html";
  audience?: "public" | "subscribers" | "private";
  slug?: string;
  publishAt?: string;
};

export function baselinePostRequest(
  fixture: OrdinaryFixture,
  opts: BaselinePostInputs,
) {
  parseOrdinaryFixture(fixture);
  return {
    post: {
      body: opts.body,
      format: opts.format ?? "markdown",
      slug_override: opts.slug ?? null,
      publish: true,
      ...(opts.publishAt ? { publish_at: opts.publishAt } : {}),
      ...(opts.audience
        ? { audience: baselineAudience(fixture, opts.audience) }
        : {}),
    },
  };
}

async function createLegacyPostViaApi(
  page: Page,
  fixture: OrdinaryFixture,
  opts: BaselinePostInputs,
): Promise<SavedPost> {
  const response = await withTimedAction(page, "api.posts.create", () =>
    page.request.post(`${BASE_URL}/api/posts/create`, {
      data: baselinePostRequest(fixture, opts),
    }),
  );
  expect(
    response.ok(),
    `posts::create failed (${response.status()}): ${await response.text()}`,
  ).toBeTruthy();
  return parseCreatedPost(fixture, await response.json());
}
type SeededPost = OrdinarySeedManifest["posts"][number];

export const HISTORICAL_COMPOSER = {
  body: SEL.postBody,
  slug: SEL.postSlug,
  audience: "#audience-base",
  format: SEL.formatButton("Org"),
  publish: SEL.publishButton("true"),
  summary: SEL.saveSummary,
} as const;

async function openBaselineComposer(
  page: Page,
  fixture: OrdinaryFixture,
): Promise<void> {
  if (fixture === "modern") return openComposerFromSidebar(page);
  await navigateInApp(page, () => click(page, '.j-nav a[href="/posts/new"]'), {
    url: "/posts/new",
    ready: HISTORICAL_COMPOSER.audience,
  });
}

export async function composeBaselinePost(
  page: Page,
  fixture: OrdinaryFixture,
  opts: Parameters<typeof composePost>[1],
) {
  parseOrdinaryFixture(fixture);
  if (fixture === "modern") return composePost(page, opts);
  return withTimedAction(page, "flow.compose_post", async () => {
    await page.locator(HISTORICAL_COMPOSER.body).waitFor();
    await page.fill(HISTORICAL_COMPOSER.body, opts.body);
    if (opts.format)
      await click(
        page,
        SEL.formatButton(opts.format === "org" ? "Org" : "Markdown"),
      );
    if (opts.slug) await page.fill(HISTORICAL_COMPOSER.slug, opts.slug);
    if (opts.audience)
      await page.selectOption(HISTORICAL_COMPOSER.audience, opts.audience);
    await click(page, SEL.publishButton(opts.publish ? "true" : "false"));
    await page.locator(HISTORICAL_COMPOSER.summary).waitFor();
    return page.locator(HISTORICAL_COMPOSER.summary);
  });
}

type FormattedTitleExpectation = {
  slug: string;
  visibleText: string;
  tags: string[];
};

const FORMATTED_TITLE_EXPECTATIONS: readonly FormattedTitleExpectation[] = [
  {
    slug: "formatted-markdown-title",
    visibleText: "Markdown emphasis strong deleted code link label",
    tags: ["em", "strong", "del", "code"],
  },
  {
    slug: "formatted-org-title",
    visibleText: "Org emphasis strong underline Org label",
    tags: ["i", "b", "u"],
  },
  {
    slug: "formatted-html-title",
    visibleText: "HTML emphasis HTML label",
    tags: ["em"],
  },
];

function seededPostPath(post: SeededPost): string {
  if (post.publishedAt === null) {
    throw new Error(`seeded Post ${post.slug} has no permalink`);
  }
  const date = post.publishedAt.slice(0, 10).replace(/-/g, "/");
  return `/~${post.author}/${date}/${post.slug}`;
}

async function expectSafeRenderedTitle(
  article: import("@playwright/test").Locator,
  expected: FormattedTitleExpectation,
): Promise<void> {
  const title = article.locator(".j-post-title");
  await expect(title).toHaveText(expected.visibleText);
  for (const tag of expected.tags) {
    await expect(title.locator(tag)).toHaveCount(1);
  }
  // The title's existing permalink is the only anchor: source links flatten to
  // their labels and active/embedded source content never reaches the browser.
  await expect(title.locator("a")).toHaveCount(1);
  await expect(title.locator("script, img, iframe, object, embed")).toHaveCount(
    0,
  );
}

function escapeXmlText(value: string): string {
  let escaped = value;
  for (const [character, entity] of [
    ["&", "&amp;"],
    ["<", "&lt;"],
    [">", "&gt;"],
  ] as const) {
    escaped = escaped.split(character).join(entity);
  }
  return escaped;
}

function atomText(entry: string, element: string): string {
  const text = entry.match(
    new RegExp(`<${element}(?:\\s[^>]*)?>([\\s\\S]*?)</${element}>`, "i"),
  )?.[1];
  expect(text, `Atom entry has ${element}`).toBeDefined();
  return text!
    .split("&lt;")
    .join("<")
    .split("&gt;")
    .join(">")
    .split("&quot;")
    .join('"')
    .split("&apos;")
    .join("'")
    .split("&amp;")
    .join("&");
}

function atomContent(entry: string): string {
  return atomText(entry, "content");
}

function atomPostId(entry: string): string {
  const id = atomText(entry, "id");
  const postId = new URL(id).pathname.match(/\/posts\/(\d+)$/)?.[1];
  expect(postId, "Atom entry id is a post member URL").toBeTruthy();
  return postId!;
}

function atomContentType(entry: string): string {
  const type = entry.match(/<content\b(?=[^>]*\btype="([^"]+)")[^>]*>/i)?.[1];
  expect(type, "Atom entry content has a type").toBeTruthy();
  return type!;
}

function atomNextUrl(document: string): string | undefined {
  const href = document.match(
    /<link\b(?=[^>]*\brel="next")(?=[^>]*\bhref="([^"]+)")[^>]*>/i,
  )?.[1];
  return href === undefined
    ? undefined
    : onServer(href.split("&amp;").join("&"));
}

async function atomCollectionEntries(
  request: APIRequestContext,
  collectionUrl: string,
  authorization: string,
): Promise<string[]> {
  const entries: string[] = [];
  let pageUrl: string | undefined = `${collectionUrl}?limit=50`;
  while (pageUrl !== undefined) {
    const response = await request.get(pageUrl, { headers: { authorization } });
    expect(response.status()).toBe(200);
    const document = await response.text();
    entries.push(
      ...Array.from(
        document.matchAll(/<entry\b[\s\S]*?<\/entry>/gi),
        (match) => match[0],
      ),
    );
    pageUrl = atomNextUrl(document);
  }
  return entries;
}

export async function verifySeededManifest(
  state: BaselineState,
  tracedContext: NewTracedContext,
): Promise<void> {
  const requestContext = await isolatedRequest(tracedContext);
  try {
    for (const access of state.authorAccess) {
      const entries = await atomCollectionEntries(
        requestContext.request,
        access.postsCollectionUrl,
        `Basic ${Buffer.from(`${access.username}:${access.appPassword}`).toString("base64")}`,
      );
      for (const post of state.seededManifest.posts.filter(
        (candidate) => candidate.author === access.username,
      )) {
        const entry = entries.find((candidate) =>
          new RegExp(`<j:slug(?:\\s[^>]*)?>${post.slug}</j:slug>`, "i").test(
            candidate,
          ),
        );
        expect(
          entry,
          `${post.author}/${post.slug} is present in AtomPub`,
        ).toBeTruthy();
        expect(atomText(entry!, "title")).toBe(post.title);
        expect(atomContent(entry!)).toBe(post.body);
        expect(atomContentType(entry!)).toBe(
          post.format === "markdown"
            ? "text/markdown"
            : post.format === "org"
              ? "text/org"
              : "html",
        );
        const audience = await requestContext.request.post(
          `${BASE_URL}/api/posts/get_audience_selection`,
          {
            headers: {
              authorization: `Basic ${Buffer.from(`${access.username}:${access.appPassword}`).toString("base64")}`,
            },
            form: { post_id: atomPostId(entry!) },
          },
        );
        expect(audience.status()).toBe(200);
        expect(await audience.json()).toEqual(
          baselineAudience(state.fixture, post.visibility),
        );
        if (post.publishedAt === null) {
          expect(entry!).toMatch(/<app:draft>yes<\/app:draft>/i);
        } else {
          const published = entry!.match(
            /<published>([^<]+)<\/published>/i,
          )?.[1];
          expect(published).toBeTruthy();
          expect(new Date(published!).toISOString()).toBe(
            new Date(post.publishedAt).toISOString(),
          );
        }
      }
    }
  } finally {
    await requestContext.dispose();
  }
}
const atomEntry = (title: string, body: string, type: string) =>
  `<?xml version="1.0"?><entry xmlns="http://www.w3.org/2005/Atom"><title>${title}</title><content type="${type}">${escapeXmlText(body)}</content></entry>`;

type IsolatedRequest = {
  request: APIRequestContext;
  dispose: () => Promise<void>;
};

async function isolatedRequest(
  tracedContext: NewTracedContext,
): Promise<IsolatedRequest> {
  const context = await tracedContext();
  return { request: context.request, dispose: () => context.close() };
}

function xmlAttribute(
  document: string,
  element: string,
  attribute: string,
): string {
  const value = document.match(
    new RegExp(`<${element}(?:\\s[^>]*)?\\s${attribute}="([^"]+)"`, "i"),
  )?.[1];
  expect(value, `AtomPub ${element} advertises ${attribute}`).toBeTruthy();
  return onServer(value!);
}

async function discoverAtomPub(
  request: APIRequestContext,
  username: string,
  authorization: string,
): Promise<
  Pick<
    BaselineState,
    "serviceUrl" | "postsCollectionUrl" | "mediaCollectionUrl"
  >
> {
  const profile = await request.get(`${BASE_URL}/~${username}`);
  expect(profile.status()).toBe(200);
  const rsdHref = (await profile.text()).match(
    /<link\b(?=[^>]*\brel="EditURI")(?=[^>]*\bhref="([^"]+)")[^>]*>/i,
  )?.[1];
  expect(rsdHref).toBeTruthy();
  const rsd = await request.get(onServer(rsdHref!));
  expect(rsd.status()).toBe(200);
  const serviceHref = (await rsd.text()).match(
    /<api\b(?=[^>]*\bapiLink="([^"]+)")[^>]*>/i,
  )?.[1];
  expect(serviceHref).toBeTruthy();
  const serviceUrl = onServer(serviceHref!);
  const service = await request.get(serviceUrl, { headers: { authorization } });
  expect(service.status()).toBe(200);
  const collections = [
    ...(await service.text()).matchAll(
      /<(?:[A-Za-z][\w.-]*:)?collection\b(?=[^>]*\bhref="([^"]+)")[^>]*>/gi,
    ),
  ].map((match) => onServer(match[1]!));
  const postsCollectionUrl = collections.find((url) => url.endsWith("/posts"));
  const mediaCollectionUrl = collections.find((url) => url.endsWith("/media"));
  expect(postsCollectionUrl).toBeTruthy();
  expect(mediaCollectionUrl).toBeTruthy();
  return {
    serviceUrl,
    postsCollectionUrl: postsCollectionUrl!,
    mediaCollectionUrl: mediaCollectionUrl!,
  };
}

/** Ordinary controls follow the source-selected fixture, never the DOM. */
export async function mintBaselineAppPassword(
  page: Page,
  fixture: OrdinaryFixture,
  label: string,
): Promise<string> {
  parseOrdinaryFixture(fixture);
  if (fixture === "modern") return mintAppPassword(page, label);
  if (new URL(page.url()).pathname !== "/sessions") {
    await goto(page, "/sessions");
  }
  await page.fill('input[name="label"]', label);
  await click(page, '.j-app-passwords button:has-text("Create app password")');
  const token = page.locator(".j-app-password-token code");
  await token.waitFor({ state: "visible", timeout: 15_000 });
  return ((await token.textContent()) ?? "").trim();
}

async function createAuthorAccess(
  tracedContext: NewTracedContext,
  username: string,
  fixture: OrdinaryFixture,
): Promise<AuthorAccess> {
  const session = await createSessionViaTool(
    username,
    "Production baseline manifest read",
  );
  const context = await tracedContext();
  try {
    await applySeededSession(context, session);
    const page = await context.newPage();
    try {
      await goto(page, "/sessions");
      const appPassword = await mintBaselineAppPassword(
        page,
        fixture,
        `Production baseline manifest ${username}`,
      );
      const authorization = `Basic ${Buffer.from(`${username}:${appPassword}`).toString("base64")}`;
      const atom = await isolatedRequest(tracedContext);
      try {
        const discovered = await discoverAtomPub(
          atom.request,
          username,
          authorization,
        );
        return {
          username,
          appPassword,
          postsCollectionUrl: discovered.postsCollectionUrl,
        };
      } finally {
        await atom.dispose();
      }
    } finally {
      await page.close();
    }
  } finally {
    await context.close();
  }
}

export async function createProductionBaseline(
  page: Page,
  tracedContext: NewTracedContext,
  seededManifest: OrdinarySeedManifest,
  fixture: OrdinaryFixture = "modern",
): Promise<BaselineState> {
  parseOrdinarySeedManifest(fixture, seededManifest);
  const aliceSession = await createSessionViaTool(
    "alice",
    "Production baseline read-only",
  );
  const operationUser = await seedUserViaTool("baseline-author", TEST_PASSWORD);
  const subscriberSession = await seedUserViaTool(
    "baseline-subscriber",
    TEST_PASSWORD,
  );
  const nonSubscriberSession = await seedUserViaTool(
    "baseline-non-subscriber",
    TEST_PASSWORD,
  );
  await login(page, operationUser.username, TEST_PASSWORD);
  const username = operationUser.username;
  allowSecondBoot(
    page,
    "the real login cold render is part of the baseline, and App Password management has no in-app navigation control",
  );
  await goto(page, "/sessions");
  const appPassword = await mintBaselineAppPassword(
    page,
    fixture,
    "Production baseline primary",
  );
  await openBaselineComposer(page, fixture);
  const markdown = await composeBaselinePost(page, fixture, {
    ...OPERATION_MANIFEST.browserMarkdown,
    publish: true,
  });
  const browserMarkdownPermalink = await followPermalink(page, markdown);
  await openBaselineComposer(page, fixture);
  const org = await composeBaselinePost(page, fixture, {
    ...OPERATION_MANIFEST.browserOrg,
    format: "org",
    publish: true,
  });
  const browserOrgPermalink = await followPermalink(page, org);
  await openBaselineComposer(page, fixture);
  const subscribers = await composeBaselinePost(page, fixture, {
    ...OPERATION_MANIFEST.subscribers,
    audience: "subscribers",
    publish: true,
  });
  const subscriberPermalink = await followPermalink(page, subscribers);
  const webHtml = await createLegacyPostViaApi(page, fixture, {
    ...OPERATION_MANIFEST.webHtml,
    format: "html",
  });
  const scheduled = await createLegacyPostViaApi(page, fixture, {
    ...OPERATION_MANIFEST.scheduledMarkdown,
    publishAt: "2035-01-01T00:00:00Z",
  });
  expect(scheduled.permalink).toContain(
    OPERATION_MANIFEST.scheduledMarkdown.slug,
  );
  const subscriberContext = await tracedContext();
  try {
    await applySeededSession(subscriberContext, subscriberSession);
    const subscriberPage = await subscriberContext.newPage();
    try {
      await subscribeTo(subscriberPage, username);
    } finally {
      await subscriberPage.close();
    }
  } finally {
    await subscriberContext.close();
  }
  const authors = seededManifest.posts
    .map((post) => post.author)
    .filter((author, index, all) => all.indexOf(author) === index);
  const authorAccess = await Promise.all(
    authors.map((author) => createAuthorAccess(tracedContext, author, fixture)),
  );
  const state: BaselineState = {
    ...bindOrdinarySeedState(fixture, seededManifest),
    username,
    appPassword,
    aliceSession,
    authorAccess,
    subscriberSession,
    nonSubscriberSession,
    serviceUrl: "",
    postsCollectionUrl: "",
    mediaCollectionUrl: "",
    mediaMemberUrl: "",
    mediaContentUrl: "",
    atomMemberUrls: [],
    browserPosts: {
      markdown: {
        permalink: browserMarkdownPermalink,
        slug: OPERATION_MANIFEST.browserMarkdown.slug,
        renderedText:
          "Baseline browser Markdown Canonical browser-created Markdown.",
      },
      org: {
        permalink: browserOrgPermalink,
        slug: OPERATION_MANIFEST.browserOrg.slug,
        renderedText: "Canonical browser-created Org.",
      },
      html: {
        permalink: webHtml.permalink,
        slug: OPERATION_MANIFEST.webHtml.slug,
        renderedText: "Canonical web HTML body.",
      },
    },
    scheduled: {
      permalink: scheduled.permalink,
      slug: OPERATION_MANIFEST.scheduledMarkdown.slug,
    },
    subscribers: {
      permalink: subscriberPermalink,
      slug: OPERATION_MANIFEST.subscribers.slug,
      renderedText: "Baseline subscribers Not public.",
    },
  };
  const authorization = auth(state);
  const atomContext = await isolatedRequest(tracedContext);
  const atom = atomContext.request;
  try {
    Object.assign(state, await discoverAtomPub(atom, username, authorization));
    for (const [title, record, type] of [
      ["Baseline Atom HTML", OPERATION_MANIFEST.atomHtml, "html"],
      ["Baseline Atom Org", OPERATION_MANIFEST.atomOrg, "text/org"],
    ] as const) {
      const created = await atom.post(state.postsCollectionUrl, {
        headers: { authorization, "content-type": "application/atom+xml" },
        data: atomEntry(title, record.body, type),
      });
      expect(created.status()).toBe(201);
      const location = created.headers()["location"];
      expect(location).toBeTruthy();
      const memberUrl = onServer(location!);
      state.atomMemberUrls.push({
        url: memberUrl,
        body: record.body,
        contentType: type,
      });
      const member = await atom.get(memberUrl, { headers: { authorization } });
      expect(member.status()).toBe(200);
      const updated = await atom.put(memberUrl, {
        headers: {
          authorization,
          "content-type": "application/atom+xml",
          "if-match": member.headers()["etag"]!,
        },
        data: await member.text(),
      });
      expect(updated.status()).toBe(200);
    }
    expect(createHash("sha256").update(new Uint8Array(PNG)).digest("hex")).toBe(
      OPERATION_MANIFEST.media.sha256,
    );
    const media = await atom.post(state.mediaCollectionUrl, {
      headers: {
        authorization,
        "content-type": "image/png",
        slug: OPERATION_MANIFEST.media.filename,
      },
      data: PNG,
    });
    expect(media.status()).toBe(201);
    state.mediaMemberUrl = onServer(media.headers()["location"]!);
    const mediaContentUrl = (await media.text()).match(
      /<content(?:\s[^>]*)?\ssrc="([^"]+)"/,
    )?.[1];
    expect(mediaContentUrl).toBeTruthy();
    state.mediaContentUrl = onServer(mediaContentUrl!);
    return state;
  } finally {
    await atomContext.dispose();
  }
}

/** Verifies pre-existing records with the original browser session and App Password only. */
export async function verifyProductionBaseline(
  page: Page,
  state: BaselineState,
  tracedContext: NewTracedContext,
): Promise<void> {
  validateBaselineState(state, state.fixture);
  const authorization = auth(state);
  await goto(page, "/sessions");
  await expect(
    page.locator("li", { hasText: "Production baseline primary" }),
  ).toBeVisible();
  for (const post of [
    state.browserPosts.markdown,
    state.browserPosts.org,
    state.browserPosts.html,
    state.subscribers,
  ]) {
    const probe = await page.context().newPage();
    try {
      await goto(probe, post.permalink);
      expect(new URL(probe.url()).pathname.endsWith(`/${post.slug}`)).toBe(
        true,
      );
      await expect(probe.locator(".j-post-body")).toHaveText(post.renderedText);
    } finally {
      await probe.close();
    }
  }
  await verifySeededManifest(state, tracedContext);

  if (state.fixture === "modern") {
    const formattedPosts = FORMATTED_TITLE_EXPECTATIONS.map((expected) => {
      const post = state.seededManifest.posts.find(
        (candidate) => candidate.slug === expected.slug,
      );
      expect(post, `sandbox manifest contains ${expected.slug}`).toBeDefined();
      expect(post!.visibility, `${expected.slug} is public`).toBe("public");
      expect(post!.publishedAt, `${expected.slug} is published`).not.toBeNull();
      return { expected, post: post! };
    });
    const formattedContext = await tracedContext();
    try {
      const timeline = await formattedContext.newPage();
      try {
        await goto(timeline, "/~alice");
        for (const { expected, post } of formattedPosts) {
          const article = timeline
            .locator("article.j-post")
            .filter({ hasText: expected.visibleText });
          await expect(article).toHaveCount(1);
          await expectSafeRenderedTitle(article, expected);
          await expect(article.locator(".j-post-title a")).toHaveAttribute(
            "href",
            new RegExp(`/${post.slug}$`),
          );
        }
      } finally {
        await timeline.close();
      }

      for (const { expected, post } of formattedPosts) {
        const permalink = await formattedContext.newPage();
        try {
          await goto(permalink, seededPostPath(post));
          expect(new URL(permalink.url()).pathname).toBe(seededPostPath(post));
          await expectSafeRenderedTitle(
            permalink.locator("article.j-post"),
            expected,
          );
        } finally {
          await permalink.close();
        }
      }
    } finally {
      await formattedContext.close();
    }
  }

  for (const viewer of [
    { session: state.subscriberSession, canView: true },
    { session: state.nonSubscriberSession, canView: false },
  ]) {
    const viewerContext = await tracedContext();
    try {
      await applySeededSession(viewerContext, viewer.session);
      const viewerPage = await viewerContext.newPage();
      try {
        await goto(viewerPage, state.subscribers.permalink);
        const body = viewerPage.locator(".j-post-body");
        if (viewer.canView) {
          await expect(body).toHaveText(state.subscribers.renderedText);
        } else {
          await expect(viewerPage.locator(SEL.error)).toContainText(
            "Post not found",
          );
          await expect(viewerPage.locator("body")).not.toContainText(
            state.subscribers.renderedText,
          );
        }
      } finally {
        await viewerPage.close();
      }
    } finally {
      await viewerContext.close();
    }
  }
  const scheduledPage = await page.context().newPage();
  try {
    await goto(scheduledPage, "/scheduled");
    const scheduledRow = scheduledPage.locator('[data-test="scheduled-row"]', {
      hasText: "Baseline scheduled Markdown",
    });
    await expect(scheduledRow).toBeVisible();
    await expect(scheduledRow.locator(".j-badge-scheduled")).toContainText(
      "Scheduled for",
    );
  } finally {
    await scheduledPage.close();
  }
  const atomContext = await isolatedRequest(tracedContext);
  try {
    const entries = await atomCollectionEntries(
      atomContext.request,
      state.postsCollectionUrl,
      authorization,
    );
    for (const [record, contentType, expectedBody] of [
      [
        OPERATION_MANIFEST.browserMarkdown,
        "text/markdown",
        `${OPERATION_MANIFEST.browserMarkdown.body}\n`,
      ],
      [
        OPERATION_MANIFEST.browserOrg,
        "text/org",
        "Canonical browser-created Org.\n",
      ],
      [OPERATION_MANIFEST.webHtml, "html", OPERATION_MANIFEST.webHtml.body],
      [
        OPERATION_MANIFEST.scheduledMarkdown,
        "text/markdown",
        `${OPERATION_MANIFEST.scheduledMarkdown.body}\n`,
      ],
      [
        OPERATION_MANIFEST.subscribers,
        "text/markdown",
        `${OPERATION_MANIFEST.subscribers.body}\n`,
      ],
      [OPERATION_MANIFEST.atomHtml, "html", OPERATION_MANIFEST.atomHtml.body],
      [
        OPERATION_MANIFEST.atomOrg,
        "text/org",
        `${OPERATION_MANIFEST.atomOrg.body}\n`,
      ],
    ] as const) {
      const entry = entries.find((candidate) =>
        new RegExp(`<j:slug(?:\\s[^>]*)?>${record.slug}</j:slug>`, "i").test(
          candidate,
        ),
      );
      expect(entry, `${record.slug} is present in AtomPub`).toBeTruthy();
      expect(atomContent(entry!)).toBe(expectedBody);
      expect(entry!).toContain(`type="${contentType}"`);
    }
    for (const member of state.atomMemberUrls) {
      const response = await atomContext.request.get(member.url, {
        headers: { authorization },
      });
      expect(response.status()).toBe(200);
      expect(response.headers()["content-type"]).toContain(
        "application/atom+xml",
      );
      const entry = await response.text();
      expect(entry).toContain(escapeXmlText(member.body));
      expect(entry).toContain(`type="${member.contentType}"`);
    }
    expect(
      (
        await atomContext.request.get(state.mediaMemberUrl, {
          headers: { authorization },
        })
      ).status(),
    ).toBe(200);
    const mediaContent = await atomContext.request.get(state.mediaContentUrl, {
      headers: { authorization },
    });
    expect(mediaContent.status()).toBe(200);
    expect(
      createHash("sha256")
        .update(new Uint8Array(await mediaContent.body()))
        .digest("hex"),
    ).toBe(OPERATION_MANIFEST.media.sha256);
  } finally {
    await atomContext.dispose();
  }
  const anonymousBrowser = await tracedContext();
  try {
    const now = new Date();
    for (const hidden of [
      {
        permalink: state.subscribers.permalink,
        marker: state.subscribers.renderedText,
      },
      {
        permalink: state.scheduled.permalink,
        marker: "Canonical scheduled web Post.",
      },
      ...state.seededManifest.posts
        .filter(
          (post) =>
            post.publishedAt !== null &&
            (post.visibility !== "public" || new Date(post.publishedAt) > now),
        )
        .map((post) => ({
          permalink: seededPostPath(post),
          marker: post.title,
        })),
    ]) {
      const anonymousPage = await anonymousBrowser.newPage();
      try {
        await goto(anonymousPage, hidden.permalink);
        await expect(anonymousPage.locator(SEL.error)).toContainText(
          "Post not found",
        );
        await expect(anonymousPage.locator("body")).not.toContainText(
          hidden.marker,
        );
      } finally {
        await anonymousPage.close();
      }
    }
  } finally {
    await anonymousBrowser.close();
  }
  const anonymousContext = await isolatedRequest(tracedContext);
  try {
    const seededMediaRecords =
      state.fixture === "modern"
        ? state.seededManifest.media
        : [state.seededManifest.media!];
    for (const seededMedia of seededMediaRecords) {
      const expectedSha256 = SEEDED_MEDIA_SHA256[seededMedia.filename];
      expect(expectedSha256, seededMedia.filename).toBeDefined();
      expect(seededMedia.sha256).toBe(expectedSha256);
      const seededContent = await anonymousContext.request.get(
        `${BASE_URL}${seededMedia.contentUrl}`,
      );
      expect(seededContent.status(), seededMedia.contentUrl).toBe(200);
      expect(seededContent.headers()["content-type"]).toBe(
        seededMedia.filename.endsWith(".svg") ? "image/svg+xml" : "text/plain",
      );
      const bytes = new Uint8Array(await seededContent.body());
      expect(bytes).toHaveLength(seededMedia.sizeBytes);
      expect(createHash("sha256").update(bytes).digest("hex")).toBe(
        expectedSha256,
      );
    }
    const now = new Date();
    const atomFeed = await fetchFeedContaining(
      anonymousContext.request,
      `${BASE_URL}/~${state.username}/feed.atom`,
      OPERATION_MANIFEST.webHtml.slug,
    );
    expect(atomFeed.body).toContain(
      "&lt;p&gt;Canonical web &lt;strong&gt;HTML&lt;/strong&gt; body.&lt;/p&gt;",
    );
    const rssFeed = await fetchFeedContaining(
      anonymousContext.request,
      `${BASE_URL}/~${state.username}/feed.rss`,
      OPERATION_MANIFEST.webHtml.slug,
    );
    expect(rssFeed.body).toContain(
      `<![CDATA[${OPERATION_MANIFEST.webHtml.body}]]>`,
    );
    const jsonFeed = await fetchFeedContaining(
      anonymousContext.request,
      `${BASE_URL}/~${state.username}/feed.json`,
      OPERATION_MANIFEST.webHtml.slug,
    );
    expect(JSON.parse(jsonFeed.body).items).toContainEqual(
      expect.objectContaining({
        content_html: OPERATION_MANIFEST.webHtml.body,
      }),
    );
    const authors = state.seededManifest.posts
      .map((post) => post.author)
      .filter((author, index, all) => all.indexOf(author) === index);
    for (const author of authors) {
      const publishedPublic = state.seededManifest.posts.filter(
        (post) =>
          post.author === author &&
          post.visibility === "public" &&
          post.publishedAt !== null &&
          new Date(post.publishedAt) <= now,
      );
      expect(publishedPublic.length).toBeGreaterThan(0);
      const hidden = state.seededManifest.posts.filter(
        (post) =>
          post.author === author &&
          (post.visibility !== "public" ||
            post.publishedAt === null ||
            new Date(post.publishedAt) > now),
      );
      for (const format of ["atom", "rss", "json"] as const) {
        const feed = await fetchFeedContaining(
          anonymousContext.request,
          `${BASE_URL}/~${author}/feed.${format}`,
          publishedPublic[0]!.slug,
        );
        for (const post of publishedPublic) {
          expect(feed.body).toContain(post.slug);
        }
        for (const post of hidden) {
          expect(feed.body).not.toContain(post.slug);
        }
      }
    }
  } finally {
    await anonymousContext.dispose();
  }
}

/** Mutation check deliberately separate from read-only verification for every recovered state. */
export async function verifyFreshAppPasswordLifecycle(
  page: Page,
  state: BaselineState,
  tracedContext: NewTracedContext,
): Promise<string> {
  const fresh = await mintBaselineAppPassword(
    page,
    state.fixture,
    "Production baseline revocable",
  );
  const authorization = `Basic ${Buffer.from(`${state.username}:${fresh}`).toString("base64")}`;
  const atomContext = await isolatedRequest(tracedContext);
  try {
    expect(
      (
        await atomContext.request.get(`${BASE_URL}/atompub/service`, {
          headers: { authorization },
        })
      ).status(),
    ).toBe(200);
  } finally {
    await atomContext.dispose();
  }
  const revocableRow = page.locator("li", {
    hasText: "Production baseline revocable",
  });
  await click(
    page,
    'li:has-text("Production baseline revocable") button:has-text("Revoke")',
  );
  await expect(revocableRow).toHaveCount(0);
  const revokedContext = await isolatedRequest(tracedContext);
  try {
    expect(
      (
        await revokedContext.request.get(`${BASE_URL}/atompub/service`, {
          headers: { authorization },
        })
      ).status(),
    ).toBe(401);
  } finally {
    await revokedContext.dispose();
  }
  return fresh;
}
