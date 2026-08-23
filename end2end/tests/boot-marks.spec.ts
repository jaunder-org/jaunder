// `test` comes from `./fixtures`, not `@playwright/test`, even though the cases below
// are pure and drive no server fn: the `traced-context` gate forbids the upstream
// import anywhere under `end2end/tests` (only `fixtures.ts` is exempt), because a spec
// that opens no `e2e.test` span makes everything it drives unattributable — and that
// under-reports SILENTLY. A blanket rule is the point; carving out "but this one is
// pure" is how the guard stops guarding. The assertions here are still pure, so the
// merge invariant is proven by their logic, not by any browser behavior (#818).
import { test, expect } from "./fixtures";
import {
  mergeDocumentTiming,
  wasmInitFromMarks,
  type DocumentTiming,
} from "./capture-trace";
import { navigationBridgeFieldsFrom } from "./fixtures";
import { goto } from "./helpers";

const wasm = {
  startTime: 10,
  durationMs: 5,
  responseEndMs: 15,
  decodedBodySize: 5_350_591,
  encodedBodySize: 862_755,
  transferSize: 863_000,
};
const marks = (count: number) =>
  Array.from({ length: count }, (_, index) => ({
    name: `jaunder.boot.m${index}`,
    startTime: index,
  }));

// `toBe` (identity), not `toEqual`: the merge PICKS a snapshot, it never builds a
// blended one. Copying would be a silent behavior change, so identity is the contract.
test.describe("mergeDocumentTiming", () => {
  test("takes the incoming snapshot when there is no existing one", () => {
    const incoming: DocumentTiming = {
      timeOriginMs: 10,
      marks: marks(4),
      wasm,
    };
    expect(mergeDocumentTiming(undefined, incoming)).toBe(incoming);
  });

  test("prefers the snapshot with more marks when it arrives second", () => {
    // The firefox ordering: `load` harvests an empty document first, mount-ready
    // harvests the full one after.
    const existing: DocumentTiming = {
      timeOriginMs: 10,
      marks: [],
      wasm: null,
    };
    const incoming: DocumentTiming = {
      timeOriginMs: 10,
      marks: marks(4),
      wasm,
    };
    expect(mergeDocumentTiming(existing, incoming)).toBe(incoming);
  });

  test("prefers the snapshot with more marks when it arrived first", () => {
    // The clobber this rule exists to prevent: a late-resolving `load` harvest must
    // not overwrite a complete mount-ready one.
    const existing: DocumentTiming = {
      timeOriginMs: 10,
      marks: marks(4),
      wasm,
    };
    const incoming: DocumentTiming = {
      timeOriginMs: 10,
      marks: [],
      wasm: null,
    };
    expect(mergeDocumentTiming(existing, incoming)).toBe(existing);
  });

  test("breaks a mark-count tie toward the incoming snapshot's wasm timing", () => {
    const existing: DocumentTiming = {
      timeOriginMs: 10,
      marks: marks(4),
      wasm: null,
    };
    const incoming: DocumentTiming = {
      timeOriginMs: 10,
      marks: marks(4),
      wasm,
    };
    expect(mergeDocumentTiming(existing, incoming)).toBe(incoming);
  });

  test("keeps the existing snapshot's wasm timing on a tie", () => {
    const existing: DocumentTiming = {
      timeOriginMs: 10,
      marks: marks(4),
      wasm,
    };
    const incoming: DocumentTiming = {
      timeOriginMs: 10,
      marks: marks(4),
      wasm: null,
    };
    expect(mergeDocumentTiming(existing, incoming)).toBe(existing);
  });

  test("keeps the existing snapshot when a tie gives neither side more", () => {
    const existing: DocumentTiming = {
      timeOriginMs: 10,
      marks: marks(4),
      wasm,
    };
    const incoming: DocumentTiming = {
      timeOriginMs: 10,
      marks: marks(4),
      wasm,
    };
    expect(mergeDocumentTiming(existing, incoming)).toBe(existing);
  });

  test("keeps the selected snapshot but fills a missing document epoch", () => {
    const existing: DocumentTiming = {
      timeOriginMs: null,
      marks: marks(4),
      wasm,
    };
    const incoming: DocumentTiming = {
      timeOriginMs: 25,
      marks: marks(4),
      wasm: null,
    };
    expect(mergeDocumentTiming(existing, incoming)).toEqual({
      ...existing,
      timeOriginMs: 25,
    });
  });
});

test("keeps a completion harvest that arrives after the fullest boot marks", () => {
  const bootSnapshot: DocumentTiming = {
    timeOriginMs: 10,
    marks: marks(4),
    wasm,
  };
  const completionSnapshot: DocumentTiming = {
    timeOriginMs: 10,
    marks: marks(4),
    wasm,
    wasmInit: {
      startMs: 10,
      doneMs: 30,
      apiMs: 12,
      path: "streaming",
      experimentArm: null,
      moduleShape: null,
    },
  };

  expect(mergeDocumentTiming(bootSnapshot, completionSnapshot)).toEqual(
    completionSnapshot,
  );
});

