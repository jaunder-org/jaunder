import { test, expect, slowBrowserFirstNavigationTimeoutMs } from "./fixtures";
import {
  BASE_URL,
  goto,
  signInAsNewUser,
  click,
  waitForSelector,
  failServerFn,
  stallServerFn,
} from "./helpers";
import { createPostViaApi } from "./posts";
import type { Page } from "@playwright/test";

test.describe("Media upload and serving", () => {
  test("authenticated user can upload and access media", async ({ page }) => {
    await signInAsNewUser(page);

    // Drive the `media::upload` server fn directly — session cookie is in page's
    // cookie jar. The fn returns 200 with the bare `UploadResponse` JSON.
    const fileContent = Buffer.from("fake image content for testing");
    const response = await page.request.post(BASE_URL + "/api/media/upload", {
      multipart: {
        file: {
          name: "test-image.jpg",
          mimeType: "image/jpeg",
          buffer: fileContent,
        },
      },
    });
    expect(response.status()).toBe(200);

    const json = await response.json();
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
    const response = await page.request.post(BASE_URL + "/api/media/upload", {
      multipart: {
        file: {
          name: "my holiday photo.jpg",
          mimeType: "image/jpeg",
          buffer: fileContent,
        },
      },
    });
    expect(response.status()).toBe(200);

    const json = await response.json();
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

  test("ordinary media delete confirms and removes unreferenced item", async ({
    page,
  }) => {
    // The display label decodes the canonical filename while the typed request keeps
    // the canonical identity needed by the delete boundary.
    await signInAsNewUser(page);
    await goto(page, "/");

    const response = await page.request.post(BASE_URL + "/api/media/upload", {
      multipart: {
        file: {
          name: "my holiday photo.jpg",
          mimeType: "image/jpeg",
          buffer: Buffer.from("spaced filename content"),
        },
      },
    });
    expect(response.status()).toBe(200);

    // Reached via the nav link and pinned on the page's own landmark, matching the
    // sibling media-page tests below — a bare `goto` races the CSR shell's mount.
    await click(page, "a[href='/media']");
    await waitForSelector(page, "button:has-text('Attach media')");
    await expect(
      page.getByRole("link", { name: "my holiday photo.jpg" }),
    ).toBeVisible();

    // The label is cosmetic and decodes. Successful deletion proves the typed request
    // retained the encoded filename key expected by `Filename` at the wire boundary.
    let deleteRequests = 0;
    let listRequests = 0;
    page.on("request", (request) => {
      if (request.url().includes("/api/media/delete")) deleteRequests += 1;
      if (request.url().includes("/api/media/list_mine")) listRequests += 1;
    });
    page.on("dialog", (dialog) => dialog.accept());
    const button = page.getByRole("button", { name: "Delete", exact: true });
    expect(await button.getAttribute("onclick")).toContain(
      "Delete this media item?",
    );

    const release = await stallServerFn(page, "media/delete");
    await button.click();
    await expect.poll(() => deleteRequests).toBe(1);
    await expect(button).toBeDisabled();
    await button.click({ force: true });
    expect(deleteRequests).toBe(1);
    release();
    await expect(
      page.getByRole("link", { name: "my holiday photo.jpg" }),
    ).toHaveCount(0);
    await expect.poll(() => listRequests).toBe(1);
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
    await click(page, "a[href='/media']");
    await waitForSelector(page, "button:has-text('Attach media')");
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

test.describe("Media delete guard", () => {
  /** Uploads `name` and returns the upload response (`url`, canonical `filename`). */
  async function uploadMedia(
    page: Page,
    name: string,
  ): Promise<{ url: string; filename: string }> {
    const response = await page.request.post(BASE_URL + "/api/media/upload", {
      multipart: {
        file: {
          name,
          mimeType: "image/jpeg",
          buffer: Buffer.from("delete guard content"),
        },
      },
    });
    expect(response.status()).toBe(200);
    return await response.json();
  }

  function countMediaRequests(page: Page): {
    deleteRequests: () => number;
    listRequests: () => number;
  } {
    let deleteRequests = 0;
    let listRequests = 0;
    page.on("request", (request) => {
      if (request.url().includes("/api/media/delete")) deleteRequests += 1;
      if (request.url().includes("/api/media/list_mine")) listRequests += 1;
    });
    return {
      deleteRequests: () => deleteRequests,
      listRequests: () => listRequests,
    };
  }

  async function submitOrdinaryDeleteOnce(page: Page): Promise<{
    release: () => void;
    deleteRequests: () => number;
    listRequests: () => number;
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
    await click(page, "a[href='/media']");
    await waitForSelector(page, "button:has-text('Attach media')");
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
    await click(page, "a[href='/media']");
    await waitForSelector(page, "button:has-text('Attach media')");
    page.on("dialog", (dialog) => dialog.accept());
    const { release, deleteRequests, listRequests } =
      await submitOrdinaryDeleteOnce(page);
    release();
    await expect.poll(deleteRequests).toBe(1);
    await expect.poll(listRequests).toBe(0);
    // Naming the post is part of the contract: this cannot pass on an empty lookup.
    await expect(
      page.getByText(
        new RegExp(`Cannot delete: referenced in post\\(s\\) ${post_id}\\.`),
      ),
    ).toBeVisible();
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

  test("forced media delete confirms, renders errors, and cannot double dispatch", async ({
    page,
  }) => {
    await signInAsNewUser(page);
    const { url } = await uploadMedia(page, "forced.jpg");
    await createPostViaApi(page, { body: `![pic](${url})` });
    await attemptDelete(page);
    const forceButton = page.getByRole("button", { name: /^Force delete / });

    await failServerFn(page, "media/delete");
    await forceButton.click();
    await expect(page.locator("p.error")).toBeVisible();
    await expect(page.getByRole("link", { name: "forced.jpg" })).toBeVisible();
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
    release();

    await expect(page.getByRole("link", { name: "forced.jpg" })).toHaveCount(0);
    await expect(page.locator("p.success")).toHaveText("Media deleted.");
    await expect.poll(counts.listRequests).toBe(1);
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
