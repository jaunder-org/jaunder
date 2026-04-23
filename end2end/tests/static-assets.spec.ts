import { test, expect } from "./fixtures";
import { BASE_URL } from "./helpers";

test("jaunder.css is served with status 200 and text/css content-type", async ({
  page,
}) => {
  const response = await page.request.get(`${BASE_URL}/style/jaunder.css`);
  expect(response.status()).toBe(200);
  const contentType = response.headers()["content-type"] ?? "";
  expect(contentType).toContain("text/css");
});

test("jaunder-themes.css is served with status 200 and text/css content-type", async ({
  page,
}) => {
  const response = await page.request.get(
    `${BASE_URL}/style/jaunder-themes.css`,
  );
  expect(response.status()).toBe(200);
  const contentType = response.headers()["content-type"] ?? "";
  expect(contentType).toContain("text/css");
});
