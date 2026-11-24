import { setTestBudget, test } from "./fixtures";
import { FEED_POLL_TIMEOUT_MS } from "./feeds";
import {
  createProductionBaseline,
  verifyFreshAppPasswordLifecycle,
  verifyProductionBaseline,
} from "./production-baseline";
import { seedConfigViaTool, seedSandboxProfileViaTool } from "./seed";

// Profile seeding changes site config, so the test sets the site config it
// depends on before creating records.

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
  // Baseline records exercise public HTML and Syndication Feed projections.
  // Make that fixture intent explicit instead of depending on the safe Private
  // fallback for an unset Site Default Audience.
  await seedConfigViaTool("posts.default_audience", "public");
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
