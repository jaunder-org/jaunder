import fs from "node:fs";
import path from "node:path";
import type {
  FullConfig,
  FullProject,
  Reporter,
  Suite,
  TestCase,
} from "@playwright/test/reporter";

export type E2eCensusTest = {
  project_id: string;
  project_name: string;
  test_id: string;
  file: string;
  line: number;
  column: number;
  title_path: string[];
};
export type E2eCensus = {
  schema_version: 1;
  complete: true;
  topology: string;
  tests: E2eCensusTest[];
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
function sourceTitlePath(test: TestCase, project: FullProject): string[] {
  const titles = test.titlePath().filter((title) => title.length > 0);
  const projectIndex = titles.indexOf(project.name);
  if (projectIndex !== -1) titles.splice(projectIndex, 1);
  return titles;
}
/** Writes the complete unsharded candidate population during a `--list` preflight. */
export default class E2eCensusReporter implements Reporter {
  private readonly outputFile: string;
  constructor(options: { outputFile?: string } = {}) {
    this.outputFile =
      options.outputFile ?? "test-results/e2e-expected-census.json";
    if (
      path.isAbsolute(this.outputFile) ||
      !this.outputFile.startsWith("test-results/")
    )
      throw new Error("E2E census output must be a relative test-results path");
  }
  onBegin(config: FullConfig, suite: Suite): void {
    const output = path.resolve(
      config.configFile ? path.dirname(config.configFile) : process.cwd(),
      this.outputFile,
    );
    fs.rmSync(output, { force: true });
    const topology = process.env.JAUNDER_E2E_CENSUS_TOPOLOGY;
    const ids = projectIds(config);
    const tests = suite.allTests().map((test) => {
      const project = projectFor(test);
      const projectId = project === undefined ? undefined : ids.get(project);
      if (project === undefined || projectId === undefined)
        throw new Error("E2E census cannot resolve a selected test project");
      return {
        project_id: projectId,
        project_name: project.name,
        test_id: test.id,
        file: path
          .relative(config.rootDir, test.location.file)
          .split(path.sep)
          .join("/"),
        line: test.location.line,
        column: test.location.column,
        title_path: sourceTitlePath(test, project),
      };
    });
    if (!topology || tests.length === 0)
      throw new Error(
        "E2E expected census requires a topology and a nonempty selected population",
      );
    const seen = new Set<string>();
    for (const test of tests) {
      const key = `${test.project_id}\0${test.project_name}\0${test.test_id}`;
      if (seen.has(key))
        throw new Error(`E2E census has duplicate selected identity ${key}`);
      seen.add(key);
    }
    tests.sort((left, right) =>
      `${left.project_id}\0${left.project_name}\0${left.test_id}`.localeCompare(
        `${right.project_id}\0${right.project_name}\0${right.test_id}`,
      ),
    );
    fs.mkdirSync(path.dirname(output), { recursive: true });
    fs.writeFileSync(
      output,
      `${JSON.stringify({ schema_version: 1, complete: true, topology, tests } satisfies E2eCensus)}\n`,
    );
  }
}
