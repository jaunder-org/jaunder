// Private consumer of xtask's fixed styling qualification protocol.
import { createHash, randomUUID } from "node:crypto";
import {
  expect,
  type BrowserContext,
  type CDPSession,
  type Page,
} from "@playwright/test";
import type { NewTracedContext } from "./fixtures";
import { allowSecondBoot } from "./bootBudget";
import { BASE_URL, goto, login, TEST_PASSWORD } from "./helpers";
import { composePost, followPermalink, openComposerFromSidebar } from "./posts";
import { seedSandboxProfileViaTool, seedUserViaTool } from "./seed";
import { uploadMedia } from "./media-helpers";

type Surface = "local" | "author-permalink" | "home";
type Asset = {
  role: string;
  url: string;
  digest: string;
  revision: string | null;
  sha256: string;
  mime: string;
  bytes: number;
  etag: string;
};
export type StylingRequest = {
  sequence: number;
  phase: "create" | "a-warm" | "app-b" | "rollback-a" | "theme-b" | "restored";
  fixture: "a" | "b-app" | "b-theme";
  transition: "application" | "studio";
  backend: "sqlite" | "postgres";
  expected_surfaces: Array<{
    surface: Surface;
    application: Asset;
    public_presentation: { stylesheet: Asset; revision: string } | null;
    topbar_border_color: string | null;
    studio_accent: string | null;
  }>;
  requested_cache_urls: string[];
  requested_http_assets: Asset[];
  seed_process?: string;
};
type CacheObservation = {
  url: string;
  evidence: "request-served-from-cache" | "request-served-from-disk-cache";
  document_path: string;
};

// Read-only CDP observation: enabling Network events never clears, disables or
// emulates the browser cache. Correlation keeps actual request/document identity.
class CacheObserver {
  private readonly requests = new Map<
    string,
    { url: string; document: string }
  >();
  private readonly hits = new Map<string, CacheObservation>();
  private constructor(private readonly session: CDPSession) {
    session.on("Network.requestWillBeSent", (event) => {
      this.requests.set(event.requestId, {
        url: event.request.url,
        document: event.documentURL,
      });
    });
    session.on("Network.requestServedFromCache", (event) => {
      this.record(event.requestId, "request-served-from-cache");
    });
    session.on("Network.responseReceived", (event) => {
      if (event.response.fromDiskCache) {
        this.record(event.requestId, "request-served-from-disk-cache");
      }
    });
  }
  static async attach(page: Page): Promise<CacheObserver> {
    const session = await page.context().newCDPSession(page);
    const observer = new CacheObserver(session);
    await session.send("Network.enable");
    return observer;
  }
  private record(id: string, evidence: CacheObservation["evidence"]): void {
    const request = this.requests.get(id);
    if (!request)
      throw new Error("cache event has no correlated browser request");
    const url = new URL(request.url);
    if (url.origin !== new URL(BASE_URL).origin) return;
    this.hits.set(url.pathname, {
      url: url.pathname,
      evidence,
      document_path: new URL(request.document).pathname,
    });
  }
  reset(): void {
    this.hits.clear();
  }
  get(url: string): CacheObservation | undefined {
    return this.hits.get(url);
  }
  async close(): Promise<void> {
    await this.session.detach();
  }
}

