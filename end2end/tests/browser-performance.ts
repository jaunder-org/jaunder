import { mkdir, rename, writeFile } from "node:fs/promises";
import { basename, dirname, join } from "node:path";

export const PERFORMANCE_PAGE_SIZE = 50;
export const PERFORMANCE_SAMPLE_COUNT = 20;

export type BrowserName = "chromium" | "firefox";
export type BackendName = "sqlite" | "postgres";
export type Position = "initial" | "deep" | "point";
export type WorkloadName =
  | "home"
  | "app"
  | "global_history"
  | "browser_post_history"
  | "browser_revision_detail";

export type PersistedCursor =
  | { kind: "timeline"; value: { created_at_us: number; post_id: number } }
  | { kind: "history"; value: { revision_id: number } };

export type Cursor = {
  workload:
    | "public_timeline"
    | "authenticated_timeline"
    | "owner_history"
    | "post_history";
  target_percent: number;
  matching_result_count: number;
  resolved_rank: number;
  cursor: PersistedCursor;
};

export type DatasetManifest = {
  schema_version: number;
  plan: {
    profile: "small" | "medium" | "large";
    generator: { version: number; seed: number };
  };
  subjects: {
    username: string;
    history_post_id: number;
    revision_id: number;
    browser_initial_rows: {
      home: number;
      app: number;
      global_history: number;
      post_history: number;
    };
  };
  cursors: Cursor[];
};

type RequiredEnvironment = {
  manifestPath: string;
  fragmentDirectory: string;
  backend: BackendName;
  browser: BrowserName;
  setup: { provisioning_us: number; seeding_us: number };
  identity: {
    nix_system: string;
    stable_derivation_identities: Array<{ name: string; identity: string }>;
    runner_image: string;
    runner_architecture: string;
    cpu_model: string;
    database_version: string;
    browser_version: string;
  };
};

export type BrowserWorkload = {
  workload: WorkloadName;
  position: Position;
  path: string;
  cursor: Cursor | undefined;
  pageSize: number | undefined;
};

const REQUIRED = [
  "JAUNDER_PERF_MANIFEST_PATH",
  "JAUNDER_PERF_FRAGMENT_DIR",
  "JAUNDER_PERF_BACKEND",
  "JAUNDER_PERF_BROWSER",
  "JAUNDER_PERF_SETUP_JSON",
  "JAUNDER_PERF_IDENTITY_JSON",
  "JAUNDER_PERF_BUILD_MODE",
] as const;

function required(env: NodeJS.ProcessEnv, name: string): string {
  const value = env[name]?.trim();
  if (!value)
    throw new Error(`${name} must be set for browser performance measurements`);
  return value;
}

function json<T>(value: string, name: string): T {
  try {
    return JSON.parse(value) as T;
  } catch {
    throw new Error(`${name} must be valid JSON`);
  }
}

function positiveInteger(value: unknown, name: string): number {
  if (typeof value !== "number" || !Number.isSafeInteger(value) || value < 0) {
    throw new Error(`${name} must be a non-negative safe integer`);
  }
  return value;
}

function positiveSubjectInteger(value: unknown, name: string): number {
  const number = positiveInteger(value, name);
  if (number === 0) throw new Error(`${name} must be positive`);
  return number;
}

function nonempty(value: unknown, name: string): string {
  if (typeof value !== "string" || value.trim() === "")
    throw new Error(`${name} must be nonempty`);
  return value;
}

export function performanceEnabled(env = process.env): boolean {
  const present = REQUIRED.filter((name) => Boolean(env[name]?.trim()));
  if (present.length === 0) return false;
  if (present.length !== REQUIRED.length) {
    throw new Error(
      `incomplete browser performance environment: missing ${REQUIRED.filter((name) => !env[name]?.trim()).join(", ")}`,
    );
  }
  return true;
}

export function readPerformanceEnvironment(
  env = process.env,
): RequiredEnvironment {
  for (const name of REQUIRED) required(env, name);

  if (required(env, "JAUNDER_PERF_BUILD_MODE") !== "release") {
    throw new Error("JAUNDER_PERF_BUILD_MODE must be release");
  }
  const backend = required(env, "JAUNDER_PERF_BACKEND");
  if (backend !== "sqlite" && backend !== "postgres")
    throw new Error("JAUNDER_PERF_BACKEND must be sqlite or postgres");
  const browser = required(env, "JAUNDER_PERF_BROWSER");
  if (browser !== "chromium" && browser !== "firefox")
    throw new Error("JAUNDER_PERF_BROWSER must be chromium or firefox");
  const setup = json<{ provisioning_us: unknown; seeding_us: unknown }>(
    required(env, "JAUNDER_PERF_SETUP_JSON"),
    "JAUNDER_PERF_SETUP_JSON",
  );
  const identity = json<RequiredEnvironment["identity"]>(
    required(env, "JAUNDER_PERF_IDENTITY_JSON"),
    "JAUNDER_PERF_IDENTITY_JSON",
  );
  positiveInteger(setup.provisioning_us, "setup.provisioning_us");
  positiveInteger(setup.seeding_us, "setup.seeding_us");
  for (const field of [
    "nix_system",
    "runner_image",
    "runner_architecture",
    "cpu_model",
    "database_version",
    "browser_version",
  ] as const)
    nonempty(identity[field], `identity.${field}`);
  if (
    !Array.isArray(identity.stable_derivation_identities) ||
    identity.stable_derivation_identities.length === 0
  )
    throw new Error("identity.stable_derivation_identities must be nonempty");
  for (const derivation of identity.stable_derivation_identities) {
    nonempty(derivation.name, "derivation.name");
    nonempty(derivation.identity, "derivation.identity");
  }
  for (
    let index = 1;
    index < identity.stable_derivation_identities.length;
    index += 1
  ) {
    if (
      identity.stable_derivation_identities[index - 1]!.name >=
      identity.stable_derivation_identities[index]!.name
    ) {
      throw new Error(
        "identity.stable_derivation_identities must have unique names in sorted order",
      );
    }
  }
  return {
    manifestPath: required(env, "JAUNDER_PERF_MANIFEST_PATH"),
    fragmentDirectory: required(env, "JAUNDER_PERF_FRAGMENT_DIR"),
    backend,
    browser,
    setup: {
      provisioning_us: positiveInteger(
        setup.provisioning_us,
        "setup.provisioning_us",
      ),
      seeding_us: positiveInteger(setup.seeding_us, "setup.seeding_us"),
    },
    identity,
  };
}