test("decodes initializer arm and module shape detail", () => {
  expect(
    wasmInitFromMarks([
      { name: "jaunder.wasm.init_start", startTime: 10 },
      {
        name: "jaunder.wasm.init_done",
        startTime: 30,
        detail: {
          path: "streaming",
          apiMs: 12,
          experimentArm: "shape",
          moduleShape: {
            imports: 2,
            importedFunctions: 1,
            importedTables: 0,
            importedMemories: 1,
            exports: 3,
            exportedFunctions: 1,
            exportedTables: 1,
            exportedMemories: 1,
            customSections: 1,
          },
        },
      },
    ]),
  ).toEqual({
    startMs: 10,
    doneMs: 30,
    apiMs: 12,
    path: "streaming",
    experimentArm: "shape",
    moduleShape: {
      imports: 2,
      importedFunctions: 1,
      importedTables: 0,
      importedMemories: 1,
      exports: 3,
      exportedFunctions: 1,
      exportedTables: 1,
      exportedMemories: 1,
      customSections: 1,
    },
  });
});

test("rejects malformed initializer completion detail", () => {
  expect(
    wasmInitFromMarks([
      { name: "jaunder.wasm.init_start", startTime: 10 },
      {
        name: "jaunder.wasm.init_done",
        startTime: 30,
        detail: { path: "streaming", apiMs: Number.NaN },
      },
    ]),
  ).toEqual({
    startMs: 10,
    doneMs: 30,
    apiMs: null,
    path: null,
    experimentArm: null,
    moduleShape: null,
  });
  expect(
    wasmInitFromMarks([
      { name: "jaunder.wasm.init_start", startTime: 10 },
      {
        name: "jaunder.wasm.init_done",
        startTime: 30,
        detail: { path: "buffered", apiMs: -1 },
      },
    ]),
  ).toEqual({
    startMs: 10,
    doneMs: 30,
    apiMs: null,
    path: null,
    experimentArm: null,
    moduleShape: null,
  });
});

test.describe("navigationBridgeFieldsFrom", () => {
  const timingWithMountDone: DocumentTiming = {
    timeOriginMs: 1_000,
    marks: [
      { name: "jaunder.boot.entry", startTime: 5 },
      { name: "jaunder.boot.mount_done", startTime: 40 },
    ],
    wasm: null,
  };

  test("reports complete bridge diagnostics when all inputs exist", () => {
    expect(
      navigationBridgeFieldsFrom(
        { committedMs: 900, mountedMs: 1_060 },
        timingWithMountDone,
      ),
    ).toEqual({
      frameSkewSchema: "bridge-v1",
      documentTimeOriginMs: 1_000,
      documentBootTotalMs: 40,
      commitToDocumentStartMs: 100,
      mountDoneToBindingMs: 20,
      frameSkewRemainderMs: 0,
    });
  });

  test("returns all-null bridge diagnostics when any required input is missing", () => {
    expect(
      navigationBridgeFieldsFrom(
        { committedMs: null, mountedMs: 1_060 },
        timingWithMountDone,
      ),
    ).toEqual({
      frameSkewSchema: null,
      documentTimeOriginMs: null,
      documentBootTotalMs: null,
      commitToDocumentStartMs: null,
      mountDoneToBindingMs: null,
      frameSkewRemainderMs: null,
    });
    expect(
      navigationBridgeFieldsFrom(
        { committedMs: 900, mountedMs: 1_060 },
        { ...timingWithMountDone, marks: [] },
      ),
    ).toEqual({
      frameSkewSchema: null,
      documentTimeOriginMs: null,
      documentBootTotalMs: null,
      commitToDocumentStartMs: null,
      mountDoneToBindingMs: null,
      frameSkewRemainderMs: null,
    });
  });

  test("returns all-null bridge diagnostics when timing inputs are malformed", () => {
    expect(
      navigationBridgeFieldsFrom(
        { committedMs: 900, mountedMs: 1_060 },
        { ...timingWithMountDone, timeOriginMs: Number.NaN },
      ),
    ).toEqual({
      frameSkewSchema: null,
      documentTimeOriginMs: null,
      documentBootTotalMs: null,
      commitToDocumentStartMs: null,
      mountDoneToBindingMs: null,
      frameSkewRemainderMs: null,
    });
    expect(
      navigationBridgeFieldsFrom(
        { committedMs: 900, mountedMs: 1_060 },
        {
          ...timingWithMountDone,
          marks: [{ name: "jaunder.boot.mount_done", startTime: Number.NaN }],
        },
      ),
    ).toEqual({
      frameSkewSchema: null,
      documentTimeOriginMs: null,
      documentBootTotalMs: null,
      commitToDocumentStartMs: null,
      mountDoneToBindingMs: null,
      frameSkewRemainderMs: null,
    });
  });
});

