/**
 * Tests for the polling primitive itself (#794, gap 6).
 *
 * These drive no browser — they declare no `page` fixture — but live as a spec
 * so they run in the same suite the primitive serves.
 */

import { test, expect } from "./fixtures";
import { pollUntil, pollUntilOrUndefined } from "./polling";

test("pollUntil returns the first non-undefined probe value", async () => {
  let calls = 0;
  const got = await pollUntil(
    "wait.test",
    () => (++calls < 3 ? undefined : "ok"),
    {
      intervalMs: 10,
      timeoutMs: 2_000,
      describe: "a value",
    },
  );
  expect(got).toBe("ok");
  expect(calls).toBe(3);
});

// The point of the pre-probe in pollUntil: `toPass` retries on ANY throw, so a
// misconfigured run (JAUNDER_CAPTURE_DIR unset => capturePathViaTool throws)
// would otherwise burn the whole timeout and report the wrong cause. Both halves
// matter — the original error must survive, and it must arrive promptly.
test("pollUntil rethrows a first-probe failure immediately", async () => {
  const startedMs = Date.now();
  await expect(
    pollUntil(
      "wait.test",
      () => {
        throw new Error("capture-path unset");
      },
      { intervalMs: 250, timeoutMs: 30_000, describe: "a value" },
    ),
  ).rejects.toThrow("capture-path unset");
  expect(
    Date.now() - startedMs,
    "a misconfigured run must fail fast, not after the poll timeout",
  ).toBeLessThan(1_000);
});

test("pollUntil throws once the timeout elapses", async () => {
  await expect(
    pollUntil("wait.test", () => undefined, {
      intervalMs: 10,
      timeoutMs: 200,
      describe: "something that never arrives",
    }),
  ).rejects.toThrow();
});

test("pollUntilOrUndefined resolves undefined instead of throwing", async () => {
  const got = await pollUntilOrUndefined("wait.test", () => undefined, {
    intervalMs: 10,
    timeoutMs: 200,
  });
  expect(got).toBeUndefined();
});

test("pollUntilOrUndefined still returns a value when one arrives", async () => {
  let calls = 0;
  const got = await pollUntilOrUndefined(
    "wait.test",
    () => (++calls < 2 ? undefined : "found"),
    { intervalMs: 10, timeoutMs: 2_000 },
  );
  expect(got).toBe("found");
});

// A falsy-but-defined value must satisfy the poll: `undefined` is the sentinel,
// not falsiness. An `if (!found)` regression would hang until timeout here.
test("pollUntil accepts a falsy non-undefined value", async () => {
  const got = await pollUntil("wait.test", () => "", {
    intervalMs: 10,
    timeoutMs: 2_000,
    describe: "an empty string",
  });
  expect(got).toBe("");
});
