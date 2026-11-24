import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { setTestBudget, test } from "./fixtures";
import { FEED_POLL_TIMEOUT_MS } from "./feeds";
import {
  applyBaselineSiteConfig,
  createProductionBaseline,
  verifyFreshAppPasswordLifecycle,
  verifyProductionBaseline,
  type BaselineState,
} from "./production-baseline";
import { seedSandboxProfileViaTool } from "./seed";

// One half of the upgrade test (docs/superpowers/specs/*-issue-2069-*). The
// `baseline` phase runs on the earlier build: it creates and verifies the
// baseline records, then hands their state and the author's browser session to
// the `upgraded` phase, which runs on the current build after an in-place
// upgrade and verifies the same records. Only the upgrade check selects this
// spec; ordinary lanes never load it.

const phase = process.env.JAUNDER_UPGRADE_PHASE;
const stateDir = process.env.JAUNDER_UPGRADE_STATE_DIR;
if ((phase !== "baseline" && phase !== "upgraded") || !stateDir) {
  throw new Error(
    "upgrade.spec.ts requires JAUNDER_UPGRADE_PHASE=baseline|upgraded and JAUNDER_UPGRADE_STATE_DIR",
  );
}
const statePath = join(stateDir, "baseline-state.json");
const sessionPath = join(stateDir, "author-session.json");

const BASELINE_SETUP_ALLOWANCE_MS = 45_000;
const BASELINE_SEQUENTIAL_FEED_POLLS = 9;
const budget = () =>
  setTestBudget(
    BASELINE_SEQUENTIAL_FEED_POLLS * FEED_POLL_TIMEOUT_MS +
      BASELINE_SETUP_ALLOWANCE_MS,
  );

if (phase === "baseline") {
  test("earlier build creates and verifies the baseline records", async ({
    page,
    tracedContext,
  }) => {
    budget();
    const seededManifest = await seedSandboxProfileViaTool("demo");
    await applyBaselineSiteConfig();
    const state = await createProductionBaseline(
      page,
      tracedContext,
      seededManifest,
    );
    // A page boots once: verify on a fresh page of the author's context, as the
    // ordinary flow spec does.
    const verificationPage = await page.context().newPage();
    try {
      await verifyProductionBaseline(verificationPage, state, tracedContext);
    } finally {
      await verificationPage.close();
    }
    writeFileSync(statePath, JSON.stringify(state));
    await page.context().storageState({ path: sessionPath });
  });
} else {
  test("current build verifies the upgraded baseline records", async ({
    tracedContext,
  }) => {
    budget();
    const state = JSON.parse(readFileSync(statePath, "utf8")) as BaselineState;
    // Only the author's context carries the earlier session; `test.use` would
    // also sign in the anonymous contexts verification opens.
    const author = await tracedContext({ storageState: sessionPath });
    const page = await author.newPage();
    await verifyProductionBaseline(page, state, tracedContext);
    await verifyFreshAppPasswordLifecycle(page, state, tracedContext);
  });
}
