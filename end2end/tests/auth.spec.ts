import { test, expect } from "@playwright/test";

test("register page shows form", async ({ page }) => {
  await page.goto("http://localhost:3000/register");

  await expect(page.locator("h1")).toHaveText("Register");
  await expect(page.locator('input[name="username"]')).toBeVisible();
  await expect(page.locator('input[name="password"]')).toBeVisible();
});

test("register with open policy succeeds", async ({ page }) => {
  await page.goto("http://localhost:3000/register");

  await page.fill('input[name="username"]', "newuser");
  await page.fill('input[name="password"]', "newpassword123");
  await page.click('button[type="submit"]');
  await page.waitForLoadState("networkidle");

  await expect(page.locator(".error")).not.toBeVisible();
});

test("login page shows form", async ({ page }) => {
  await page.goto("http://localhost:3000/login");

  await expect(page.locator("h1")).toHaveText("Login");
  await expect(page.locator('input[name="username"]')).toBeVisible();
  await expect(page.locator('input[name="password"]')).toBeVisible();
});

test("login with valid credentials succeeds", async ({ page }) => {
  await page.goto("http://localhost:3000/login");

  await page.fill('input[name="username"]', "testlogin");
  await page.fill('input[name="password"]', "testpassword123");
  await page.click('button[type="submit"]');
  await page.waitForLoadState("networkidle");

  await expect(page.locator(".error")).not.toBeVisible();
});

test("login with wrong password shows error", async ({ page }) => {
  await page.goto("http://localhost:3000/login");

  await page.fill('input[name="username"]', "testlogin");
  await page.fill('input[name="password"]', "wrongpassword!");
  await page.click('button[type="submit"]');
  await page.waitForLoadState("networkidle");

  await expect(page.locator(".error")).toBeVisible();
});

test("logout page logs out", async ({ page }) => {
  // Log in first to establish a session
  await page.goto("http://localhost:3000/login");
  await page.fill('input[name="username"]', "testlogin");
  await page.fill('input[name="password"]', "testpassword123");
  await page.click('button[type="submit"]');
  await page.waitForLoadState("networkidle");

  // Now navigate to logout
  await page.goto("http://localhost:3000/logout");

  await expect(page.locator("h1")).toContainText("Logging out");
  await page.waitForLoadState("networkidle");

  await expect(page.locator("p")).toContainText("You have been logged out.");
});
