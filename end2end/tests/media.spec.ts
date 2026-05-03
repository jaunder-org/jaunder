import {
  test,
  expect,
  hydrationHeavyFirstNavigationTimeoutMs,
} from "./fixtures";
import { BASE_URL, register } from "./helpers";

test.describe("Media upload and serving", () => {
  test("authenticated user can upload and access media", async ({
    page,
  }, testInfo) => {
    await register(
      page,
      hydrationHeavyFirstNavigationTimeoutMs(testInfo, 30000),
    );

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
});
