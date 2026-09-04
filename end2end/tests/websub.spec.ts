import { test, expect } from "./fixtures";
import {
  click,
  goto,
  signInAs,
  signInAsNewUser,
  waitForSelector,
} from "./helpers";
import { redriveDeadLettersViaCli, seedDeadLettersViaTool } from "./seed";
import { findPingWave, type CapturedPing } from "./websub";

function capturedPingLine(feedUrl: string, sentAt: string): string {
  const ping: CapturedPing = {
    hub_url: "https://hub.example.test/",
    feed_url: feedUrl,
    sent_at: sentAt,
  };
  return JSON.stringify(ping);
}

const RSS_URL = "http://localhost:3000/~alice/feed.rss";
const ATOM_URL = "http://localhost:3000/~alice/feed.atom";
const SITE_URL = "http://localhost:3000/feed.rss";

test("records before the cursor cannot satisfy a ping wave", async () => {
  const lines = [
    capturedPingLine(RSS_URL, "before-rss"),
    capturedPingLine(ATOM_URL, "before-atom"),
    capturedPingLine(SITE_URL, "after-site"),
  ];

  expect(findPingWave(lines, 2, [RSS_URL, ATOM_URL])).toBeUndefined();
});

test("an unrelated Site Syndication Feed ping cannot complete a wave", async () => {
  const lines = [
    capturedPingLine(RSS_URL, "rss"),
    capturedPingLine(SITE_URL, "site"),
  ];

  expect(findPingWave(lines, 0, [RSS_URL, ATOM_URL])).toBeUndefined();
});

test("a missing expected URL leaves the wave incomplete", async () => {
  const lines = [capturedPingLine(RSS_URL, "rss")];

  expect(findPingWave(lines, 0, [RSS_URL, ATOM_URL])).toBeUndefined();
});

test("duplicate requested URLs collapse to one result", async () => {
  const lines = [capturedPingLine(RSS_URL, "rss")];

  const wave = findPingWave(lines, 0, [RSS_URL, RSS_URL]);
  expect(wave?.map((ping) => ping.feed_url)).toEqual([RSS_URL]);
});

test("duplicate captured URLs preserve the first matching record", async () => {
  const lines = [
    capturedPingLine(RSS_URL, "first"),
    capturedPingLine(RSS_URL, "second"),
  ];

  const wave = findPingWave(lines, 0, [RSS_URL]);
  expect(wave?.map((ping) => ping.sent_at)).toEqual(["first"]);
});

test("a complete wave follows deduplicated request order", async () => {
  const lines = [
    capturedPingLine(RSS_URL, "rss"),
    capturedPingLine(ATOM_URL, "atom"),
  ];

  const wave = findPingWave(lines, 0, [ATOM_URL, RSS_URL, ATOM_URL]);
  expect(wave?.map((ping) => ping.feed_url)).toEqual([ATOM_URL, RSS_URL]);
});

test("operator filters, pages, redrives, and rejects a stale WebSub selection", async ({
  page,
}) => {
  const regenerationIds = await seedDeadLettersViaTool("regeneration", 51);
  const publicationIds = await seedDeadLettersViaTool("publication", 1);
  await signInAs(page, "testoperator");
  await goto(page, "/admin/websub");

  const regeneration = page.locator(".j-websub-dead-letters").filter({
    has: page.getByRole("heading", { name: "Regeneration dead letters" }),
  });
  const publication = page.locator(".j-websub-dead-letters").filter({
    has: page.getByRole("heading", { name: "Publication dead letters" }),
  });
  await expect(regeneration.locator("tbody tr")).toHaveCount(50);
  await expect(
    publication.getByText(String(publicationIds[0]), { exact: true }),
  ).toBeVisible();

  await click(page, '[data-test="websub-next-page"]');
  const redriveRow = regeneration.locator("tbody tr").filter({
    has: page.getByText(String(regenerationIds[0]), { exact: true }),
  });
  await expect(redriveRow).toBeVisible();
  await redriveRow.locator('input[type="checkbox"]').check();
  await click(page, '[data-test="websub-redrive-regeneration"]');
  await expect(regeneration.locator("p.success")).toContainText(
    "Selected dead-letter events queued for redrive.",
  );
  await expect(regeneration.locator("tbody tr")).toHaveCount(50);
  await expect(
    regeneration.locator('[data-test="websub-redrive-regeneration"]'),
  ).toBeDisabled();

  const staleRow = publication.locator("tbody tr").filter({
    has: page.getByText(String(publicationIds[0]), { exact: true }),
  });
  await staleRow.locator('input[type="checkbox"]').check();
  await redriveDeadLettersViaCli([String(publicationIds[0])]);
  await click(page, '[data-test="websub-redrive-publication"]');
  await expect(publication.locator("p.error")).toContainText(
    "one or more selected events are no longer dead-lettered",
  );
});

test("operator persists the configured WebSub hub", async ({ page }) => {
  await signInAs(page, "testoperator");
  await goto(page, "/admin/websub");

  const hub = page.locator('input[name="hub_url"]');
  await waitForSelector(page, 'input[name="hub_url"]');
  await hub.fill("https://hub.operator.example/");
  await click(page, 'button:has-text("Save WebSub Hub")');
  await expect(page.locator("p.success")).toContainText("WebSub hub saved.");
  await expect(hub).toHaveValue("https://hub.operator.example/");
});

test("nonoperators cannot inspect the WebSub recovery surface", async ({
  page,
}) => {
  await signInAsNewUser(page);
  await goto(page, "/admin/websub");

  await expect(page.locator("p.error").first()).toBeVisible();
  await expect(page.locator("table.j-table")).toHaveCount(0);
});
