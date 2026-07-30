import { test, expect, slowBrowserFirstNavigationTimeoutMs } from "./fixtures";
import { BASE_URL, goto, register, click, waitForSelector } from "./helpers";

test.describe("Media upload and serving", () => {
  test("authenticated user can upload and access media", async ({
    page,
  }, testInfo) => {
    await register(page, slowBrowserFirstNavigationTimeoutMs(testInfo, 30000));

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
  }, testInfo) => {
    await register(page, slowBrowserFirstNavigationTimeoutMs(testInfo, 30000));

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

  test("the media row decodes its label but not its delete key", async ({
    page,
  }, testInfo) => {
    // The media library is a CSR view, so this is the only surface that can observe both
    // spellings of one filename (#720). `component.rs` used to derive a single String for
    // the link text and the hidden delete field; they diverge now, and getting it wrong is
    // invisible to type checking — the label would show `my%20holiday%20photo.jpg`, or the
    // delete would fail at the wire door.
    await register(page, slowBrowserFirstNavigationTimeoutMs(testInfo, 30000));

    const response = await page.request.post(BASE_URL + "/api/upload_media", {
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
    // sibling media-page tests below — a bare `goto` races the CSR shell's hydration.
    await click(page, "a[href='/media']");
    await waitForSelector(page, "button:has-text('Attach media')");

    // Wait on the *label*, not the hidden input: `waitForSelector` waits for visibility,
    // which a `type="hidden"` field never reaches.
    //
    // The label is cosmetic and decodes; the hidden field is the lookup key and does not.
    await expect(
      page.getByRole("link", { name: "my holiday photo.jpg" }),
    ).toBeVisible();
    await expect(page.locator('input[name="filename"]')).toHaveValue(
      "my%20holiday%20photo.jpg",
    );

    // And the round-trip: deleting through the form succeeds, which is the end-to-end
    // check that the key was not decoded — `Filename`'s wire door rejects a raw value.
    page.on("dialog", (dialog) => dialog.accept());
    await click(page, 'button:has-text("Delete")');
    await expect(
      page.getByRole("link", { name: "my holiday photo.jpg" }),
    ).toHaveCount(0);
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
    await register(page, slowBrowserFirstNavigationTimeoutMs(testInfo, 30000));
    await waitForSelector(page, "a[href='/media']");
  });

  test("media manage page is reachable via nav link", async ({
    page,
  }, testInfo) => {
    await register(page, slowBrowserFirstNavigationTimeoutMs(testInfo, 30000));
    await click(page, "a[href='/media']");
    await waitForSelector(page, "button:has-text('Attach media')");
  });

  test("upload widget on create-post page uploads file and shows URL", async ({
    page,
  }, testInfo) => {
    await register(page, slowBrowserFirstNavigationTimeoutMs(testInfo, 30000));
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
  }, testInfo) => {
    await register(page, slowBrowserFirstNavigationTimeoutMs(testInfo, 30000));
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
