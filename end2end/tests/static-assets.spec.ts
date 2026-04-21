import { test, expect } from "./fixtures";

test("jaunder.css is served with status 200 and text/css content-type", async ({
  page,
}) => {
  const response = await page.request.get(
    "http://localhost:3000/style/jaunder.css",
  );
  expect(response.status()).toBe(200);
  const contentType = response.headers()["content-type"] ?? "";
  expect(contentType).toContain("text/css");
});

test("jaunder-themes.css is served with status 200 and text/css content-type", async ({
  page,
}) => {
  const response = await page.request.get(
    "http://localhost:3000/style/jaunder-themes.css",
  );
  expect(response.status()).toBe(200);
  const contentType = response.headers()["content-type"] ?? "";
  expect(contentType).toContain("text/css");
});
