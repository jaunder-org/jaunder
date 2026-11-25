import { readFileSync, rmSync } from "node:fs";
import { spawnSync } from "node:child_process";
import path from "node:path";

type CensusTest = {
  project_name: string;
  file: string;
  line: number;
  column: number;
  title_path: string[];
};
type Census = { schema_version: 1; complete: true; tests: CensusTest[] };
type Lane = {
  browser: string;
  enabled: boolean;
  shardIndex?: number;
  shardCount?: number;
};
const root = path.dirname(path.resolve(process.argv[1]));
const playwrightCli =
  process.env.JAUNDER_E2E_PLAYWRIGHT_CLI ?? "node_modules/.bin/playwright";
const { lanes, shardWeights } = JSON.parse(
  readFileSync(path.join(root, "e2e-lanes.json"), "utf8"),
) as { lanes: Lane[]; shardWeights: Record<string, string> };
function fail(message: string): never {
  throw new Error(`E2E census contract: ${message}`);
}
function run(
  label: string,
  project: string,
  shard: string[],
  weights?: string,
): Census {
  const output = `test-results/census-contract-${label}.json`;
  rmSync(path.join(root, output), { force: true });
  const result = spawnSync(
    process.execPath,
    [
      playwrightCli,
      "test",
      "--list",
      "--config",
      "playwright.config.ts",
      "--project",
      project,
      ...shard,
    ],
    {
      cwd: root,
      env: {
        ...process.env,
        JAUNDER_E2E_CENSUS_PREPARE: "1",
        JAUNDER_E2E_CENSUS_TOPOLOGY: `${label}-contract`,
        JAUNDER_E2E_CENSUS_OUTPUT: output,
        ...(weights ? { PWTEST_SHARD_WEIGHTS: weights } : {}),
      },
      encoding: "utf8",
    },
  );
  if (result.status !== 0) fail(`Playwright --list failed: ${result.stderr}`);
  return JSON.parse(readFileSync(path.join(root, output), "utf8")) as Census;
}
function canonical(test: CensusTest): string {
  if (
    !test.project_name ||
    !test.file ||
    !Number.isInteger(test.line) ||
    !Number.isInteger(test.column) ||
    !Array.isArray(test.title_path) ||
    test.title_path.length === 0 ||
    test.title_path.some((title) => !title)
  )
    fail(`malformed canonical identity: ${JSON.stringify(test)}`);
  return JSON.stringify([
    test.project_name,
    test.file,
    test.line,
    test.column,
    test.title_path,
  ]);
}
function population(census: Census): Set<string> {
  if (
    census.schema_version !== 1 ||
    !census.complete ||
    census.tests.length === 0
  )
    fail("incomplete census");
  const values = census.tests.map(canonical);
  const set = new Set(values);
  if (set.size !== values.length) fail("duplicate canonical identity");
  return set;
}
// Each browser's lane shards, under the lanes' shard weights, must select its
// one project's unsharded population exactly once: a dependency project would
// repeat in every shard.
for (const browser of [...new Set(lanes.map((lane) => lane.browser))]) {
  const shards = [
    ...new Set(
      lanes
        .filter((lane) => lane.enabled && lane.browser === browser)
        .map((lane) => `${lane.shardIndex}/${lane.shardCount}`),
    ),
  ];
  const full = population(run(browser, browser, []));
  const owner = new Map<string, string>();
  for (const shard of shards) {
    const label = `${browser}-${shard.replace("/", "-of-")}`;
    const selected = run(
      label,
      browser,
      [`--shard=${shard}`],
      shardWeights[browser],
    );
    for (const id of population(selected)) {
      if (!full.has(id)) fail(`${label} selects a test outside ${browser}`);
      const prior = owner.get(id);
      if (prior !== undefined) fail(`${label} and ${prior} both select ${id}`);
      owner.set(id, label);
    }
  }
  if (owner.size !== full.size)
    fail(`${browser} shards miss ${full.size - owner.size} tests`);
  console.log(
    `E2E census contract passed: ${full.size} ${browser} tests each selected once across ${shards.length} shards`,
  );
}
