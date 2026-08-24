import fs from "node:fs";
import path from "node:path";
import type { TestInfo } from "@playwright/test";
import type {
  FullConfig,
  FullResult,
  FullProject,
  Reporter,
  Suite,
  TestCase,
  TestResult,
} from "@playwright/test/reporter";

export const DURATION_BUDGET_ATTACHMENT = "duration-budget-effective-timeout";

export type DurationBudgetAttempt = {
  retry: number;
  effective_timeout_ms: number;
};

export type DurationBudgetTest = {
  test_id: string;
  project_id: string;
  project_name: string;
  title: string;
  file: string;
  line: number;
  attempts: DurationBudgetAttempt[];
};

export type DurationBudgetManifest = {
  schema_version: 1;
  complete: true;
  tests: DurationBudgetTest[];
};

type DurationBudgetAttachment = DurationBudgetAttempt & {
  test_id: string;
};

type DiscoveredTest = Omit<DurationBudgetTest, "attempts">;

/**
 * The fixture sends the final runtime timeout through Playwright's public
 * attachment channel because reporter and worker processes do not share memory.
 */
export async function attachEffectiveTimeout(
  testInfo: Pick<TestInfo, "attach" | "retry" | "testId" | "timeout">,
): Promise<void> {
  await testInfo.attach(DURATION_BUDGET_ATTACHMENT, {
    body: JSON.stringify({
      test_id: testInfo.testId,
      retry: testInfo.retry,
      effective_timeout_ms: testInfo.timeout,
    } satisfies DurationBudgetAttachment),
    contentType: "application/json",
  });
}

export class DurationBudgetManifestCollector {
  private readonly tests = new Map<string, DurationBudgetTest>();
  private readonly observedRetries = new Map<string, Set<number>>();
  private readonly recordedRetries = new Map<string, Set<number>>();
  private complete = true;
  constructor(discoveredTests: DiscoveredTest[]) {
    for (const discovered of discoveredTests) {
      if (this.tests.has(discovered.test_id)) {
        this.complete = false;
        continue;
      }
      this.tests.set(discovered.test_id, { ...discovered, attempts: [] });
      this.observedRetries.set(discovered.test_id, new Set());
      this.recordedRetries.set(discovered.test_id, new Set());
    }
  }

  invalidate(): void {
    this.complete = false;
  }

  observeAttempt(testId: string, retry: number): void {
    const observed = this.observedRetries.get(testId);
    if (observed === undefined || !Number.isInteger(retry) || retry < 0) {
      this.complete = false;
      return;
    }
    if (observed.has(retry)) {
      this.complete = false;
      return;
    }
    observed.add(retry);
  }

  recordBudget(attempt: DurationBudgetAttachment): void {
    const test = this.tests.get(attempt.test_id);
    const observed = this.observedRetries.get(attempt.test_id);
    const recorded = this.recordedRetries.get(attempt.test_id);
    if (
      test === undefined ||
      observed === undefined ||
      recorded === undefined ||
      !observed.has(attempt.retry) ||
      recorded.has(attempt.retry) ||
      !Number.isInteger(attempt.retry) ||
      attempt.retry < 0 ||
      !Number.isFinite(attempt.effective_timeout_ms) ||
      attempt.effective_timeout_ms <= 0
    ) {
      this.complete = false;
      return;
    }
    recorded.add(attempt.retry);
    test.attempts.push({
      retry: attempt.retry,
      effective_timeout_ms: attempt.effective_timeout_ms,
    });
  }

  manifest(): DurationBudgetManifest | undefined {
    if (!this.complete) return undefined;
    for (const [testId, test] of this.tests) {
      const observed = this.observedRetries.get(testId);
      const recorded = this.recordedRetries.get(testId);
      if (
        observed === undefined ||
        recorded === undefined ||
        observed.size === 0 ||
        observed.size !== recorded.size
      ) {
        return undefined;
      }
      test.attempts.sort((left, right) => left.retry - right.retry);
    }
    return {
      schema_version: 1,
      complete: true,
      tests: [...this.tests.values()],
    };
  }
}

