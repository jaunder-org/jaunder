import { setTestBudget, test } from "./fixtures";
import { FEED_POLL_TIMEOUT_MS } from "./feeds";
import {
  createProductionBaseline,
  verifyFreshAppPasswordLifecycle,
  verifyProductionBaseline,
} from "./production-baseline";
import { seedConfigViaTool, seedSandboxProfileViaTool } from "./seed";

// This remains an ordinary cross-browser E2E spec; Playwright schedules it in
// the serial admin-site project because profile seeding temporarily changes site config.

const BASELINE_SETUP_ALLOWANCE_MS = 45_000;
const BASELINE_SEQUENTIAL_FEED_POLLS = 9;
test("production baseline creates and verifies canonical browser and AtomPub records", async ({
  page,
  tracedContext,
}) => {
  setTestBudget(
    BASELINE_SEQUENTIAL_FEED_POLLS * FEED_POLL_TIMEOUT_MS +
      BASELINE_SETUP_ALLOWANCE_MS,
  );
  const seededManifest = await seedSandboxProfileViaTool("demo");
  await seedConfigViaTool("site.base_url", "https://example.com");
  try {
    const state = await createProductionBaseline(
      page,
      tracedContext,
      seededManifest,
    );
    const verificationPage = await page.context().newPage();
    try {
      await verifyProductionBaseline(verificationPage, state, tracedContext);
      await verifyFreshAppPasswordLifecycle(
        verificationPage,
        state,
        tracedContext,
      );
    } finally {
      await verificationPage.close();
    }
  } finally {
    await seedConfigViaTool("site.title", "jaunder.local");
  }
});
