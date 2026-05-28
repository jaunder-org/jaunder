import { test, expect } from "@playwright/test";
import { goto, register, BASE_URL } from "./helpers";
import {
  hydrationHeavyTimeoutMs,
  hydrationHeavyFirstNavigationTimeoutMs,
} from "./fixtures";

const FORMATS: { ext: string; mime: string }[] = [
  { ext: "rss", mime: "application/rss+xml" },
  { ext: "atom", mime: "application/atom+xml" },
  { ext: "json", mime: "application/feed+json" },
];

test("auto-discovery links are present on site home and user timeline, and resolve", async ({
  page,
}, info) => {
  info.setTimeout(hydrationHeavyTimeoutMs(info, 60_000));
  const username = await register(
    page,
    hydrationHeavyFirstNavigationTimeoutMs(info, 30_000),
  );

  // Test site home feed discovery
  await goto(page, "/");
  const homeLinks = await page.$$eval('head link[rel="alternate"]', (els) =>
    els.map((e) => ({
      href: (e as HTMLLinkElement).href,
      type: (e as HTMLLinkElement).type,
    })),
  );

  // Verify all three formats exist on home
  for (const fmt of FORMATS) {
    const link = homeLinks.find((l) => l.type === fmt.mime);
    expect(link, `${fmt.mime} on /`).toBeTruthy();
    const res = await page.request.get(link!.href);
    expect(res.status()).toBe(200);
    expect(res.headers()["content-type"]).toContain(fmt.mime);
  }

  // Test user timeline feed discovery (canonical user URL is ~-prefixed)
  await goto(page, `/~${username}`);
  const userLinks = await page.$$eval('head link[rel="alternate"]', (els) =>
    els.map((e) => ({
      href: (e as HTMLLinkElement).href,
      type: (e as HTMLLinkElement).type,
    })),
  );

  // Verify all three formats exist on user timeline
  for (const fmt of FORMATS) {
    const link = userLinks.find((l) => l.type === fmt.mime);
    expect(link, `${fmt.mime} on /~${username}`).toBeTruthy();
    const res = await page.request.get(link!.href);
    expect(res.status()).toBe(200);
    expect(res.headers()["content-type"]).toContain(fmt.mime);
  }
});