// The regression guard for #818's actual defect. Unthresholded on purpose: it
// asserts the mechanism works at all, which needs no knowledge of the coverage
// distribution and so can ship before the distribution exists. Gradual erosion is
// #831's job.
test("boot fetches the wasm once and the harness captures the full mark set", async ({
  page,
  bootTiming,
}) => {
  // Piggy-backed on this navigation rather than given its own `test()` (#866).
  // Boot cost is per-navigation and the suite already runs 211 of them for 137
  // tests (#867), so a spec that exists only to navigate makes the thing we are
  // trying to measure worse.
  //
  // Counts by PATHNAME: the server content-negotiates `.br`/`.gz` against the
  // same URL and answers conditional requests with `304`, so encoding and status
  // are both the wrong discriminator — a variant or a revalidation is still one
  // fetch of one resource.
  //
  // Cache state is COLD: each test gets a fresh `browser.newContext()`, so the
  // HTTP cache starts empty and this is the uncached path. The warm path is not
  // exercised here.
  const wasmRequests: string[] = [];
  page.on("request", (request) => {
    if (new URL(request.url()).pathname === "/pkg/jaunder.wasm") {
      wasmRequests.push(request.url());
    }
  });

  // `goto` awaits `waitForMount` itself, so the mount binding has already fired
  // and its harvest is either done or in flight — which is what `bootTiming`'s
  // `settle()` covers.
  await goto(page, "/");

  // The failure mode this exists for is silent (#866): a preload whose request
  // mode differs from wasm-bindgen's fetch can download the bundle twice.
  expect(
    wasmRequests.length,
    `expected exactly one /pkg/jaunder.wasm request, got ${wasmRequests.length}: ${wasmRequests.join(", ")}`,
  ).toBe(1);

  // `init_done` is emitted after the fire-and-forget initializer's promise
  // resolves. Wait for that specific mark, not network idle, before settlement.
  await page.waitForFunction(
    (name) => performance.getEntriesByName(name, "mark").length !== 0,
    "jaunder.wasm.init_done",
  );
  const timing = await bootTiming();
  expect(
    timing,
    "no document timing was harvested for the mounted navigation",
  ).toBeDefined();

  expect(timing?.timeOriginMs).toEqual(expect.any(Number));

  // Assert the SHAPE, never the names: mark names live only in Rust and are
  // discovered by prefix, so enumerating them here would reintroduce exactly the
  // cross-language drift `MOUNTED_ATTR` suffers (#794). `>=`, never `===`, so
  // adding a mark in `client::perf` extends the set instead of reddening the build.
  const names = (timing?.marks ?? []).map((mark) => mark.name);
  expect(names.length).toBeGreaterThanOrEqual(4);
  expect(names.every((name) => name.startsWith("jaunder."))).toBe(true);
  expect(new Set(names).size).toBe(names.length);

  // Resource timing remains a delivery diagnostic. Direct initializer marks
  // supply the independent initialization measurements below.
  expect(timing?.wasm ?? null).not.toBeNull();

  const completionDetail = await page.evaluate(() => {
    const entry = performance
      .getEntriesByType("mark")
      .find(
        (mark) => mark.name === "jaunder.wasm.init_done",
      ) as PerformanceMark;
    return entry.detail;
  });
  expect(completionDetail).toMatchObject({
    path: "streaming",
    moduleShape: {
      exports: expect.any(Number),
      exportedFunctions: expect.any(Number),
      imports: expect.any(Number),
      importedFunctions: expect.any(Number),
      customSections: expect.any(Number),
    },
  });
  expect([null, "baseline", "shape", "shape-many", "shape-count"]).toContain(
    completionDetail.experimentArm,
  );
  expect(timing?.wasmInit).toMatchObject({
    path: "streaming",
    moduleShape: {
      exports: expect.any(Number),
      exportedFunctions: expect.any(Number),
      imports: expect.any(Number),
      importedFunctions: expect.any(Number),
      customSections: expect.any(Number),
    },
  });
  expect([null, "baseline", "shape", "shape-many", "shape-count"]).toContain(
    timing?.wasmInit?.experimentArm,
  );
  expect(timing?.wasmInit?.startMs).not.toBeNull();
  expect(timing?.wasmInit?.doneMs).not.toBeNull();
  expect(timing?.wasmInit?.apiMs).not.toBeNull();
});

