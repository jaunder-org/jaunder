import type { Page, TestInfo } from "@playwright/test";
import { expect, nonBrowserTest as test } from "./fixtures";
import { runVisualAssertion } from "./visual-diagnostics";

test("the timeline probe explicitly skips an unknown producer lane after the assertion", async ({}, info) => {
  const records: string[] = [];
  let asserted = false;
  await runVisualAssertion(
    fakePage(async () => ({})),
    "home-timeline.png",
    async () => {
      asserted = true;
    },
    () => ({
      ...sink(info, async (name, options) => {
        expect(asserted).toBe(true);
        if (name.startsWith("visual-corner-probe-"))
          records.push(options!.body!.toString());
      })(),
      project: { ...info.project, name: "chromium-visual" },
    }),
    async () => ({ available: false }),
  );
  expect(records).toHaveLength(1);
  expect(JSON.parse(records[0])).toMatchObject({
    status: "skipped",
    reason: "unknown-or-ineligible-producer-lane",
  });
});

function cornerBoundary(
  options: {
    restoreThrows?: boolean;
    oversized?: boolean;
    captureThrows?: boolean;
    drift?: boolean;
    lostStyle?: boolean;
    scrollThrows?: boolean;
    stylesheetThrows?: boolean;
  } = {},
) {
  let dy = 0;
  let restored = false;
  let removed = false;
  let scrollRestores = 0;
  const offsets: number[] = [];
  const captureTimeouts: number[] = [];
  const cssTexts: string[] = [];
  const attachments: Array<{ name: string; body: Buffer }> = [];
  const state = (index: number) => {
    const y = index === 0 ? 900 : 100.25 + dy;
    return {
      rect: {
        x: 20,
        y,
        left: 20,
        top: y,
        right: 92,
        bottom: y + 32,
        width: 72,
        height: 32,
      },
      scroll: { x: 0, y: options.drift && offsets.length === 3 ? 1 : 0 },
      viewport: { width: 1280, height: 720, dpr: 1 },
      transform: "none",
      radius: { top_right: "6px", bottom_right: "6px" },
      font_status: "loaded",
      main_document: true,
    };
  };
  const locator = (index: number) => ({
    getAttribute: async () =>
      restored && options.lostStyle ? "wrong-style" : "color: red;",
    evaluate: async (_: unknown, value: unknown) => {
      if (typeof value === "number") {
        dy = value;
        offsets.push(value);
        return;
      }
      if (typeof value === "string" || value === null) {
        restored = true;
        if (options.restoreThrows) throw new Error("inline restore failed");
        return;
      }
      return state(index);
    },
    screenshot: async (args: { timeout: number; style: string }) => {
      captureTimeouts.push(args.timeout);
      cssTexts.push(args.style);
      if (options.captureThrows) throw new Error("capture failed");
      const png = Buffer.alloc(options.oversized ? 262145 : 25);
      Buffer.from("89504e470d0a1a0a", "hex").copy(png);
      png.writeUInt32BE(72, 16);
      png.writeUInt32BE(32, 20);
      png[24] = dy === 0 ? 0 : 1;
      return png;
    },
  });
  const page = {
    ...fakePage(async () => {
      scrollRestores++;
      if (options.scrollThrows && scrollRestores === 4)
        throw new Error("scroll restore failed");
      return { x: 0, y: 0 };
    }),
    addStyleTag: async ({ content }: { content: string }) => {
      cssTexts.push(content);
      return {
        evaluate: async () => {
          removed = true;
          if (options.stylesheetThrows)
            throw new Error("stylesheet removal failed");
        },
      };
    },
    locator: (selector: string) => {
      expect(selector).toBe(
        "#j-trusted-post-actions button.j-post-action-trigger",
      );
      return { count: async () => 2, nth: locator };
    },
  } as unknown as Page;
  const attach: TestInfo["attach"] = async (name, args) => {
    attachments.push({ name, body: Buffer.from(args!.body!) });
  };
  return {
    page,
    attach,
    offsets,
    captureTimeouts,
    cssTexts,
    attachments,
    restoration: () => ({ restored, removed, scrollRestores }),
  };
}

function cornerInfo(
  info: TestInfo,
  attach: TestInfo["attach"],
  timeout = 30000,
) {
  return () => ({
    ...sink(info, attach, timeout)(),
    project: { ...info.project, name: "chromium-visual" },
  });
}
const sqliteProducer = async () => ({
  schema_version: 1,
  lane: "sqlite-chromium-unsplit",
});

test("the corner probe captures bounded reversible phases and restores the selected second control", async ({}, info) => {
  const boundary = cornerBoundary();
  await runVisualAssertion(
    boundary.page,
    "home-timeline.png",
    async () => {},
    cornerInfo(info, boundary.attach),
    sqliteProducer,
  );
  expect(boundary.offsets).toEqual([0, -0.25, 0]);
  expect(boundary.captureTimeouts).toHaveLength(3);
  expect(boundary.captureTimeouts.every((t) => t > 0 && t <= 150)).toBe(true);
  expect(new Set(boundary.cssTexts).size).toBe(1);
  const images = boundary.attachments.filter(
    (a) =>
      a.name.endsWith("zero-before") ||
      a.name.endsWith("integer-snap") ||
      a.name.endsWith("zero-after"),
  );
  expect(images).toHaveLength(3);
  expect(new Set(images.map((a) => a.name)).size).toBe(3);
  expect(images.every((a) => a.body.length <= 262144)).toBe(true);
  const record = JSON.parse(boundary.attachments.at(-1)!.body.toString());
  expect(record).toMatchObject({ status: "available", selected_index: 1 });
  expect(record.phases.map((p: { png: unknown }) => p.png)).toEqual(
    expect.arrayContaining([
      expect.objectContaining({ width: 72, height: 32 }),
    ]),
  );
  expect(boundary.restoration()).toMatchObject({
    restored: true,
    removed: true,
  });
});

