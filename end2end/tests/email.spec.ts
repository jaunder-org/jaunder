import { test, expect, hydrationHeavyTimeoutMs } from "./fixtures";
import * as fs from "fs";
import { waitForHydration } from "./hydration";

const MAIL_CAPTURE_FILE =
  process.env.JAUNDER_MAIL_CAPTURE_FILE ?? "/tmp/jaunder-mail.jsonl";

interface CapturedEmail {
  to: string[];
  from: string | null;
  subject: string;
  body_text: string;
}

function readLatestEmail(): CapturedEmail | null {
  if (!fs.existsSync(MAIL_CAPTURE_FILE)) return null;
  const content = fs.readFileSync(MAIL_CAPTURE_FILE, "utf-8");
  const lines = content
    .trim()
    .split("\n")
    .filter((l) => l.trim());
  if (lines.length === 0) return null;
  return JSON.parse(lines[lines.length - 1]) as CapturedEmail;
}

async function waitForLatestEmail(timeoutMs = 5000): Promise<CapturedEmail> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const email = readLatestEmail();
    if (email) return email;
    await new Promise((resolve) => setTimeout(resolve, 100));
  }

  throw new Error(
    `timed out waiting for captured email at ${MAIL_CAPTURE_FILE}`,
  );
}

// M3.10.11: Full email verification flow.
test("email verification flow completes successfully", async ({
  page,
}, testInfo) => {
  test.setTimeout(hydrationHeavyTimeoutMs(testInfo, 15_000));

  // Log in
  await page.goto("http://localhost:3000/login", {
    waitUntil: "domcontentloaded",
  });
  await waitForHydration(page);
  await page.fill('input[name="username"]', "testlogin");
  await page.fill('input[name="password"]', "testpassword123");
  await page.click('button[type="submit"]');
  // waitForURL is unreliable in Firefox for location.replace() navigations; wait
  // for the logout link that SSR renders on the home page after successful auth.
  await page.waitForSelector("a[href='/logout']");
  await waitForHydration(page);

  // Navigate to email settings and submit an address
  await page.goto("http://localhost:3000/profile/email", {
    waitUntil: "domcontentloaded",
  });
  await waitForHydration(page);
  await page.fill('input[name="email"]', "testlogin@example.com");
  await page.click('button[type="submit"]');

  await expect(page.locator('p:has-text("Check your email")')).toBeVisible({
    timeout: 10_000,
  });

  // Extract the verification token from the captured mail file
  const email = await waitForLatestEmail();
  const tokenMatch = email!.body_text.match(/token=([^\s]+)/);
  expect(tokenMatch).not.toBeNull();
  const token = tokenMatch![1];

  // Visit the verification link
  await page.goto(`http://localhost:3000/verify-email?token=${token}`, {
    waitUntil: "domcontentloaded",
  });
  await waitForHydration(page);
  await expect(page.locator('p:has-text("verified")')).toBeVisible({
    timeout: 10_000,
  });

  // Confirm email is shown as verified on the profile page
  await page.goto("http://localhost:3000/profile/email", {
    waitUntil: "domcontentloaded",
  });
  await waitForHydration(page);
  await expect(page.locator("p")).toContainText("verified", {
    timeout: 10_000,
  });
});

// M3.10.12: Invalid token shows an error.
test("visiting verify-email with invalid token shows error", async ({
  page,
}, testInfo) => {
  test.setTimeout(hydrationHeavyTimeoutMs(testInfo, 10_000));
  await page.goto(
    "http://localhost:3000/verify-email?token=totally_invalid_token",
    {
      waitUntil: "domcontentloaded",
    },
  );
  await page.waitForLoadState("networkidle");
  await expect(page.locator(".error")).toBeVisible();
});
