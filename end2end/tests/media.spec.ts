import { test, expect, slowBrowserFirstNavigationTimeoutMs } from "./fixtures";
import {
  BASE_URL,
  confirmedMutation,
  goto,
  signInAsNewUser,
  signInAsNewUserRecord,
  click,
  waitForSelector,
  failServerFn,
  type MutationOutcome,
  stallServerFn,
} from "./helpers";
import { createPostViaApi } from "./posts";
import { navigateInApp } from "./navigate";
import type { Page } from "@playwright/test";
import { seedConfigViaTool } from "./seed";

type UploadedMedia = { url: string; filename: string };

/** Uploads `name` and returns the upload response (`url`, canonical `filename`). */
async function uploadMedia(
  page: Page,
  name: string,
  content: Buffer = Buffer.from("delete guard content"),
): Promise<UploadedMedia> {
  const response = await page.request.post(BASE_URL + "/api/media/upload", {
    multipart: {
      file: {
        name,
        mimeType: "image/jpeg",
        buffer: content,
      },
    },
  });
  expect(response.status()).toBe(200);
  return confirmedMutation(
    (await response.json()) as MutationOutcome<UploadedMedia>,
    "media::upload",
  );
}

function countMediaRequests(page: Page): {
  capabilityRequests: () => number;
  deleteRequests: () => number;
  listRequests: () => number;
  usageRequests: () => number;
} {
  let capabilityRequests = 0;
  let deleteRequests = 0;
  let listRequests = 0;
  let usageRequests = 0;
  page.on("request", (request) => {
    if (request.url().includes("/api/media/get_uploads_enabled"))
      capabilityRequests += 1;
    if (request.url().includes("/api/media/delete")) deleteRequests += 1;
    if (request.url().includes("/api/media/list_mine")) listRequests += 1;
    if (request.url().includes("/api/media/get_usage")) usageRequests += 1;
  });
  return {
    capabilityRequests: () => capabilityRequests,
    deleteRequests: () => deleteRequests,
    listRequests: () => listRequests,
    usageRequests: () => usageRequests,
  };
}

async function openMediaLibrary(page: Page): Promise<void> {
  await navigateInApp(page, () => click(page, "a[href='/media']"), {
    url: "/media",
    ready: "button:has-text('Attach media')",
  });
}