// FullProject exposes names, while the JSON reporter assigns collision-safe IDs
// in resolved-project order; deriving them here keeps both artifacts joinable.
function resolvedProjectIds(config: FullConfig): Map<FullProject, string> {
  const projectIds = new Map<FullProject, string>();
  const assigned = new Set<string>();
  for (const project of config.projects) {
    for (let suffix = 0; ; suffix += 1) {
      const id = `${project.name}${suffix === 0 ? "" : suffix}`;
      if (assigned.has(id)) continue;
      assigned.add(id);
      projectIds.set(project, id);
      break;
    }
  }
  return projectIds;
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

function attachmentPayload(
  attachment: TestResult["attachments"][number],
): DurationBudgetAttachment | undefined {
  try {
    const raw =
      attachment.body?.toString("utf8") ??
      (attachment.path === undefined
        ? undefined
        : fs.readFileSync(attachment.path, "utf8"));
    if (raw === undefined) return undefined;
    const parsed: unknown = JSON.parse(raw);
    if (
      typeof parsed !== "object" ||
      parsed === null ||
      !Object.prototype.hasOwnProperty.call(parsed, "test_id") ||
      !Object.prototype.hasOwnProperty.call(parsed, "retry") ||
      !Object.prototype.hasOwnProperty.call(parsed, "effective_timeout_ms")
    ) {
      return undefined;
    }
    const value = parsed as Record<string, unknown>;
    if (
      typeof value.test_id !== "string" ||
      typeof value.retry !== "number" ||
      typeof value.effective_timeout_ms !== "number"
    ) {
      return undefined;
    }
    return {
      test_id: value.test_id,
      retry: value.retry,
      effective_timeout_ms: value.effective_timeout_ms,
    };
  } catch {
    return undefined;
  }
}

function manifestPath(config: FullConfig, outputFile: string): string {
  const configDirectory = config.configFile
    ? path.dirname(config.configFile)
    : process.cwd();
  return path.resolve(configDirectory, outputFile);
}

export default class DurationBudgetManifestReporter implements Reporter {
  private readonly outputFile: string;
  private collector: DurationBudgetManifestCollector | undefined;
  private outputPath: string | undefined;

  constructor(options: { outputFile?: string } = {}) {
    this.outputFile =
      options.outputFile ?? "test-results/duration-budget-manifest.json";
  }

  onBegin(config: FullConfig, suite: Suite): void {
    this.outputPath = manifestPath(config, this.outputFile);
    // An interrupted run must never leave a previous complete manifest behind.
    fs.rmSync(this.outputPath, { force: true });

    const projectIds = resolvedProjectIds(config);
    let discoveryComplete = true;
    const discovered = suite.allTests().map((test): DiscoveredTest => {
      const project = projectFor(test);
      const projectId =
        project === undefined ? undefined : projectIds.get(project);
      const file = path
        .relative(config.rootDir, test.location.file)
        .split(path.sep)
        .join("/");
      if (project === undefined || projectId === undefined) {
        discoveryComplete = false;
        return {
          test_id: test.id,
          project_id: "",
          project_name: "",
          title: test.title,
          file,
          line: test.location.line,
        };
      }
      return {
        test_id: test.id,
        project_id: projectId,
        project_name: project.name,
        title: test.title,
        file,
        line: test.location.line,
      };
    });
    this.collector = new DurationBudgetManifestCollector(discovered);
    if (!discoveryComplete) this.collector.invalidate();
  }

  onTestEnd(test: TestCase, result: TestResult): void {
    const collector = this.collector;
    if (collector === undefined) return;
    collector.observeAttempt(test.id, result.retry);

    const attachments = result.attachments.filter(
      (attachment) => attachment.name === DURATION_BUDGET_ATTACHMENT,
    );
    if (attachments.length !== 1) {
      collector.recordBudget({
        test_id: test.id,
        retry: result.retry,
        effective_timeout_ms: 0,
      });
      return;
    }
    const attempt = attachmentPayload(attachments[0]);
    if (
      attempt === undefined ||
      attempt.test_id !== test.id ||
      attempt.retry !== result.retry
    ) {
      collector.recordBudget({
        test_id: test.id,
        retry: result.retry,
        effective_timeout_ms: 0,
      });
      return;
    }
    collector.recordBudget(attempt);
  }

  onEnd(_result: FullResult): void {
    const manifest = this.collector?.manifest();
    if (manifest === undefined || this.outputPath === undefined) return;

    const directory = path.dirname(this.outputPath);
    fs.mkdirSync(directory, { recursive: true });
    const temporaryPath = path.join(
      directory,
      `.${path.basename(this.outputPath)}.${process.pid}.tmp`,
    );
    try {
      fs.writeFileSync(temporaryPath, `${JSON.stringify(manifest, null, 2)}\n`);
      fs.renameSync(temporaryPath, this.outputPath);
    } finally {
      fs.rmSync(temporaryPath, { force: true });
    }
  }
}
