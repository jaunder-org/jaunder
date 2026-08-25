import { test, expect } from "./fixtures";
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