test.describe("Media upload and serving", () => {
  test("authenticated user can upload and access media", async ({ page }) => {
    await signInAsNewUser(page);

    // Drive `media::upload` directly — the session cookie is in the page's
    // cookie jar and the helper unwraps its confirmed mutation payload.
    const fileContent = Buffer.from("fake image content for testing");
    const json = await uploadMedia(page, "test-image.jpg", fileContent);
    expect(json.filename).toBe("test-image.jpg");
    expect(json.url).toContain("/media/upload/");

    // Access the served file (public, no auth needed)
    const serveResponse = await page.request.get(BASE_URL + json.url);
    expect(serveResponse.status()).toBe(200);
    expect(serveResponse.headers()["cache-control"]).toBe(
      "public, max-age=31536000, immutable",
    );
  });

  test("a filename needing percent-encoding uploads and serves", async ({
    page,
  }) => {
    await signInAsNewUser(page);

    // A space is a legal filename, so this is an ordinary upload. Through a real browser
    // stack it is also the one place the whole chain is exercised: the URL the server
    // derives, the name it wrote on disk, and the request the browser sends back for it
    // (#675). Before the fix the derived URL carried a raw space.
    const fileContent = Buffer.from("spaced filename content");
    const json = await uploadMedia(page, "my holiday photo.jpg", fileContent);
    // Since #720 the wire field carries the canonical encoded spelling — it is a lookup
    // key, not a display value, so it matches the URL segment and the on-disk name byte
    // for byte. Display surfaces decode it.
    expect(json.filename).toBe("my%20holiday%20photo.jpg");
    expect(json.url).toContain("my%20holiday%20photo.jpg");
    expect(json.url).not.toContain(" ");

    const serveResponse = await page.request.get(BASE_URL + json.url);
    expect(serveResponse.status()).toBe(200);
    expect(await serveResponse.text()).toBe("spaced filename content");
  });

  test("proxy canonicalizes a media source URL without following its redirect", async ({
    page,
  }) => {
    const { userId } = await signInAsNewUserRecord(page);
    const proxyUrl = new URL("/media/proxy", BASE_URL);
    proxyUrl.searchParams.set("url", "HTTP://EXAMPLE.COM:80");
    proxyUrl.searchParams.set("user_id", String(userId));

    // A zero redirect budget observes the server boundary and keeps this test
    // from contacting the remote target.
    const response = await page.request.get(proxyUrl.href, { maxRedirects: 0 });

    expect(response.status()).toBe(307);
    expect(response.headers()["location"]).toBe("http://example.com/");
  });

  test("proxy rejects an invalid media source URL", async ({ page }) => {
    const { userId } = await signInAsNewUserRecord(page);
    const proxyUrl = new URL("/media/proxy", BASE_URL);
    proxyUrl.searchParams.set("url", "not-a-url");
    proxyUrl.searchParams.set("user_id", String(userId));

    const response = await page.request.get(proxyUrl.href, { maxRedirects: 0 });

    expect(response.status()).toBe(400);
  });

  test("ordinary media delete confirms and removes unreferenced item", async ({
    page,
  }) => {
    // The display label decodes the canonical filename while the typed request keeps
    // the canonical identity needed by the delete boundary.
    await signInAsNewUser(page);
    await goto(page, "/");

    await uploadMedia(
      page,
      "my holiday photo.jpg",
      Buffer.from("spaced filename content"),
    );

    // Reached via the nav link and pinned on the page's own landmark, matching the
    // sibling media-page tests below — a bare `goto` races the CSR shell's mount.
    await openMediaLibrary(page);
    await expect(
      page.getByRole("link", { name: "my holiday photo.jpg" }),
    ).toBeVisible();

    // The label is cosmetic and decodes. Successful deletion proves the typed request
    // retained the encoded filename key expected by `Filename` at the wire boundary.
    const counts = countMediaRequests(page);
    page.on("dialog", (dialog) => dialog.accept());
    const button = page.getByRole("button", { name: "Delete", exact: true });
    expect(await button.getAttribute("onclick")).toContain(
      "Delete this media item?",
    );

    const release = await stallServerFn(page, "media/delete");
    await button.click();
    await expect.poll(counts.deleteRequests).toBe(1);
    await expect(button).toBeDisabled();
    await button.click({ force: true });
    expect(counts.deleteRequests()).toBe(1);
    release();
    await expect(
      page.getByRole("link", { name: "my holiday photo.jpg" }),
    ).toHaveCount(0);
    await expect.poll(counts.listRequests).toBe(1);
    await expect.poll(counts.usageRequests).toBe(1);
  });

  test("unauthenticated upload is rejected", async ({ page }) => {
    // No session: `require_auth()` rejects and the server fn returns a serialized
    // `WebError::Unauthorized` — not necessarily a bare 401 status.
    const response = await page.request.post(BASE_URL + "/api/media/upload", {
      multipart: {
        file: {
          name: "test.jpg",
          mimeType: "image/jpeg",
          buffer: Buffer.from("data"),
        },
      },
    });
    expect(response.ok()).toBeFalsy();
    const body = await response.text();
    expect(body).toContain("unauthorized");
  });

  test("media nav link appears for authenticated users", async ({
    page,
  }, testInfo) => {
    await signInAsNewUser(page);
    // Seeded helpers don't navigate (spec D5) — mount `/` so the sidebar exists.
    await goto(page, "/", {
      timeout: slowBrowserFirstNavigationTimeoutMs(testInfo, 30_000),
    });
    await waitForSelector(page, "a[href='/media']");
  });

  test("media manage page is reachable via nav link", async ({
    page,
  }, testInfo) => {
    await signInAsNewUser(page);
    // Seeded helpers don't navigate (spec D5) — mount `/` so the sidebar exists.
    await goto(page, "/", {
      timeout: slowBrowserFirstNavigationTimeoutMs(testInfo, 30_000),
    });
    await openMediaLibrary(page);
  });

  test("upload widget on create-post page uploads file and shows URL", async ({
    page,
  }) => {
    await signInAsNewUser(page);
    await goto(page, "/posts/new");

    // Use setInputFiles on the hidden file input to bypass the OS dialog.
    const fileInput = page.locator("input[type='file']").first();
    await fileInput.setInputFiles({
      name: "test-image.png",
      mimeType: "image/png",
      buffer: Buffer.from("fake png content"),
    });

    // The upload should complete and show the URL in a readonly input.
    await page
      .locator("input[readonly]")
      .waitFor({ state: "visible", timeout: 10000 });
    const url = await page.locator("input[readonly]").inputValue();
    expect(url).toContain("/media/upload/");
  });

  test("upload widget on the /app cockpit uploads file and shows URL", async ({
    page,
  }) => {
    await signInAsNewUser(page);
    // The /app cockpit shows the InlineComposer (#181), which includes MediaUpload.
    await goto(page, "/app");
    await waitForSelector(page, ".j-composer");
    const fileInput = page.locator(".j-composer input[type='file']").first();
    await fileInput.setInputFiles({
      name: "home-image.png",
      mimeType: "image/png",
      buffer: Buffer.from("fake png content for home"),
    });
    await page
      .locator(".j-composer input[readonly]")
      .waitFor({ state: "visible", timeout: 10000 });
    const url = await page.locator(".j-composer input[readonly]").inputValue();
    expect(url).toContain("/media/upload/");
  });
});