test("restoration failures invalidate ancillary evidence but do not block other cleanup or replace the matcher error", async ({}, info) => {
  const boundary = cornerBoundary({ restoreThrows: true });
  await expect(
    runVisualAssertion(
      boundary.page,
      "home-timeline.png",
      rejectAssertion,
      cornerInfo(info, boundary.attach),
      sqliteProducer,
    ),
  ).rejects.toBe(original);
  expect(boundary.restoration()).toEqual({
    restored: true,
    removed: true,
    scrollRestores: 5,
  });
  const record = JSON.parse(boundary.attachments.at(-1)!.body.toString());
  expect(record.status).toBe("INVALID");
  expect(record.cleanup_errors[0]).toContain("inline-style");
});

test("original failure attachment paths are frozen before ancillary PNGs", async ({}, info) => {
  const boundary = cornerBoundary();
  const provider = () => ({
    ...cornerInfo(info, boundary.attach)(),
    outputDir: "/original-test-output",
    attachments: ["actual", "expected", "diff"].map((name) => ({
      name,
      path: `/original-test-output/${name}.png`,
      contentType: "image/png",
    })),
  });
  await expect(
    runVisualAssertion(
      boundary.page,
      "home-timeline.png",
      rejectAssertion,
      provider,
      sqliteProducer,
    ),
  ).rejects.toBe(original);
  const saved = boundary.attachments.findIndex((a) =>
    a.name.endsWith("original-evidence"),
  );
  const capture = boundary.attachments.findIndex((a) =>
    a.name.endsWith("zero-before"),
  );
  expect(saved).toBeGreaterThanOrEqual(0);
  expect(saved).toBeLessThan(capture);
  const record = JSON.parse(boundary.attachments[saved].body.toString());
  expect(
    record.original_evidence.attachments.map((a: { path: string }) => a.path),
  ).toEqual([
    "/original-test-output/actual.png",
    "/original-test-output/expected.png",
    "/original-test-output/diff.png",
  ]);
  expect(record.original_evidence.context_path).toBe(
    "/original-test-output/error-context.md",
  );
  expect(record.original_evidence.trace_path).toBe(
    "/original-test-output/trace.zip",
  );
});

test("a successful restoration RPC with mismatched inline style invalidates the probe", async ({}, info) => {
  const boundary = cornerBoundary({ lostStyle: true });
  await runVisualAssertion(
    boundary.page,
    "home-timeline.png",
    async () => {},
    cornerInfo(info, boundary.attach),
    sqliteProducer,
  );
  expect(JSON.parse(boundary.attachments.at(-1)!.body.toString()).status).toBe(
    "INVALID",
  );
});

for (const fault of ["oversized", "captureThrows", "drift"] as const) {
  test(`a ${fault} probe never manufactures a passing assertion failure`, async ({}, info) => {
    const boundary = cornerBoundary({ [fault]: true });
    await runVisualAssertion(
      boundary.page,
      "home-timeline.png",
      async () => {},
      cornerInfo(info, boundary.attach),
      sqliteProducer,
    );
    const record = JSON.parse(boundary.attachments.at(-1)!.body.toString());
    expect(record.status).toBe(fault === "drift" ? "INVALID" : "unavailable");
    expect(boundary.restoration()).toMatchObject({
      restored: true,
      removed: true,
    });
  });
}

for (const fault of ["scrollThrows", "stylesheetThrows"] as const) {
  test(`a ${fault} cleanup failure is nonthrowing and INVALID`, async ({}, info) => {
    const boundary = cornerBoundary({ [fault]: true });
    await runVisualAssertion(
      boundary.page,
      "home-timeline.png",
      async () => {},
      cornerInfo(info, boundary.attach),
      sqliteProducer,
    );
    expect(boundary.restoration()).toMatchObject({
      restored: true,
      removed: true,
    });
    const record = JSON.parse(boundary.attachments.at(-1)!.body.toString());
    expect(record.status).toBe("INVALID");
    expect(record.cleanup_errors).toHaveLength(1);
  });
}

for (const guard of ["filename", "project", "retry"] as const) {
  test(`the ${guard} guard excludes additional probe captures`, async ({}, info) => {
    const boundary = cornerBoundary();
    const provider = () => ({
      ...cornerInfo(info, boundary.attach)(),
      ...(guard === "project" ? { project: info.project } : {}),
      retry: guard === "retry" ? 1 : 0,
    });
    await runVisualAssertion(
      boundary.page,
      guard === "filename" ? "other.png" : "home-timeline.png",
      async () => {},
      provider,
      async () => {
        throw new Error("producer must not be read");
      },
    );
    expect(boundary.captureTimeouts).toEqual([]);
    expect(
      boundary.attachments.some((a) =>
        a.name.startsWith("visual-corner-probe-"),
      ),
    ).toBe(false);
  });
}

test("less than 850ms remaining admits no probe capture", async ({}, info) => {
  const boundary = cornerBoundary();
  await runVisualAssertion(
    boundary.page,
    "home-timeline.png",
    async () => {},
    cornerInfo(info, boundary.attach, 849),
    sqliteProducer,
  );
  expect(boundary.captureTimeouts).toEqual([]);
  const record = JSON.parse(boundary.attachments.at(-1)!.body.toString());
  expect(record.status).toBe("skipped");
});

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