export class StylingSession {
  private readonly anonymousId = randomUUID();
  private readonly authenticatedId = randomUUID();
  private permalink: string | undefined;
  private created = false;
  private constructor(
    private readonly anonymous: BrowserContext,
    private readonly authenticated: BrowserContext,
    private readonly publicPage: Page,
    private readonly homePage: Page,
    private readonly publicCache: CacheObserver,
    private readonly homeCache: CacheObserver,
    private readonly registerCanaries: (
      values: readonly string[],
    ) => Promise<void>,
  ) {}
  static async start(
    tracedContext: NewTracedContext,
    registerCanaries: (values: readonly string[]) => Promise<void>,
  ): Promise<StylingSession> {
    const anonymous = await tracedContext();
    const authenticated = await tracedContext();
    try {
      const publicPage = await anonymous.newPage();
      const homePage = await authenticated.newPage();
      return new StylingSession(
        anonymous,
        authenticated,
        publicPage,
        homePage,
        await CacheObserver.attach(publicPage),
        await CacheObserver.attach(homePage),
        registerCanaries,
      );
    } catch (error) {
      await Promise.allSettled([anonymous.close(), authenticated.close()]);
      throw error;
    }
  }
  private async create(request: StylingRequest): Promise<void> {
    if (this.created || !request.seed_process)
      throw new Error("invalid styling Create phase");
    process.env.JAUNDER_E2E_SEED_PROCESS = request.seed_process;
    // The demo baseline intentionally renders a raw text Media fixture as an
    // image. Styling qualification owns a decodable image, not that raw-byte
    // continuity scenario; ordinary #1419 demo coverage stays independent.
    await seedSandboxProfileViaTool("standard");
    const author = await seedUserViaTool("styling-author", TEST_PASSWORD);
    await login(this.homePage, author.username, TEST_PASSWORD);
    const image = await uploadMedia(
      this.homePage,
      "styling-qualification.svg",
      Buffer.from(
        '<svg xmlns="http://www.w3.org/2000/svg" width="96" height="64" viewBox="0 0 96 64"><rect width="96" height="64" fill="#386d89"/></svg>',
      ),
      "image/svg+xml",
    );
    await openComposerFromSidebar(this.homePage);
    const post = await composePost(this.homePage, {
      body: `# Styling qualification\n\nRetained browser release continuity.\n\n![Styling qualification image](${image.url})`,
      slug: "styling-qualification",
      publish: true,
    });
    this.permalink = await followPermalink(this.homePage, post);
    const cookies = await this.authenticated.cookies();
    const marker = await this.homePage.evaluate(
      () => localStorage.getItem("jaunder_auth") ?? "",
    );
    await this.registerCanaries([
      TEST_PASSWORD,
      ...cookies.map((cookie) => cookie.value),
      marker,
    ]);
    this.created = true;
  }
  async run(request: StylingRequest) {
    if (request.phase === "create") await this.create(request);
    if (!this.created || !this.permalink)
      throw new Error("styling observation precedes Create");
    this.publicCache.reset();
    this.homeCache.reset();
    const surfaces = [];
    for (const expected of request.expected_surfaces) {
      const page =
        expected.surface === "home" ? this.homePage : this.publicPage;
      const path =
        expected.surface === "home"
          ? "/app"
          : expected.surface === "local"
            ? "/"
            : this.permalink;
      if (page.url() !== "about:blank") {
        allowSecondBoot(
          page,
          "release qualification intentionally observes a fresh document entry after each immutable-package cutover",
        );
      }
      await goto(page, path);
      await expect(page.locator(".j-root")).toBeVisible();
      await expect(page.locator(".j-topbar")).toBeVisible();
      await expect(
        page.locator(
          '[data-jaunder-part="post-body"] img[alt="Styling qualification image"]',
        ),
      ).toBeVisible();
      if (expected.surface === "home") {
        await expect(page.locator(".j-root")).toHaveAttribute(
          "data-jaunder-private",
          "true",
        );
        await expect(page.locator("a[href='/logout']")).toBeVisible();
      } else {
        await expect(page.locator(".j-root")).toHaveAttribute(
          "data-theme",
          "studio",
        );
      }
      await expect
        .poll(() =>
          page.evaluate(() =>
            Array.from(
              document.querySelectorAll<HTMLLinkElement>(
                'link[rel="stylesheet"]',
              ),
            ).every((link) => link.sheet !== null),
          ),
        )
        .toBe(true);
      const observation = await page.evaluate(async (surface: Surface) => {
        await document.fonts.ready;
        await Promise.all(
          Array.from(document.images).map((image) => image.decode()),
        );
        const publicLinks = Array.from(
          document.querySelectorAll<HTMLLinkElement>(
            "link[data-jaunder-theme-stylesheet]",
          ),
        );
        const application = Array.from(
          document.querySelectorAll<HTMLLinkElement>('link[rel="stylesheet"]'),
        ).filter(
          (link) =>
            !link.hasAttribute("data-jaunder-theme-stylesheet") &&
            !link.hasAttribute("data-jaunder-theme-staged"),
        );
        if (application.length !== 1)
          throw new Error("document does not have one application stylesheet");
        const app = application[0].getAttribute("href") ?? "";
        if (!/^\/theme\/[0-9a-f]{64}$/.test(app))
          throw new Error("application stylesheet is not digest-addressed");
        const topbar = document.querySelector(".j-topbar");
        if (!topbar) throw new Error("document lacks trusted topbar");
        const themeSurface = document.querySelector(
          "[data-jaunder-theme-surface]",
        );
        let presentation = null;
        if (surface !== "home") {
          if (publicLinks.length !== 1 || !themeSurface)
            throw new Error("public document lacks one active presentation");
          const seedElement = document.getElementById("jaunder-seed");
          if (!seedElement?.textContent)
            throw new Error("fresh public document lacks projector seed");
          const seed = JSON.parse(seedElement.textContent) as {
            theme: { stylesheet_url: string; revision: string };
          };
          const stylesheet = publicLinks[0].getAttribute("href") ?? "";
          if (
            seed.theme.stylesheet_url !== stylesheet ||
            !/^[0-9a-f]{64}$/.test(seed.theme.revision)
          ) {
            throw new Error(
              "active presentation disagrees with fresh projector revision",
            );
          }
          presentation = {
            stylesheet_url: stylesheet,
            revision: seed.theme.revision,
          };
        }
        return {
          surface,
          application_url: app,
          application_digest: app.slice("/theme/".length),
          public_presentation: presentation,
          topbar_border_color: getComputedStyle(topbar).borderBottomColor,
          studio_accent:
            surface === "home"
              ? null
              : getComputedStyle(themeSurface!)
                  .getPropertyValue("--accent")
                  .trim(),
          public_links: publicLinks.length,
          staged_package_links: document.querySelectorAll(
            "link[data-jaunder-theme-staged]",
          ).length,
        };
      }, expected.surface);
      expect(observation.application_url).toBe(expected.application.url);
      expect(observation.public_presentation?.stylesheet_url ?? null).toBe(
        expected.public_presentation?.stylesheet.url ?? null,
      );
      expect(observation.public_presentation?.revision ?? null).toBe(
        expected.public_presentation?.revision ?? null,
      );
      expect(observation.public_links).toBe(
        expected.surface === "home" ? 0 : 1,
      );
      expect(observation.staged_package_links).toBe(0);
      if (expected.topbar_border_color !== null)
        expect(observation.topbar_border_color).toBe(
          expected.topbar_border_color,
        );
      if (expected.studio_accent !== null)
        expect(observation.studio_accent).toBe(expected.studio_accent);
      surfaces.push(observation);
      const html = await page.content();
      expect(html).not.toContain("/style/jaunder.css");
      expect(html).not.toContain("/style/jaunder-themes.css");
    }
    const cached_assets = [];
    for (const url of request.requested_cache_urls) {
      await expect
        .poll(() =>
          Boolean(this.publicCache.get(url) ?? this.homeCache.get(url)),
        )
        .toBe(true);
      const hit = this.publicCache.get(url) ?? this.homeCache.get(url);
      if (!hit)
        throw new Error("required actual browser cache evidence missing");
      cached_assets.push(hit);
    }
    const old_assets = [];
    for (const asset of request.requested_http_assets) {
      const response = await this.anonymous.request.get(
        `${BASE_URL}${asset.url}`,
      );
      expect(response.status()).toBe(200);
      const bytes = await response.body();
      const headers = response.headers();
      const conditional = await this.anonymous.request.get(
        `${BASE_URL}${asset.url}`,
        {
          headers: { "If-None-Match": headers.etag },
        },
      );
      const conditionalBytes = await conditional.body();
      const observed = {
        role: asset.role,
        url: new URL(response.url()).pathname,
        digest: new URL(response.url()).pathname.slice("/theme/".length),
        sha256: createHash("sha256").update(bytes).digest("hex"),
        mime: headers["content-type"],
        bytes: bytes.length,
        etag: headers.etag,
        cache_control: headers["cache-control"],
        status_200: response.status(),
        status_304: conditional.status(),
        body_bytes_304: conditionalBytes.length,
        readable_after_restore:
          request.phase === "restored" ? response.status() === 200 : null,
      };
      expect(observed.sha256).toBe(asset.sha256);
      expect(observed.mime).toBe(asset.mime);
      expect(observed.bytes).toBe(asset.bytes);
      expect(observed.etag).toBe(asset.etag);
      expect(observed.cache_control).toBe(
        "public, max-age=31536000, immutable",
      );
      expect(observed.status_304).toBe(304);
      expect(observed.body_bytes_304).toBe(0);
      old_assets.push(observed);
    }
    for (const path of ["/style/jaunder.css", "/style/jaunder-themes.css"]) {
      const response = await this.anonymous.request.get(`${BASE_URL}${path}`);
      expect(response.status()).toBe(404);
    }
    await this.registerCanaries(
      (await this.authenticated.cookies()).map((cookie) => cookie.value),
    );
    return {
      sequence: request.sequence,
      phase: request.phase,
      fixture: request.fixture,
      transition: request.transition,
      backend: request.backend,
      anonymous_context_id: this.anonymousId,
      authenticated_context_id: this.authenticatedId,
      surfaces,
      cached_assets,
      old_assets,
    };
  }
  async close(): Promise<void> {
    const results = await Promise.allSettled([
      this.publicCache.close(),
      this.homeCache.close(),
      this.anonymous.close(),
      this.authenticated.close(),
    ]);
    const failures = results.filter((result) => result.status === "rejected");
    if (failures.length) throw new Error("styling context cleanup failed");
  }
}