test.describe("Media upload capability", () => {
  test.afterEach(async () => {
    await seedConfigViaTool("media.uploads_enabled", "true");
  });

  test("disabled media page remains usable for existing media while uploads are rejected", async ({
    page,
  }) => {
    await seedConfigViaTool("media.uploads_enabled", "true");
    await signInAsNewUser(page);

    const existing = await uploadMedia(
      page,
      "read-only-media.jpg",
      Buffer.from("media remains available while uploads are disabled"),
    );
    await seedConfigViaTool("media.uploads_enabled", "false");

    await goto(page, "/");
    const counts = countMediaRequests(page);

    await navigateInApp(page, () => click(page, "a[href='/media']"), {
      url: "/media",
      ready: "text=Media uploads are disabled by the site operator.",
    });
    await expect.poll(counts.capabilityRequests).toBe(1);
    await expect(
      page.getByText("Media uploads are disabled by the site operator.", {
        exact: true,
      }),
    ).toBeVisible();
    await expect(
      page.getByRole("button", { name: "Attach media" }),
    ).toHaveCount(0);
    await expect(page.locator("input[type='file']")).toHaveCount(0);

    await expect(
      page.getByRole("link", { name: "read-only-media.jpg" }),
    ).toBeVisible();
    const retrieval = await page.request.get(BASE_URL + existing.url);
    expect(retrieval.status()).toBe(200);

    const rejected = await page.request.post(BASE_URL + "/api/media/upload", {
      multipart: {
        file: {
          name: "rejected.jpg",
          mimeType: "image/jpeg",
          buffer: Buffer.from("manager must reject this direct upload"),
        },
      },
    });
    expect(rejected.status()).toBe(403);
    expect(await rejected.text()).toContain("media uploads are disabled");

    page.on("dialog", (dialog) => dialog.accept());
    await page.getByRole("button", { name: "Delete", exact: true }).click();
    await expect(
      page.getByText("Media deleted.", { exact: true }),
    ).toBeVisible();
    await expect(
      page.getByRole("link", { name: "read-only-media.jpg" }),
    ).toHaveCount(0);
  });

  test("enabled media page retains its upload control", async ({ page }) => {
    await seedConfigViaTool("media.uploads_enabled", "true");
    await signInAsNewUser(page);
    await goto(page, "/");
    await openMediaLibrary(page);

    const fileInput = page.locator("input[type='file']");
    await expect(fileInput).toHaveCount(1);
    await expect(
      page.getByRole("button", { name: "Attach media" }),
    ).toBeVisible();
    await fileInput.setInputFiles({
      name: "enabled-media.jpg",
      mimeType: "image/jpeg",
      buffer: Buffer.from("enabled uploads still use the existing widget"),
    });
    await expect(page.locator("input[readonly]")).toBeVisible();
  });
});

