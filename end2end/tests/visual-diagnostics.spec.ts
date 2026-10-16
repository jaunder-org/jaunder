import type { Page, TestInfo } from "@playwright/test";
import { expect, nonBrowserTest as test } from "./fixtures";
import { runVisualAssertion } from "./visual-diagnostics";

const original = new Error("original screenshot matcher rejection");
const rejectAssertion = async () => {
  throw original;
};
const fakePage = (evaluate: () => Promise<unknown>) =>
  ({
    evaluate,
    context: () => ({
      browser: () => ({
        version: () => "test-browser",
        browserType: () => ({ name: () => "chromium" }),
      }),
    }),
  }) as unknown as Page;

function sink(
  info: TestInfo,
  attach: TestInfo["attach"],
  timeout = info.timeout,
) {
  return () => ({
    timeout,
    startedUnixMs: Date.now(),
    testId: info.testId,
    retry: info.retry,
    project: info.project,
    config: info.config,
    attach,
  });
}

test("a passing visual assertion performs no diagnostic browser or sink reads", async () => {
  const page = fakePage(async () => {
    throw new Error("unexpected browser read");
  });
  await runVisualAssertion(
    page,
    "pass.png",
    async () => {},
    () => {
      throw new Error("unexpected sink read");
    },
  );
});

test("a visual failure attaches bounded post-failure state without input values or text", async ({
  page,
}, info) => {
  await page.setContent(
    '<div class="j-root"><input value="private-value-sentinel"><button class="j-post-action-trigger" aria-expanded="false">private-text-sentinel</button></div>',
  );
  const names: string[] = [];
  let text = "";
  const attach: TestInfo["attach"] = async (name, options) => {
    names.push(name);
    text = options?.body?.toString() ?? "";
    await info.attach(name, options);
  };
  await expect(
    runVisualAssertion(
      page,
      "failure.png",
      rejectAssertion,
      sink(info, attach),
    ),
  ).rejects.toBe(original);
  expect(names).toHaveLength(1);
  expect(text).not.toContain("private-value-sentinel");
  expect(text).not.toContain("private-text-sentinel");
  const diagnostic = JSON.parse(text);
  expect(diagnostic.observation_phase).toBe("post-assertion-failure");
  expect(diagnostic.capture_instant_geometry).toBe("unobserved");
  expect(diagnostic.rendered_font_face).toBe("unmeasured");
  expect(diagnostic.initial_pointer_coordinates).toBe("unobserved");
  expect(diagnostic.observation.nodes).toHaveLength(2);
  expect(diagnostic.runtime.browser_version).not.toBe("unavailable");
  expect(diagnostic.assertion_window.failed.unix_ms).toBeGreaterThanOrEqual(
    diagnostic.assertion_window.began.unix_ms,
  );
});

test("the normal harness clock and attachment sink retain failure evidence", async ({
  page,
}, info) => {
  await page.setContent('<div class="j-root"></div>');
  const before = info.attachments.length;
  await expect(
    runVisualAssertion(page, "default-sink.png", rejectAssertion),
  ).rejects.toBe(original);
  expect(
    info.attachments
      .slice(before)
      .some((a) => a.name.startsWith("visual-failure-")),
  ).toBe(true);
});

test("post-failure observation bounds the control population", async ({
  page,
}, info) => {
  await page.setContent(
    '<div class="j-root">' +
      '<button class="j-post-action-trigger"></button>'.repeat(20) +
      "</div>",
  );
  let body = "";
  await expect(
    runVisualAssertion(
      page,
      "bounded.png",
      rejectAssertion,
      sink(info, async (_, options) => {
        body = options?.body?.toString() ?? "";
      }),
    ),
  ).rejects.toBe(original);
  const value = JSON.parse(body);
  expect(value.observation.nodes).toHaveLength(16);
  expect(value.observation.omitted_nodes).toBe(5);
});

test("a closed-page collection rejection preserves the identical matcher error", async ({}, info) => {
  let attachments = 0;
  const page = fakePage(async () => {
    throw new Error("page closed");
  });
  await expect(
    runVisualAssertion(
      page,
      "closed.png",
      rejectAssertion,
      sink(info, async () => {
        attachments++;
      }),
    ),
  ).rejects.toBe(original);
  expect(attachments).toBe(0);
});

test("an ancillary warning failure cannot replace the matcher error", async ({}, info) => {
  const warn = console.warn;
  console.warn = () => {
    throw new Error("warning sink unavailable");
  };
  try {
    const page = fakePage(async () => {
      throw new Error("closed");
    });
    await expect(
      runVisualAssertion(
        page,
        "warning.png",
        rejectAssertion,
        sink(info, async () => {}),
      ),
    ).rejects.toBe(original);
  } finally {
    console.warn = warn;
  }
});

test("an attachment rejection preserves the identical matcher error", async ({}, info) => {
  const page = fakePage(async () => ({ available: true }));
  let attempted = false;
  await expect(
    runVisualAssertion(
      page,
      "sink.png",
      rejectAssertion,
      sink(info, async () => {
        attempted = true;
        throw new Error("sink unavailable");
      }),
    ),
  ).rejects.toBe(original);
  expect(attempted).toBe(true);
});

for (const stage of ["collection", "attachment"] as const) {
  test(`a never-resolving ${stage} cannot consume the original failure budget`, async ({}, info) => {
    const pending = () => new Promise<never>(() => {});
    const page = fakePage(stage === "collection" ? pending : async () => ({}));
    const begin = Date.now();
    await expect(
      runVisualAssertion(
        page,
        "deadline.png",
        rejectAssertion,
        sink(info, stage === "attachment" ? pending : async () => {}, 200),
      ),
    ).rejects.toBe(original);
    expect(Date.now() - begin).toBeLessThan(1_000);
  });
}

test("an exhausted budget skips diagnostics without extending the test timeout", async ({}, info) => {
  let reads = 0;
  const page = fakePage(async () => {
    reads++;
    return {};
  });
  const timeout = info.timeout;
  await expect(
    runVisualAssertion(
      page,
      "exhausted.png",
      rejectAssertion,
      sink(info, async () => {}, 50),
    ),
  ).rejects.toBe(original);
  expect(reads).toBe(0);
  expect(info.timeout).toBe(timeout);
});

test("two rejected visual assertions receive distinct attachment identities", async ({}, info) => {
  const names: string[] = [];
  const page = fakePage(async () => ({}));
  const capture = sink(info, async (name) => {
    names.push(name);
  });
  for (let i = 0; i < 2; i++)
    await expect(
      runVisualAssertion(page, "same.png", rejectAssertion, capture),
    ).rejects.toBe(original);
  expect(new Set(names).size).toBe(2);
});
