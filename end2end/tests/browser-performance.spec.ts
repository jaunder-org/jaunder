import { resolve } from "node:path";
import { nonBrowserTest as test, expect } from "./fixtures";
import {
  PERFORMANCE_SAMPLE_COUNT,
  browserDiagnosticArtifacts,
  browserWorkloads,
  performanceEnabled,
  readPerformanceEnvironment,
  summarize,
  validateManifest,
} from "./browser-performance";

const manifest = {
  schema_version: 1,
  plan: { profile: "small", generator: { version: 1, seed: 1434 } },
  subjects: {
    username: "performance",
    history_post_id: 42,
    revision_id: 99,
    browser_initial_rows: {
      home: 50,
      app: 10,
      global_history: 100,
      post_history: 33,
    },
  },
  cursors: [
    "public_timeline",
    "authenticated_timeline",
    "owner_history",
    "post_history",
  ].map((workload) => ({
    workload,
    target_percent: 80,
    matching_result_count: 100,
    resolved_rank: 80,
    cursor: {
      kind: workload.includes("history") ? "history" : "timeline",
      value: workload.includes("history")
        ? { revision_id: 99 }
        : { created_at_us: 1, post_id: 42 },
    },
  })),
};

const environment = {
  JAUNDER_PERF_MANIFEST_PATH: "/tmp/dataset-manifest-v1.json",
  JAUNDER_PERF_FRAGMENT_DIR: "/tmp/fragments",
  JAUNDER_PERF_BACKEND: "sqlite",
  JAUNDER_PERF_BROWSER: "chromium",
  JAUNDER_PERF_BUILD_MODE: "release",
  JAUNDER_PERF_SETUP_JSON: JSON.stringify({
    provisioning_us: 1,
    seeding_us: 2,
  }),
  JAUNDER_PERF_IDENTITY_JSON: JSON.stringify({
    nix_system: "x86_64-linux",
    stable_derivation_identities: [
      { name: "server", identity: "sha256-server" },
    ],
    runner_image: "nixos",
    runner_architecture: "x86_64",
    cpu_model: "test-cpu",
    database_version: "sqlite",
    browser_version: "chromium",
  }),
};

test("plans the six canonical browser measurements from authoritative cursors", () => {
  const workloads = browserWorkloads(validateManifest(manifest));
  expect(workloads).toHaveLength(6);
  expect(
    workloads.map(({ workload, position }) => `${workload}:${position}`),
  ).toEqual([
    "home:initial",
    "app:initial",
    "global_history:initial",
    "global_history:deep",
    "browser_post_history:initial",
    "browser_revision_detail:point",
  ]);
  expect(PERFORMANCE_SAMPLE_COUNT).toBe(20);
});

test("maps browser diagnostic references beneath the configured fragment directory", () => {
  const fragmentDirectory = "/var/lib/jaunder/performance-fragments";
  const workload = browserWorkloads(validateManifest(manifest))[3]!;
  const artifacts = browserDiagnosticArtifacts(fragmentDirectory, workload, 7);

  expect(artifacts).toEqual({
    navigation: {
      destination:
        "/var/lib/jaunder/performance-fragments/diagnostics/global_history-deep-7.navigation.json",
      reference: "diagnostics/global_history-deep-7.navigation.json",
    },
    trace: {
      destination:
        "/var/lib/jaunder/performance-fragments/diagnostics/global_history-deep-7.zip",
      reference: "diagnostics/global_history-deep-7.zip",
    },
  });
  for (const artifact of Object.values(artifacts)) {
    expect(artifact.reference.trim()).not.toBe("");
    expect(artifact.reference.startsWith("/")).toBe(false);
    expect(resolve(fragmentDirectory, artifact.reference)).toBe(
      artifact.destination,
    );
  }
});

test("computes shared midpoint median and nearest-rank p95", () => {
  expect(summarize([1, 2, 4, 100])).toEqual({
    sample_count: 4,
    minimum_us: 1,
    maximum_us: 100,
    mean_us: 26,
    median_us: 3,
    p95_us: 100,
  });
});

test("rejects malformed performance environment and manifest", () => {
  expect(() =>
    readPerformanceEnvironment({
      ...environment,
      JAUNDER_PERF_BUILD_MODE: "debug",
    }),
  ).toThrow("must be release");
  expect(() =>
    readPerformanceEnvironment({
      ...environment,
      JAUNDER_PERF_FRAGMENT_DIR: "fragments",
    }),
  ).toThrow("absolute path");
  expect(() =>
    performanceEnabled({ JAUNDER_PERF_MANIFEST_PATH: "manifest" }),
  ).toThrow("incomplete");
  const reversedIdentities = {
    ...environment,
    JAUNDER_PERF_IDENTITY_JSON: JSON.stringify({
      ...JSON.parse(environment.JAUNDER_PERF_IDENTITY_JSON),
      stable_derivation_identities: [
        { name: "server", identity: "server" },
        { name: "client", identity: "client" },
      ],
    }),
  };
  expect(() => readPerformanceEnvironment(reversedIdentities)).toThrow(
    "sorted order",
  );
  expect(() => validateManifest({ ...manifest, schema_version: 2 })).toThrow(
    "schema_version",
  );
  expect(() =>
    validateManifest({
      ...manifest,
      subjects: {
        ...manifest.subjects,
        browser_initial_rows: {
          ...manifest.subjects.browser_initial_rows,
          app: -1,
        },
      },
    }),
  ).toThrow("browser_initial_rows.app");
  expect(() =>
    browserWorkloads(validateManifest({ ...manifest, cursors: [] })),
  ).toThrow("owner_history");
});