test("initializer records buffered when streaming is unavailable and restores APIs", async ({
  page,
  bootTiming,
}) => {
  await page.addInitScript(() => {
    const scope = globalThis as typeof globalThis & {
      __jaunderOriginalStreaming?: typeof WebAssembly.instantiateStreaming;
      __jaunderOriginalInstantiate?: typeof WebAssembly.instantiate;
    };
    scope.__jaunderOriginalStreaming = WebAssembly.instantiateStreaming;
    scope.__jaunderOriginalInstantiate = WebAssembly.instantiate;
    WebAssembly.instantiateStreaming = undefined as never;
  });

  await goto(page, "/");
  await page.waitForFunction(
    (name) => performance.getEntriesByName(name, "mark").length !== 0,
    "jaunder.wasm.init_done",
  );
  expect((await bootTiming())?.wasmInit).toMatchObject({ path: "buffered" });
  expect(
    await page.evaluate(
      () =>
        WebAssembly.instantiateStreaming === undefined &&
        WebAssembly.instantiate ===
          (
            globalThis as typeof globalThis & {
              __jaunderOriginalInstantiate?: typeof WebAssembly.instantiate;
            }
          ).__jaunderOriginalInstantiate,
    ),
  ).toBe(true);
});

test("initializer records buffered after MIME-rejected streaming and restores APIs", async ({
  page,
  bootTiming,
}) => {
  await page.addInitScript(() => {
    const scope = globalThis as typeof globalThis & {
      __jaunderOriginalStreaming?: typeof WebAssembly.instantiateStreaming;
      __jaunderOriginalInstantiate?: typeof WebAssembly.instantiate;
    };
    scope.__jaunderOriginalStreaming = WebAssembly.instantiateStreaming;
    scope.__jaunderOriginalInstantiate = WebAssembly.instantiate;
  });
  await page.route("**/pkg/jaunder.wasm", async (route) => {
    const response = await route.fetch();
    await route.fulfill({
      response,
      headers: {
        ...response.headers(),
        "content-type": "application/octet-stream",
      },
    });
  });

  await goto(page, "/");
  await page.waitForFunction(
    (name) => performance.getEntriesByName(name, "mark").length !== 0,
    "jaunder.wasm.init_done",
  );
  expect((await bootTiming())?.wasmInit).toMatchObject({ path: "buffered" });
  expect(
    await page.evaluate(
      () =>
        WebAssembly.instantiateStreaming ===
          (
            globalThis as typeof globalThis & {
              __jaunderOriginalStreaming?: typeof WebAssembly.instantiateStreaming;
            }
          ).__jaunderOriginalStreaming &&
        WebAssembly.instantiate ===
          (
            globalThis as typeof globalThis & {
              __jaunderOriginalInstantiate?: typeof WebAssembly.instantiate;
            }
          ).__jaunderOriginalInstantiate,
    ),
  ).toBe(true);
});

test("failed streaming initialization stays incomplete and restores APIs", async ({
  page,
}) => {
  await page.addInitScript(() => {
    const scope = globalThis as typeof globalThis & {
      __jaunderRejectedStreaming?: typeof WebAssembly.instantiateStreaming;
      __jaunderOriginalInstantiate?: typeof WebAssembly.instantiate;
    };
    scope.__jaunderRejectedStreaming = async () => {
      throw new Error("forced streaming rejection");
    };
    scope.__jaunderOriginalInstantiate = WebAssembly.instantiate;
    WebAssembly.instantiateStreaming = scope.__jaunderRejectedStreaming;
  });

  await expect(goto(page, "/", { timeout: 2_000 })).rejects.toThrow();
  const marks = await page.evaluate(() =>
    performance
      .getEntriesByType("mark")
      .filter((entry) => entry.name.startsWith("jaunder.wasm."))
      .map((entry) => ({ name: entry.name, startTime: entry.startTime })),
  );
  expect(marks.map((mark) => mark.name)).toContain("jaunder.wasm.init_start");
  expect(marks.map((mark) => mark.name)).not.toContain(
    "jaunder.wasm.init_done",
  );
  expect(wasmInitFromMarks(marks)).toEqual({
    startMs: expect.any(Number),
    doneMs: null,
    apiMs: null,
    path: null,
    experimentArm: null,
    moduleShape: null,
  });
  expect(
    await page.evaluate(
      () =>
        WebAssembly.instantiateStreaming ===
          (
            globalThis as typeof globalThis & {
              __jaunderRejectedStreaming?: typeof WebAssembly.instantiateStreaming;
            }
          ).__jaunderRejectedStreaming &&
        WebAssembly.instantiate ===
          (
            globalThis as typeof globalThis & {
              __jaunderOriginalInstantiate?: typeof WebAssembly.instantiate;
            }
          ).__jaunderOriginalInstantiate,
    ),
  ).toBe(true);
});
