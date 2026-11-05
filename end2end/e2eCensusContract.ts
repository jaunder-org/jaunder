import { readFileSync, rmSync } from "node:fs";
import { spawnSync } from "node:child_process";
import path from "node:path";

type CensusTest = {
  file: string;
  line: number;
  column: number;
  title_path: string[];
};
type Census = { schema_version: 1; complete: true; tests: CensusTest[] };
const root = path.dirname(path.resolve(process.argv[1]));
const playwrightCli =
  process.env.JAUNDER_E2E_PLAYWRIGHT_CLI ?? "node_modules/.bin/playwright";
const outputs = [
  "test-results/unsplit-firefox-census.json",
  "test-results/experimental-firefox-census.json",
];
function fail(message: string): never {
  throw new Error(`E2E census contract: ${message}`);
}
function run(output: string, projects: string[]): Census {
  rmSync(path.join(root, output), { force: true });
  const result = spawnSync(
    process.execPath,
    [
      playwrightCli,
      "test",
      "--list",
      "--config",
      "playwright.config.ts",
      ...projects.flatMap((project) => ["--project", project]),
    ],
    {
      cwd: root,
      env: {
        ...process.env,
        JAUNDER_E2E_CENSUS_PREPARE: "1",
        JAUNDER_E2E_CENSUS_TOPOLOGY: "firefox-contract",
        JAUNDER_E2E_CENSUS_OUTPUT: output,
      },
      encoding: "utf8",
    },
  );
  if (result.status !== 0) fail(`Playwright --list failed: ${result.stderr}`);
  return JSON.parse(readFileSync(path.join(root, output), "utf8")) as Census;
}
function canonical(test: CensusTest): string {
  if (
    !test.file ||
    !Number.isInteger(test.line) ||
    !Number.isInteger(test.column) ||
    !Array.isArray(test.title_path) ||
    test.title_path.length === 0 ||
    test.title_path.some((title) => !title)
  )
    fail(`malformed canonical source identity: ${JSON.stringify(test)}`);
  return JSON.stringify([test.file, test.line, test.column, test.title_path]);
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
  if (set.size !== values.length) fail("duplicate canonical source identity");
  return set;
}
const unsplit = population(
  run(outputs[0], [
    "firefox-visual",
    "firefox",
    "firefox-admin-site",
    "firefox-admin",
  ]),
);
const experimental = population(
  run(outputs[1], [
    "firefox-ordinary",
    "firefox-special-visual",
    "firefox-special-global-configuration",
    "firefox-special-invite",
  ]),
);
const missing = [...unsplit].filter((id) => !experimental.has(id));
const unexpected = [...experimental].filter((id) => !unsplit.has(id));
if (missing.length || unexpected.length)
  fail(
    `reciprocal selection mismatch: missing=${missing.length}, unexpected=${unexpected.length}`,
  );
console.log(
  `E2E census contract passed: ${unsplit.size} canonical Firefox tests`,
);
