import { appendFile, readFile, rename, writeFile } from "node:fs/promises";

import { setTestBudget, test } from "./fixtures";
import {
  createProductionBaseline,
  verifyFreshAppPasswordLifecycle,
  verifyProductionBaseline,
  type BaselineState,
} from "./production-baseline";
import { allowSecondBoot } from "./bootBudget";
import { goto, login, TEST_PASSWORD } from "./helpers";
import { seedSandboxProfileViaTool } from "./seed";

const statePath = process.env.JAUNDER_PRODUCTION_BASELINE_STATE;
const canaryPath = process.env.JAUNDER_PRODUCTION_BASELINE_CANARY_PATH;
const coordinator = process.env.JAUNDER_PRODUCTION_BASELINE_COORDINATOR;
const PRODUCTION_BASELINE_HARNESS_BUDGET_MS = 30 * 60_000;

type BrowserRequest = {
  sequence: number;
  phase: "create" | "read-only" | "restored" | "close";
  seed_process?: string;
};

type BrowserResult = {
  sequence: number;
  phase: BrowserRequest["phase"];
  checks: Array<{ id: string; outcome: "passed"; duration_ms: number }>;
  error?: string;
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

async function publish(
  path: string,
  value: BrowserResult | { readonly: true },
): Promise<void> {
  const temporary = `${path}.tmp`;
  await writeFile(temporary, JSON.stringify(value));
  await rename(temporary, path);
}

async function waitForRequest(sequence: number): Promise<BrowserRequest> {
  if (!coordinator) throw new Error("baseline browser coordinator is missing");
  const path = `${coordinator}/request-${sequence}.json`;
  const deadline = Date.now() + 90_000;
  for (;;) {
    try {
      return JSON.parse(await readFile(path, "utf8")) as BrowserRequest;
    } catch (error: unknown) {
      if (!(
        error instanceof Error &&
        "code" in error &&
        error.code === "ENOENT"
      ))
        throw error;
      if (Date.now() >= deadline)
        throw new Error(`timed out waiting for browser request ${sequence}`);
      await new Promise<void>((resolve) => setTimeout(resolve, 50));
    }
  }
}

async function runPhase(
  request: BrowserRequest,
  page: Parameters<typeof createProductionBaseline>[0],
  tracedContext: Parameters<typeof createProductionBaseline>[1],
): Promise<BrowserResult> {
  if (!statePath) throw new Error("baseline browser state path is missing");
  const started = performance.now();
  let checks: string[];
  if (request.phase === "create") {
    if (!request.seed_process)
      throw new Error("create phase omitted the seed process path");
    process.env.JAUNDER_E2E_SEED_PROCESS = request.seed_process;
    const seededManifest = await seedSandboxProfileViaTool("demo");
    const state = await createProductionBaseline(
      page,
      tracedContext,
      seededManifest,
    );
    await registerCanaries([
      TEST_PASSWORD,
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
    await writeFile(statePath, JSON.stringify(state));
    checks = ["browser-create", "atompub", "feeds"];
  } else {
    const state = JSON.parse(
      await readFile(statePath, "utf8"),
    ) as BaselineState;
    const continuityPage = await page.context().newPage();
    try {
      await verifyProductionBaseline(continuityPage, state, tracedContext);
    } finally {
      await continuityPage.close();
    }
    if (request.phase === "restored") {
      const recoveryContext = await tracedContext();
      try {
        const recoveryPage = await recoveryContext.newPage();
        allowSecondBoot(
          recoveryPage,
          "fresh authentication after continuity verification is part of the recovery contract",
        );
        await login(recoveryPage, state.username, TEST_PASSWORD);
        allowSecondBoot(
          recoveryPage,
          "App Password management has no in-app navigation control after fresh login",
        );
        await goto(recoveryPage, "/sessions");
        const fresh = await verifyFreshAppPasswordLifecycle(
          recoveryPage,
          state,
          tracedContext,
        );
        await registerCanaries([fresh]);
      } finally {
        await recoveryContext.close();
      }
    }
    checks = ["browser-read-only", "atompub", "feeds"];
  }
  const durationMs = Math.max(
    1,
    Math.floor((performance.now() - started) / checks.length),
  );
  return {
    sequence: request.sequence,
    phase: request.phase,
    checks: checks.map((id) => ({
      id,
      outcome: "passed",
      duration_ms: durationMs,
    })),
  };
}

test("production baseline host bridge preserves one browser context", async ({
  page,
  tracedContext,
}) => {
  test.skip(
    !statePath || !coordinator,
    "only xtask supplies restricted baseline coordinator",
  );
  if (!statePath || !coordinator) return;
  setTestBudget(PRODUCTION_BASELINE_HARNESS_BUDGET_MS);
  await writeFile(`${coordinator}/ready`, "");
  for (let sequence = 1; ; sequence += 1) {
    const request = await waitForRequest(sequence);
    if (request.sequence !== sequence)
      throw new Error(
        `unexpected browser request sequence ${request.sequence}`,
      );
    if (request.phase === "close") {
      await publish(`${coordinator}/closed`, { readonly: true });
      return;
    }
    try {
      await publish(
        `${coordinator}/result-${sequence}.json`,
        await runPhase(request, page, tracedContext),
      );
    } catch (error) {
      await publish(`${coordinator}/result-${sequence}.json`, {
        sequence,
        phase: request.phase,
        checks: [],
        error:
          error instanceof Error
            ? (error.stack ?? error.message)
            : String(error),
      });
      throw error;
    }
  }
});
