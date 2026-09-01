import { test, expect } from "./fixtures";
import { BASE_URL } from "./helpers";

const STYLESHEETS = ["jaunder.css", "jaunder-themes.css"] as const;

for (const filename of STYLESHEETS) {
  test(`${filename} is served with status 200 and text/css content-type`, async ({
    page,
  }) => {
    const response = await page.request.get(`${BASE_URL}/style/${filename}`);
    expect(response.status()).toBe(200);
    const contentType = response.headers()["content-type"] ?? "";
    expect(contentType).toContain("text/css");
  });
}
