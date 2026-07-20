import { test, expect, slowBrowserFirstNavigationTimeoutMs } from "./fixtures";
import { BASE_URL, goto, register, click, waitForSelector } from "./helpers";

test.describe("Media upload and serving", () => {
  test("authenticated user can upload and access media", async ({
    page,
  }, testInfo) => {
    await register(page, slowBrowserFirstNavigationTimeoutMs(testInfo, 30000));

    // Upload via the API directly — session cookie is in page's cookie jar
    const fileContent = Buffer.from("fake image content for testing");
    const response = await page.request.post(BASE_URL + "/media/upload", {
      multipart: {
        file: {
          name: "test-image.jpg",
          mimeType: "image/jpeg",
          buffer: fileContent,
        },
      },
    });
    expect(response.status()).toBe(201);

    const json = await response.json();
    expect(json.sha256).toBeTruthy();
    expect(json.filename).toBe("test-image.jpg");
    expect(json.url).toContain("/media/upload/");

    // Access the served file (public, no auth needed)
    const serveResponse = await page.request.get(BASE_URL + json.url);
    expect(serveResponse.status()).toBe(200);
    expect(serveResponse.headers()["cache-control"]).toBe(
      "public, max-age=31536000, immutable",
    );
  });

  test("unauthenticated upload returns 401", async ({ page }) => {
    const response = await page.request.post(BASE_URL + "/media/upload", {
      multipart: {
        file: {
          name: "test.jpg",
          mimeType: "image/jpeg",
          buffer: Buffer.from("data"),
        },
      },
    });
    expect(response.status()).toBe(401);
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