test.describe("Media delete guard", () => {
  async function submitOrdinaryDeleteOnce(page: Page): Promise<{
    release: () => void;
    deleteRequests: () => number;
    listRequests: () => number;
    usageRequests: () => number;
  }> {
    const counts = countMediaRequests(page);
    const release = await stallServerFn(page, "media/delete");
    const button = page.getByRole("button", { name: "Delete", exact: true });
    await button.click();
    await expect.poll(counts.deleteRequests).toBe(1);
    await expect(button).toBeDisabled();
    await button.press("Enter");
    expect(counts.deleteRequests()).toBe(1);
    return { release, ...counts };
  }

  /**
   * Opens the media library and clicks the row's Delete, accepting the confirm.
   *
   * `exact: true` matters: once a refusal renders, the page also holds a
   * "Force delete <name>" button, and Playwright's `has-text` is a
   * case-insensitive *substring* — so a looser selector becomes ambiguous in
   * exactly the state these tests are about.
   */
  async function attemptDelete(page: Page): Promise<void> {
    await goto(page, "/");
    await openMediaLibrary(page);
    page.on("dialog", (dialog) => dialog.accept());
    await page.getByRole("button", { name: "Delete", exact: true }).click();
  }

  test("ordinary media delete confirms and refuses referenced item", async ({
    page,
  }) => {
    // The whole causal chain, which exists only end to end: rendering the post wrote
    // its post_media rows, and the guard reads them. No Rust test spans it.
    await signInAsNewUser(page);
    const { url } = await uploadMedia(page, "referenced.jpg");
    const { post_id } = await createPostViaApi(page, {
      body: `![pic](${url})`,
    });

    await goto(page, "/");
    await openMediaLibrary(page);
    page.on("dialog", (dialog) => dialog.accept());
    const { release, deleteRequests, listRequests, usageRequests } =
      await submitOrdinaryDeleteOnce(page);
    release();
    await expect.poll(deleteRequests).toBe(1);
    // Naming the post is part of the contract: this cannot pass on an empty lookup.
    await expect(
      page.getByText(
        new RegExp(`Cannot delete: referenced in post\\(s\\) ${post_id}\\.`),
      ),
    ).toBeVisible();
    expect(listRequests()).toBe(0);
    expect(usageRequests()).toBe(0);
    // The library labels a row with the *decoded* name.
    await expect(
      page.getByRole("link", { name: "referenced.jpg" }),
    ).toBeVisible();
    const forceButton = page.getByRole("button", { name: /^Force delete / });
    await expect(forceButton).toBeVisible();
    expect(await forceButton.getAttribute("onclick")).toContain(
      "Delete anyway?",
    );
  });

  test("forced media delete refuses rowless references and cannot double dispatch", async ({
    page,
    tracedContext,
  }) => {
    await signInAsNewUser(page);
    const { url } = await uploadMedia(page, "forced.jpg");
    // Force may discard the owner's own reconstruction. A different user's Post
    // supplies the global rowless reference that force must still preserve.
    const referenceContext = await tracedContext();
    try {
      const referencePage = await referenceContext.newPage();
      await signInAsNewUser(referencePage);
      await createPostViaApi(referencePage, { body: `![pic](${url})` });
    } finally {
      await referenceContext.close();
    }
    await attemptDelete(page);
    const forceButton = page.getByRole("button", { name: /^Force delete / });

    const refusalError = await page.locator("p.error").innerText();
    const failedCounts = countMediaRequests(page);
    await failServerFn(page, "media/delete");
    await forceButton.click();
    await expect.poll(failedCounts.deleteRequests).toBe(1);
    await expect(page.locator("p.error")).not.toHaveText(refusalError);
    await expect(page.getByRole("link", { name: "forced.jpg" })).toBeVisible();
    expect(failedCounts.listRequests()).toBe(0);
    expect(failedCounts.usageRequests()).toBe(0);
    await page.unroute("**/api/media/delete");

    await page.getByRole("button", { name: "Delete", exact: true }).click();
    await expect(
      page.getByText(/Cannot delete: referenced in post/),
    ).toBeVisible();
    await expect(forceButton).toBeVisible();
    const counts = countMediaRequests(page);
    const release = await stallServerFn(page, "media/delete");
    await forceButton.click();
    await expect.poll(counts.deleteRequests).toBe(1);
    await expect(forceButton).toBeDisabled();
    await forceButton.click({ force: true });
    expect(counts.deleteRequests()).toBe(1);
    const settled = page.waitForResponse(
      (response) =>
        response.url().includes("/api/media/delete") &&
        response.request().method() === "POST",
    );
    release();
    await settled;

    await expect(forceButton).toBeEnabled();
    await expect(
      page.getByText(/Cannot delete: referenced in post/),
    ).toBeVisible();
    await expect(page.getByRole("link", { name: "forced.jpg" })).toBeVisible();
    expect(counts.listRequests()).toBe(0);
    expect(counts.usageRequests()).toBe(0);
  });

  test("a post embedding the raw filename spelling blocks deletion", async ({
    page,
  }) => {
    // The #675 symptom, proved through the guard rather than the parser. The upload
    // returns the canonical percent-encoded URL; the post embeds the *raw* spelling,
    // which an exact-substring scan cannot see.
    await signInAsNewUser(page);
    const { url } = await uploadMedia(page, "my holiday photo.jpg");
    const rawUrl = url.replace(/%20/g, " ");
    expect(rawUrl).not.toBe(url);
    await createPostViaApi(page, { body: `<img src="${rawUrl}">` });

    await attemptDelete(page);
    await expect(
      page.getByText(/Cannot delete: referenced in post/),
    ).toBeVisible();
  });

  test("a post embedding the AtomPub member URL blocks deletion", async ({
    page,
  }) => {
    // The member URL shares no prefix with the serve URL, so an exact-URL match
    // could never see it however the filename was spelled.
    const username = await signInAsNewUser(page);
    const { url } = await uploadMedia(page, "linked.jpg");
    // "/media/upload/<p1>/<p2>/<sha>/<name>" splits to 7 parts with a leading "".
    const parts = url.split("/");
    const sha = parts[5];
    const name = parts[6];
    expect(sha).toHaveLength(64);
    await createPostViaApi(page, {
      body: `<a href="/atompub/${username}/media/${sha}/${name}">doc</a>`,
    });

    await attemptDelete(page);
    await expect(
      page.getByText(/Cannot delete: referenced in post/),
    ).toBeVisible();
  });
});
