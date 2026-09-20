import fs from "node:fs";
import path from "node:path";
import type {
  FullConfig,
  FullProject,
  Reporter,
  Suite,
  TestCase,
} from "@playwright/test/reporter";

type Selected = { project_id: string; project_name: string; test_id: string };
type LaneManifest = {
  schema_version: 1;
  complete: true;
  lane: string;
  backend: string;
  browser: string;
  partition: string;
  shard_index: number | null;
  shard_count: number | null;
  tests: Selected[];
};
function projectIds(config: FullConfig): Map<FullProject, string> {
  const ids = new Map<FullProject, string>();
  const used = new Set<string>();
  for (const project of config.projects)
    for (let suffix = 0; ; suffix += 1) {
      const id = `${project.name}${suffix === 0 ? "" : suffix}`;
      if (!used.has(id)) {
        used.add(id);
        ids.set(project, id);
        break;
      }
    }
  return ids;
}
function projectFor(test: TestCase): FullProject | undefined {
  for (
    let suite: Suite | undefined = test.parent;
    suite !== undefined;
    suite = suite.parent
  ) {
    const project = suite.project();
    if (project !== undefined) return project;
  }
  return undefined;
}
function required(name: string): string {
  const value = process.env[name];
  if (!value) throw new Error(`E2E lane manifest is missing ${name}`);
  return value;
}
function optionalNumber(name: string): number | null {
  const value = process.env[name];
  if (value === undefined || value === "") return null;
  const parsed = Number(value);
  if (!Number.isInteger(parsed) || parsed < 1)
    throw new Error(`E2E lane manifest has invalid ${name}`);
  return parsed;
}
/** Writes one fresh, lane-authenticated selected population before execution. */
export default class E2eLaneManifestReporter implements Reporter {
  onBegin(config: FullConfig, suite: Suite): void {
    const output = path.resolve(
      config.configFile ? path.dirname(config.configFile) : process.cwd(),
      "test-results/e2e-lane-manifest.json",
    );
    fs.rmSync(output, { force: true });
    const shard_index = optionalNumber("JAUNDER_E2E_SHARD_INDEX");
    const shard_count = optionalNumber("JAUNDER_E2E_SHARD_COUNT");
    if (
      (shard_index === null) !== (shard_count === null) ||
      (shard_index !== null && shard_index > shard_count!)
    )
      throw new Error("E2E lane manifest has invalid shard metadata");
    const ids = projectIds(config);
    const tests = suite.allTests().map((test) => {
      const project = projectFor(test);
      const project_id = project === undefined ? undefined : ids.get(project);
      if (project === undefined || project_id === undefined)
        throw new Error("E2E lane manifest cannot resolve selected project");
      return { project_id, project_name: project.name, test_id: test.id };
    });
    if (tests.length === 0)
      throw new Error("E2E lane manifest has no selected tests");
    const seen = new Set<string>();
    for (const test of tests) {
      const key = `${test.project_id}\0${test.project_name}\0${test.test_id}`;
      if (seen.has(key))
        throw new Error(`E2E lane manifest has duplicate identity ${key}`);
      seen.add(key);
    }
    tests.sort((a, b) =>
      `${a.project_id}\0${a.project_name}\0${a.test_id}`.localeCompare(
        `${b.project_id}\0${b.project_name}\0${b.test_id}`,
      ),
    );
    fs.mkdirSync(path.dirname(output), { recursive: true });
    const manifest: LaneManifest = {
      schema_version: 1,
      complete: true,
      lane: required("JAUNDER_E2E_LANE"),
      backend: required("JAUNDER_E2E_BACKEND"),
      browser: required("JAUNDER_E2E_BROWSER"),
      partition: required("JAUNDER_E2E_PARTITION"),
      shard_index,
      shard_count,
      tests,
    };
    fs.writeFileSync(output, `${JSON.stringify(manifest)}\n`);
  }
}
