import { appendFile, readFile, writeFile } from "node:fs/promises";

import { test } from "./fixtures";
import {
  createProductionBaseline,
  verifyFreshAppPasswordLifecycle,
  verifyProductionBaseline,
  type BaselineState,
} from "./production-baseline";
import { allowSecondBoot } from "./bootBudget";
import { goto, login, TEST_PASSWORD } from "./helpers";
import { seedConfigViaTool, seedSandboxProfileViaTool } from "./seed";

const statePath = process.env.JAUNDER_PRODUCTION_BASELINE_STATE;
const canaryPath = process.env.JAUNDER_PRODUCTION_BASELINE_CANARY_PATH;
const phase = process.env.JAUNDER_PRODUCTION_BASELINE_PHASE;
type BaselineBrowserState = {
  cookies: Array<{
    name: string;
    value: string;
    domain: string;
    path: string;
    expires: number;
    httpOnly: boolean;
    secure: boolean;
    sameSite: "Strict" | "Lax" | "None";
  }>;
};

async function registerCanaries(values: readonly string[]): Promise<void> {
  if (!canaryPath) return;
  await appendFile(
    canaryPath,
    values
      .filter((value) => value.length > 0)
      .map((value) => `${JSON.stringify(value)}\n`)
      .join(""),
  );
}

test("production baseline host bridge runs the shared behavior flow", async ({
  page,
  tracedContext,
}) => {
  test.skip(
    !statePath || !phase,
    "only xtask supplies restricted baseline state",
  );
  if (!statePath || !phase) return;
  const started = performance.now();
  let checks: string[];
  if (phase === "create") {
    const seededManifest = await seedSandboxProfileViaTool("demo");
    await seedConfigViaTool("site.base_url", "https://localhost:8443");
    const state = await createProductionBaseline(
      page,
      tracedContext,
      seededManifest,
    );
    const browser = await page.context().storageState();
    await registerCanaries([
      TEST_PASSWORD,
      ...browser.cookies.map((cookie) => cookie.value),
      state.appPassword,
      ...state.authorAccess.map((access) => access.appPassword),
      state.aliceSession.token,
      state.aliceSession.setCookie,
      state.aliceSession.marker,
      state.subscriberSession.token,
      state.subscriberSession.setCookie,
      state.subscriberSession.marker,
      state.nonSubscriberSession.token,
      state.nonSubscriberSession.setCookie,
      state.nonSubscriberSession.marker,
    ]);
    await writeFile(statePath, JSON.stringify({ browser, state }));
    checks = ["browser-create", "atompub", "feeds"];
  } else {
    const saved = JSON.parse(await readFile(statePath, "utf8")) as {
      browser: BaselineBrowserState;
      state: BaselineState;
    };
    await page.context().addCookies(saved.browser.cookies);
    await verifyProductionBaseline(page, saved.state, tracedContext);
    if (phase === "restored") {
      await page.context().clearCookies();
      allowSecondBoot(
        page,
        "fresh authentication after continuity verification is part of the recovery contract",
      );
      await login(page, saved.state.username, TEST_PASSWORD);
      allowSecondBoot(
        page,
        "App Password management has no in-app navigation control after fresh login",
      );
      await goto(page, "/sessions");
      const fresh = await verifyFreshAppPasswordLifecycle(
        page,
        saved.state,
        tracedContext,
      );
      await registerCanaries([fresh]);
    }
    if (phase !== "read-only" && phase !== "restored")
      throw new Error(`unknown production baseline phase ${phase}`);
    checks = ["browser-read-only", "atompub", "feeds"];
  }
  const durationMs = Math.max(
    1,
    Math.floor((performance.now() - started) / checks.length),
  );
  console.log(
    "production-baseline-result=" +
      JSON.stringify({
        phase,
        checks: checks.map((id) => ({
          id,
          outcome: "passed",
          duration_ms: durationMs,
        })),
      }),
  );
});
