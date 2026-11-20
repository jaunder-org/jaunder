import { readFileSync, rmSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { createRequire } from "node:module";
import path from "node:path";
import type { Topology } from "./e2eProjects";

type CensusTest = {
  file: string;
  line: number;
  column: number;
  title_path: string[];
};
type Census = { schema_version: 1; complete: true; tests: CensusTest[] };
const root = path.dirname(path.resolve(process.argv[1]));
// This script runs under plain `node` (type stripping), whose resolver needs
// the `.ts` extension that this tsconfig does not let an import path carry.
const { gatedBrowsers, projectName, roles } = createRequire(
  path.resolve(process.argv[1]),
)("./e2eProjects.ts") as typeof import("./e2eProjects");
const playwrightCli =
  process.env.JAUNDER_E2E_PLAYWRIGHT_CLI ?? "node_modules/.bin/playwright";
function fail(message: string): never {
  throw new Error(`E2E census contract: ${message}`);
}
function run(browser: string, output: string, projects: string[]): Census {
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
        JAUNDER_E2E_CENSUS_TOPOLOGY: `${browser}-contract`,
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
// Each browser's split lane projects must select exactly its unsplit graph.
for (const browser of gatedBrowsers) {
  const projects = (topology: Topology) =>
    roles.map((role) => projectName(browser, topology, role));
  const unsplit = population(
    run(
      browser,
      `test-results/unsplit-${browser}-census.json`,
      projects("unsplit"),
    ),
  );
  const split = population(
    run(
      browser,
      `test-results/split-${browser}-census.json`,
      projects("split"),
    ),
  );
  const missing = [...unsplit].filter((id) => !split.has(id));
  const unexpected = [...split].filter((id) => !unsplit.has(id));
  if (missing.length || unexpected.length)
    fail(
      `${browser} reciprocal selection mismatch: missing=${missing.length}, unexpected=${unexpected.length}`,
    );
  console.log(
    `E2E census contract passed: ${unsplit.size} canonical ${browser} tests`,
  );
}
