import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { test } from "./fixtures";
import {
  applyBaselineSiteConfig,
  createProductionBaseline,
  setBaselineTestBudget,
  verifyFreshAppPasswordLifecycle,
  verifyProductionBaseline,
  type BaselineState,
} from "./production-baseline";
import { seedSandboxProfileViaTool } from "./seed";

// One half of the upgrade test (docs/adr/0265-upgrade-test.md). The
// `baseline` phase runs on the earlier build: it creates and verifies the
// baseline records, then hands their state and the author's browser session to
// the `upgraded` phase, which runs on the current build after an in-place
// upgrade and verifies the same records. Only the upgrade check selects this
// spec; without its phase it skips.

const phase = process.env.JAUNDER_UPGRADE_PHASE;
const stateDir = process.env.JAUNDER_UPGRADE_STATE_DIR ?? "";
test.skip(
  (phase !== "baseline" && phase !== "upgraded") || stateDir === "",
  "runs only as a phase of the upgrade check",
);
const statePath = join(stateDir, "baseline-state.json");
const sessionPath = join(stateDir, "author-session.json");

if (phase === "baseline") {
  test("earlier build creates and verifies the baseline records", async ({
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
    setBaselineTestBudget();
    const state = JSON.parse(readFileSync(statePath, "utf8")) as BaselineState;
    // Only the author's context carries the earlier session; `test.use` would
    // also sign in the anonymous contexts verification opens.
    const author = await tracedContext({ storageState: sessionPath });
    const page = await author.newPage();
    await verifyProductionBaseline(page, state, tracedContext);
    await verifyFreshAppPasswordLifecycle(page, state, tracedContext);
  });
}
