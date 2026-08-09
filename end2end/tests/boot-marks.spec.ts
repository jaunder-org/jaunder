// `test` comes from `./fixtures`, not `@playwright/test`, even though the cases below
// are pure and drive no server fn: the `traced-context` gate forbids the upstream
// import anywhere under `end2end/tests` (only `fixtures.ts` is exempt), because a spec
// that opens no `e2e.test` span makes everything it drives unattributable — and that
// under-reports SILENTLY. A blanket rule is the point; carving out "but this one is
// pure" is how the guard stops guarding. The assertions here are still pure, so the
// merge invariant is proven by their logic, not by any browser behavior (#818).
import { test, expect } from "./fixtures";
import { mergeDocumentTiming, type DocumentTiming } from "./capture-trace";
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
    const incoming: DocumentTiming = { marks: marks(4), wasm };
    expect(mergeDocumentTiming(undefined, incoming)).toBe(incoming);
  });

  test("prefers the snapshot with more marks when it arrives second", () => {
    // The firefox ordering: `load` harvests an empty document first, mount-ready
    // harvests the full one after.
    const existing: DocumentTiming = { marks: [], wasm: null };
    const incoming: DocumentTiming = { marks: marks(4), wasm };
    expect(mergeDocumentTiming(existing, incoming)).toBe(incoming);
  });

  test("prefers the snapshot with more marks when it arrived first", () => {
    // The clobber this rule exists to prevent: a late-resolving `load` harvest must
    // not overwrite a complete mount-ready one.
    const existing: DocumentTiming = { marks: marks(4), wasm };
    const incoming: DocumentTiming = { marks: [], wasm: null };
    expect(mergeDocumentTiming(existing, incoming)).toBe(existing);
  });

  test("breaks a mark-count tie toward the incoming snapshot's wasm timing", () => {
    const existing: DocumentTiming = { marks: marks(4), wasm: null };
    const incoming: DocumentTiming = { marks: marks(4), wasm };
    expect(mergeDocumentTiming(existing, incoming)).toBe(incoming);
  });

  test("keeps the existing snapshot's wasm timing on a tie", () => {
    const existing: DocumentTiming = { marks: marks(4), wasm };
    const incoming: DocumentTiming = { marks: marks(4), wasm: null };
    expect(mergeDocumentTiming(existing, incoming)).toBe(existing);
  });

  test("keeps the existing snapshot when a tie gives neither side more", () => {
    const existing: DocumentTiming = { marks: marks(4), wasm };
    const incoming: DocumentTiming = { marks: marks(4), wasm };
    expect(mergeDocumentTiming(existing, incoming)).toBe(existing);
  });
});

// The regression guard for #818's actual defect. Unthresholded on purpose: it
// asserts the mechanism works at all, which needs no knowledge of the coverage
// distribution and so can ship before the distribution exists. Gradual erosion is
// #831's job.
test("the harness captures the full boot mark set after mount", async ({
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

  // The failure mode this exists for is silent (#866): a `<link rel="preload">`
  // whose request mode does not match wasm-bindgen's own `fetch` does not error,
  // it downloads the 2.2 MB bundle TWICE — strictly worse than having no preload.
  // Nothing else in the suite would notice.
  //
  // Proven able to fail, by mutation, rather than assumed (#866). Pointing the
  // preload at `/pkg/jaunder.wasm?mutation-proof` — same pathname, distinct URL,
  // so no cache coalescing — reddened this with
  //   "expected exactly one /pkg/jaunder.wasm request, got 2"
  // naming both URLs. Reverted afterwards. A guard that has never been red is
  // not a guard.
  //
  // Note what did NOT reproduce it: a `crossorigin="anonymous"` mismatch against
  // wasm-bindgen's same-origin `init()` fetch. Chromium coalesced that and made
  // one request. So this asserts the property, not that specific hazard, and
  // firefox is not known to behave the same way.
  expect(
    wasmRequests.length,
    `expected exactly one /pkg/jaunder.wasm request, got ${wasmRequests.length}: ${wasmRequests.join(", ")}`,
  ).toBe(1);

  const timing = await bootTiming();
  expect(
    timing,
    "no document timing was harvested for the mounted navigation",
  ).toBeDefined();

  // Assert the SHAPE, never the names: mark names live only in Rust and are
  // discovered by prefix, so enumerating them here would reintroduce exactly the
  // cross-language drift `MOUNTED_ATTR` suffers (#794). `>=`, never `===`, so
  // adding a mark in `client::perf` extends the set instead of reddening the build.
  const names = (timing?.marks ?? []).map((mark) => mark.name);
  expect(names.length).toBeGreaterThanOrEqual(4);
  expect(names.every((name) => name.startsWith("jaunder."))).toBe(true);
  expect(new Set(names).size).toBe(names.length);

  // The wasm resource entry is the other half of the decomposition — without it
  // `wasmInstantiateMs` is null and the boot total cannot be closed.
  expect(timing?.wasm ?? null).not.toBeNull();
});
