import { test, expect } from "./fixtures";
import {
  BASE_URL,
  goto,
  signInAsNewUser,
  confirmedMutation,
  type MutationOutcome,
} from "./helpers";
import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import {
  readFileSync,
  writeFileSync,
  mkdtempSync,
  rmSync,
  readdirSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { resolve, join } from "node:path";
import { imageFixtureDirectory } from "./media-helpers";

const originalPath = join(imageFixtureDirectory, "jpeg-original.jpg");
const privateTags = [
  "IFD0:Make",
  "GPS:GPSLatitude",
  "ExifIFD:UserComment",
  "XMP-dc:Description",
  "IFD1:ThumbnailImage",
];

// The same pinned TEST-only oracle observes the owned input and actual public
// bytes. It is never an editor or runtime upload dependency.
function ordinaryTags(bytes: Buffer): Record<string, unknown> {
  const directory = mkdtempSync(join(tmpdir(), "jaunder-browser-oracle-"));
  try {
    const path = join(directory, "image");
    writeFileSync(path, bytes, { mode: 0o600 });
    return JSON.parse(
      execFileSync(
        "exiftool",
        [
          "-j",
          "-G1",
          "-Make",
          "-GPSLatitude",
          "-UserComment",
          "-Description",
          "-ThumbnailImage",
          path,
        ],
        { encoding: "utf8" },
      ),
    )[0];
  } finally {
    rmSync(directory, { recursive: true });
  }
}

test("real image UI upload sanitizes public bytes and clearly rejects malformed input", async ({
  page,
}) => {
  const original = readFileSync(originalPath);
  const before = ordinaryTags(original);
  for (const tag of privateTags) expect(before).toHaveProperty(tag);
  await signInAsNewUser(page);
  await goto(page, "/media");
  await expect(page.locator(".j-topbar h1")).toHaveText("Media");
  const input = page.locator("input[type='file']:not([accept]):not([capture])");
  const submitted = page.waitForResponse(
    (response) =>
      response.url().endsWith("/api/media/upload") &&
      response.request().method() === "POST",
  );
  await input.setInputFiles({
    name: "private.jpg",
    mimeType: "image/jpeg",
    buffer: original,
  });
  const response = await submitted;
  expect(response.status()).toBe(200);
  type Upload = {
    url: string;
    filename: string;
    sha256: string;
    size_bytes: number;
    content_type: string;
  };
  const uploaded = confirmedMutation(
    (await response.json()) as MutationOutcome<Upload>,
    "media::upload",
  );
  await expect(
    page.getByRole("link", { name: "private.jpg", exact: true }),
  ).toBeVisible();
  const publicResponse = await page.request.get(BASE_URL + uploaded.url);
  expect(publicResponse.status()).toBe(200);
  expect(publicResponse.headers()["content-type"]).toBe("image/jpeg");
  const publicBytes = await publicResponse.body();
  const digest = createHash("sha256").update(publicBytes).digest("hex");
  expect(digest).toBe(uploaded.sha256);
  expect(uploaded.url).toContain(digest);
  expect(publicResponse.headers()["etag"]).toBe(`"sha256-${digest}"`);
  expect(uploaded.size_bytes).toBe(publicBytes.length);
  const after = ordinaryTags(publicBytes);
  for (const tag of privateTags) expect(after).not.toHaveProperty(tag);
  expect(publicBytes.equals(original)).toBe(false);
  expect(readFileSync(originalPath).equals(original)).toBe(true);
  // Real browser decoding observes the retained image, not just a 200 response.
  expect(
    await page.evaluate(async (url) => {
      const image = new Image();
      image.src = url;
      await image.decode();
      return { width: image.naturalWidth, height: image.naturalHeight };
    }, uploaded.url),
  ).toEqual({ width: 32, height: 24 });
  const usageBefore = await (
    await page.request.post(BASE_URL + "/api/media/get_usage")
  ).json();
  expect(usageBefore.used_bytes).toBe(publicBytes.length);
  const invalid = Buffer.from("not an image");
  const rejected = page.waitForResponse(
    (candidate) =>
      candidate.url().endsWith("/api/media/upload") &&
      candidate.request().method() === "POST",
  );
  await input.setInputFiles({
    name: "rejected.jpg",
    mimeType: "image/jpeg",
    buffer: invalid,
  });
  expect((await rejected).status()).not.toBe(200);
  await expect(page.getByText("invalid image", { exact: true })).toBeVisible();
  await expect(
    page.getByRole("link", { name: "rejected.jpg", exact: true }),
  ).toHaveCount(0);
  expect(
    await (await page.request.post(BASE_URL + "/api/media/get_usage")).json(),
  ).toEqual(usageBefore);
  const rejectedDigest = createHash("sha256").update(invalid).digest("hex");
  const rejectedUrl = uploaded.url
    .replace(
      `${digest.slice(0, 2)}/${digest.slice(2, 4)}/${digest}`,
      `${rejectedDigest.slice(0, 2)}/${rejectedDigest.slice(2, 4)}/${rejectedDigest}`,
    )
    .replace("private.jpg", "rejected.jpg");
  expect((await page.request.get(BASE_URL + rejectedUrl)).status()).toBe(404);
  const storage = process.env.JAUNDER_STORAGE_PATH;
  expect(
    storage,
    "launcher supplies its actual disposable storage root",
  ).toBeTruthy();
  await expect
    .poll(() => readdirSync(resolve(storage!, "media/tmp")))
    .toEqual([]);
  expect(readFileSync(originalPath).equals(original)).toBe(true);
});