function cursor(
  manifest: DatasetManifest,
  workload: Cursor["workload"],
): Cursor {
  const found = manifest.cursors.find(
    (candidate) => candidate.workload === workload,
  );
  if (!found) throw new Error(`manifest lacks cursor for ${workload}`);
  if (
    found.target_percent !== 80 ||
    found.resolved_rank < 1 ||
    found.matching_result_count < PERFORMANCE_PAGE_SIZE
  )
    throw new Error(`manifest cursor for ${workload} is invalid`);
  return found;
}

export function validateManifest(value: unknown): DatasetManifest {
  if (typeof value !== "object" || value === null)
    throw new Error("manifest must be an object");
  const manifest = value as DatasetManifest;
  if (manifest.schema_version !== 1)
    throw new Error("manifest schema_version must be 1");
  nonempty(manifest.plan?.profile, "manifest.plan.profile");
  positiveInteger(
    manifest.plan?.generator?.version,
    "manifest.plan.generator.version",
  );
  positiveInteger(
    manifest.plan?.generator?.seed,
    "manifest.plan.generator.seed",
  );
  const initialRows = manifest.subjects?.browser_initial_rows;
  for (const route of [
    "home",
    "app",
    "global_history",
    "post_history",
  ] as const) {
    positiveSubjectInteger(
      initialRows?.[route],
      `manifest.subjects.browser_initial_rows.${route}`,
    );
  }
  positiveSubjectInteger(
    manifest.subjects?.history_post_id,
    "manifest.subjects.history_post_id",
  );
  positiveSubjectInteger(
    manifest.subjects?.revision_id,
    "manifest.subjects.revision_id",
  );
  if (!Array.isArray(manifest.cursors))
    throw new Error("manifest.cursors must be an array");
  return manifest;
}

export function browserWorkloads(manifest: DatasetManifest): BrowserWorkload[] {
  const ownerHistory = cursor(manifest, "owner_history");
  const postHistoryPath = `/posts/${manifest.subjects.history_post_id}/history`;
  return [
    {
      workload: "home",
      position: "initial",
      path: "/",
      cursor: undefined,
      pageSize: PERFORMANCE_PAGE_SIZE,
    },
    {
      workload: "app",
      position: "initial",
      path: "/app",
      cursor: undefined,
      pageSize: PERFORMANCE_PAGE_SIZE,
    },
    {
      workload: "global_history",
      position: "initial",
      path: "/history",
      cursor: undefined,
      pageSize: PERFORMANCE_PAGE_SIZE,
    },
    {
      workload: "global_history",
      position: "deep",
      path: "/history",
      cursor: ownerHistory,
      pageSize: PERFORMANCE_PAGE_SIZE,
    },
    {
      workload: "browser_post_history",
      position: "initial",
      path: postHistoryPath,
      cursor: undefined,
      pageSize: PERFORMANCE_PAGE_SIZE,
    },
    {
      workload: "browser_revision_detail",
      position: "point",
      path: `${postHistoryPath}/${manifest.subjects.revision_id}`,
      cursor: undefined,
      pageSize: undefined,
    },
  ];
}

export function summarize(samples: readonly number[]) {
  if (
    samples.length === 0 ||
    samples.some((sample) => !Number.isSafeInteger(sample) || sample < 0)
  )
    throw new Error("samples must be nonempty integer microseconds");
  const sorted = [...samples].sort((left, right) => left - right);
  const total = sorted.reduce((sum, sample) => sum + sample, 0);
  const middle = Math.floor(sorted.length / 2);
  return {
    sample_count: sorted.length,
    minimum_us: sorted[0]!,
    maximum_us: sorted.at(-1)!,
    mean_us: Math.floor(total / sorted.length),
    median_us:
      sorted.length % 2 === 0
        ? Math.floor((sorted[middle - 1]! + sorted[middle]!) / 2)
        : sorted[middle]!,
    p95_us: sorted[Math.ceil(sorted.length * 0.95) - 1]!,
  };
}

export async function atomicWriteJson(
  path: string,
  value: unknown,
): Promise<void> {
  await mkdir(dirname(path), { recursive: true });
  const temporary = join(
    dirname(path),
    `.${basename(path)}.${process.pid}.${crypto.randomUUID()}.tmp`,
  );
  await writeFile(temporary, `${JSON.stringify(value)}\n`, "utf8");
  await rename(temporary, path);
}
