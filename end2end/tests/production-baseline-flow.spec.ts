import { test } from "./fixtures";
import {
  applyBaselineSiteConfig,
  createProductionBaseline,
  setBaselineTestBudget,
  verifyFreshAppPasswordLifecycle,
  verifyProductionBaseline,
} from "./production-baseline";
import { seedSandboxProfileViaTool } from "./seed";

// Profile seeding changes site config, so the test sets the site config it
// depends on before creating records.

test("production baseline creates and verifies canonical browser and AtomPub records", async ({
  page,
  tracedContext,
}) => {
  setBaselineTestBudget();
  const seededManifest = await seedSandboxProfileViaTool("demo");
  await applyBaselineSiteConfig();
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
});
