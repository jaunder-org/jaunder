import { test } from "./fixtures";
import {
  applyBaselineSiteConfig,
  createProductionBaseline,
  setBaselineTestBudget,
  verifyFreshAppPasswordLifecycle,
  verifyProductionBaseline,
} from "./production-baseline";
import { seedConfigViaTool, seedSandboxProfileViaTool } from "./seed";

// This remains an ordinary cross-browser E2E spec; Playwright schedules it in
// the serial admin-site project because profile seeding temporarily changes site config.

test("production baseline creates and verifies canonical browser and AtomPub records", async ({
  page,
  tracedContext,
}) => {
  setBaselineTestBudget();
  const seededManifest = await seedSandboxProfileViaTool("demo");
  await applyBaselineSiteConfig();
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
    await seedConfigViaTool("posts.default_audience", "private");
  }
});
