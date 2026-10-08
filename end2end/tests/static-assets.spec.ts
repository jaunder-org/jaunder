import { test, expect } from "./fixtures";
import { BASE_URL } from "./helpers";

const RETIRED_STYLESHEETS = ["jaunder.css", "jaunder-themes.css"] as const;

for (const filename of RETIRED_STYLESHEETS) {
  test(`${filename} is retired before the document fallback`, async ({
    page,
  }) => {
    const response = await page.request.get(`${BASE_URL}/style/${filename}`);
    expect(response.status()).toBe(404);
    expect(response.headers()["content-type"] ?? "").not.toContain("text/html");
  });
}

test("the stable favicon revalidates its stored representation", async ({
  page,
}) => {
  const response = await page.request.get(`${BASE_URL}/favicon.ico`);
  expect(response.status()).toBe(200);
  expect(response.headers()["content-type"]).toMatch(/^image\//);
  expect(response.headers()["cache-control"]).toBe("no-cache");
  expect((await response.body()).length).toBeGreaterThan(0);
  const etag = response.headers().etag;
  expect(etag).toBeTruthy();
  const conditional = await page.request.get(`${BASE_URL}/favicon.ico`, {
    headers: { "If-None-Match": etag },
  });
  expect(conditional.status()).toBe(304);
  expect(conditional.headers().etag).toBe(etag);
  expect(conditional.headers()["cache-control"]).toBe("no-cache");
  expect((await conditional.body()).length).toBe(0);
});
