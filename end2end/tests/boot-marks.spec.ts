// `test` comes from `./fixtures`, not `@playwright/test`, even though the cases below
// are pure and drive no server fn: the `traced-context` gate forbids the upstream
// import anywhere under `end2end/tests` (only `fixtures.ts` is exempt), because a spec
// that opens no `e2e.test` span makes everything it drives unattributable — and that
// under-reports SILENTLY. A blanket rule is the point; carving out "but this one is
// pure" is how the guard stops guarding. The assertions here are still pure, so the
// merge invariant is proven by their logic, not by any browser behavior (#818).
import { test, expect } from "./fixtures";
import { mergeDocumentTiming, type DocumentTiming } from "./capture-trace";

const wasm = { startTime: 10, durationMs: 5, responseEndMs: 15 };
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
