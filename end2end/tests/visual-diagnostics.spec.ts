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
    onGeometry?: () => void;
    wideGeometry?: boolean;
  } = {},
) {
  let dy = 0;
  let restored = false;
  let removed = false;
  let scrollRestores = 0;
  const offsets: number[] = [];
  const captureTimeouts: number[] = [];
  const cssTexts: string[] = [];
  const attachments: Array<{
    name: string;
    body: Buffer;
    contentType?: string;
  }> = [];
  const state = (index: number) => {
    const x = options.wideGeometry ? 20.123456789012345 : 20;
    const width = options.wideGeometry ? 72.12345678901235 : 72;
    const height = options.wideGeometry ? 32.123456789012345 : 32;
    const y =
      index === 0
        ? 900
        : (options.wideGeometry ? 100.12345678901234 : 100.25) + dy;
    return {
      rect: {
        x,
        y,
        left: x,
        top: y,
        right: x + width,
        bottom: y + height,
        width,
        height,
      },
      scroll: {
        x: options.wideGeometry ? 0.12345678901234567 : 0,
        y: options.wideGeometry
          ? 0.12345678901234567
          : options.drift && offsets.length === 3
            ? 1
            : 0,
      },
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
      options.onGeometry?.();
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
    attachments.push({
      name,
      body: Buffer.from(args!.body!),
      contentType: args?.contentType,
    });
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
  revision: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
});

test("checkpoint rejection is precise and bounded, stops captures, and still attempts post-cleanup metadata", async ({}, info) => {
  const boundary = cornerBoundary();
  const warnings: string[] = [];
  const warn = console.warn;
  console.warn = (...args) => {
    warnings.push(args.map(String).join(" "));
  };
  try {
    const attempts: string[] = [];
    const attach: TestInfo["attach"] = async (name, args) => {
      attempts.push(name);
      if (name.includes("checkpoint-zero-before"))
        throw new TypeError(
          "checkpoint sink rejected token=private-sentinel " +
            "x".repeat(400) +
            "\nstack sentinel",
        );
      await boundary.attach(name, args);
    };
    await runVisualAssertion(
      boundary.page,
      "home-timeline.png",
      async () => {},
      cornerInfo(info, attach),
      sqliteProducer,
    );
    expect(boundary.captureTimeouts).toHaveLength(1);
    const detail = warnings.find((text) =>
      text.includes("stage=checkpoint-zero-before:attachment"),
    )!;
    expect(detail).toContain("error_type=TypeError");
    expect(detail).toContain("checkpoint-zero-before");
    expect(detail.split("message=")[1].length).toBeLessThanOrEqual(256);
    expect(detail).not.toContain("private-sentinel");
    expect(detail).not.toContain("stack sentinel");
    expect(Number(detail.match(/elapsed_ms=(\d+)/)![1])).toBeGreaterThanOrEqual(
      0,
    );
    expect(Number(detail.match(/remaining_ms=(\d+)/)![1])).toBeGreaterThan(0);
    const final = JSON.parse(boundary.attachments.at(-1)!.body.toString());
    expect(final).toMatchObject({
      status: "unavailable",
      reason: "stopped-after-unavailable-checkpoint",
      cleanup_errors: [],
    });
    expect(final.phases[0].checkpoint_status).toBe("unavailable");
    expect(attempts.at(-1)).toMatch(/^visual-corner-probe-\d+-\d+$/);
    expect(boundary.restoration()).toMatchObject({
      restored: true,
      removed: true,
    });
  } finally {
    console.warn = warn;
  }
});

test("a non-positive capture timeout is skipped and warns at the precise capture-admission stage", async ({}, info) => {
  let drain = () => {};
  const boundary = cornerBoundary({ onGeometry: () => drain() });
  const warnings: string[] = [];
  const warn = console.warn;
  console.warn = (...args) => {
    warnings.push(args.map(String).join(" "));
  };
  try {
    const visual = cornerInfo(info, boundary.attach)();
    drain = () => {
      if (boundary.offsets.length === 1)
        visual.timeout = Date.now() - visual.startedUnixMs + 220;
    };
    await runVisualAssertion(
      boundary.page,
      "home-timeline.png",
      async () => {},
      () => visual,
      sqliteProducer,
    );
    expect(boundary.captureTimeouts).toEqual([]);
    expect(warnings.join(" ")).toContain("stage=zero-before:capture-budget");
    expect(warnings.join(" ")).not.toContain(
      "stage=zero-before:geometry-before",
    );
    expect(boundary.restoration()).toMatchObject({
      restored: true,
      removed: true,
    });
  } finally {
    console.warn = warn;
  }
});

for (const exhaustion of ["cleanup-reserve", "exhausted"] as const) {
  test(`a first checkpoint survives ${exhaustion} admission and original PASS stays unchanged`, async ({}, info) => {
    const boundary = cornerBoundary();
    const warnings: string[] = [];
    const warn = console.warn;
    console.warn = (...args) => {
      warnings.push(args.map(String).join(" "));
    };
    try {
      const visual = cornerInfo(info, async (name, args) => {
        await boundary.attach(name, args);
        if (name.includes("checkpoint-zero-before"))
          visual.timeout =
            Date.now() -
            visual.startedUnixMs +
            (exhaustion === "cleanup-reserve" ? 220 : 100);
      })();
      await runVisualAssertion(
        boundary.page,
        "home-timeline.png",
        async () => {},
        () => visual,
        sqliteProducer,
      );
      expect(
        boundary.attachments.some((a) =>
          a.name.includes("checkpoint-zero-before"),
        ),
      ).toBe(true);
      expect(boundary.captureTimeouts).toHaveLength(1);
      expect(boundary.captureTimeouts.every((t) => t > 0 && t <= 150)).toBe(
        true,
      );
      expect(warnings.join(" ")).toContain("phase-admission:budget");
      if (exhaustion === "cleanup-reserve") {
        expect(boundary.restoration()).toMatchObject({
          restored: true,
          removed: true,
        });
        expect(
          JSON.parse(boundary.attachments.at(-1)!.body.toString()).reason,
        ).toContain("stopped-before-capture");
      } else expect(warnings.join(" ")).toContain("final-json:budget");
    } finally {
      console.warn = warn;
    }
  });
}

test("JSON serialization failure is distinguished from budget, size and sink rejection without replacing the matcher error", async ({}, info) => {
  const boundary = cornerBoundary();
  const warnings: string[] = [];
  const warn = console.warn;
  console.warn = (...args) => {
    warnings.push(args.map(String).join(" "));
  };
  try {
    const badName = {
      toJSON() {
        throw new TypeError("serialization rejected");
      },
    } as unknown as string;
    const visual = {
      ...cornerInfo(info, boundary.attach)(),
      attachments: [{ name: badName, contentType: "image/png" }],
    };
    await expect(
      runVisualAssertion(
        boundary.page,
        "home-timeline.png",
        rejectAssertion,
        () => visual,
        sqliteProducer,
      ),
    ).rejects.toBe(original);
    expect(warnings.join(" ")).toContain(
      "stage=original-evidence:serialization",
    );
    expect(warnings.join(" ")).toContain("stage=final-json:serialization");
    expect(warnings.join(" ")).toContain("message=serialization rejected");
    expect(boundary.captureTimeouts).toEqual([]);
  } finally {
    console.warn = warn;
  }
});

test("oversized checkpoint metadata becomes valid compact omission JSON without dropping retained PNG progress", async ({}, info) => {
  const boundary = cornerBoundary({ wideGeometry: true });
  // Escaped source text exercises the byte cap independently of character count.
  const producer = async () => ({
    ...(await sqliteProducer()),
    revision: "\u0001".repeat(64),
  });
  await runVisualAssertion(
    boundary.page,
    "home-timeline.png",
    async () => {},
    cornerInfo(info, boundary.attach),
    producer,
  );
  const checkpoints = boundary.attachments.filter((a) =>
    a.name.includes("checkpoint-"),
  );
  expect(checkpoints).toHaveLength(3);
  const first = JSON.parse(checkpoints[0].body.toString());
  expect(first).toMatchObject({
    phase: "zero-before",
    outcome: "available",
    attachment_status: "attached",
    metadata_status: "compact-omissions",
    omissions: expect.arrayContaining(["geometry-brackets", "css_sha256"]),
  });
  expect(first.png.sha256).toMatch(/^[a-f0-9]{64}$/);
  expect(first.png.name).toContain("zero-before");
  expect(first.source_sha256).toMatch(/^[a-f0-9]{64}$/);
  expect(checkpoints.every((a) => a.body.length <= 1024)).toBe(true);
  expect(
    JSON.parse(boundary.attachments.at(-1)!.body.toString()).cleanup,
  ).toEqual({
    "inline-style": "completed",
    scroll: "completed",
    stylesheet: "completed",
  });
});

for (const attachmentCount of [40, 56, 80]) {
  test(`earlier probe JSON (${attachmentCount} originals) and three checkpoints preserve reserved final cleanup capacity within 32KiB`, async ({}, info) => {
    const boundary = cornerBoundary({
      restoreThrows: attachmentCount !== 40,
      scrollThrows: attachmentCount === 80,
      stylesheetThrows: attachmentCount === 80,
    });
    const warnings: string[] = [];
    const warn = console.warn;
    console.warn = (...args) => {
      warnings.push(args.map(String).join(" "));
    };
    try {
      const visual = {
        ...cornerInfo(info, boundary.attach)(),
        attachments: Array.from({ length: attachmentCount }, () => ({
          name: "n".repeat(200),
          path: "p".repeat(200),
          contentType: "application/json",
        })),
      };
      await expect(
        runVisualAssertion(
          boundary.page,
          "home-timeline.png",
          rejectAssertion,
          () => visual,
          sqliteProducer,
        ),
      ).rejects.toBe(original);
      const json = boundary.attachments.filter(
        (a) =>
          a.name.startsWith("visual-corner-probe-") &&
          a.contentType === "application/json",
      );
      expect(
        json.reduce((sum, a) => sum + a.body.length, 0),
      ).toBeLessThanOrEqual(32768);
      const checkpoints = json.filter((a) => a.name.includes("checkpoint-"));
      expect(checkpoints).toHaveLength(3);
      expect(checkpoints.every((a) => a.body.length <= 1024)).toBe(true);
      const final = JSON.parse(json.at(-1)!.body.toString());
      expect(final).toMatchObject({
        probe: expect.stringContaining("visual-corner-probe-"),
        original_outcome: "failed",
        status: attachmentCount !== 40 ? "INVALID" : "available",
        record_kind: "post-cleanup",
        cleanup: {
          "inline-style":
            attachmentCount !== 40 ? "failed-or-unverified" : "completed",
          scroll: attachmentCount === 80 ? "failed-or-unverified" : "completed",
          stylesheet:
            attachmentCount === 80 ? "failed-or-unverified" : "completed",
        },
        pngs_attached: 3,
        checkpoints_attached: 3,
        omissions: expect.arrayContaining([
          "original_evidence-details",
          "phase-details",
        ]),
      });
      expect(json.at(-1)!.body.length).toBeLessThanOrEqual(2048);
      expect(
        json.slice(0, -1).reduce((sum, a) => sum + a.body.length, 0),
      ).toBeLessThanOrEqual(32768 - 2048);
      expect(json[0].body.length).toBeLessThanOrEqual(32768 - 2048 - 3 * 1024);
      if (attachmentCount === 80)
        expect(JSON.parse(json[0].body.toString()).omissions).toContain(
          "original_evidence-details",
        );
      if (attachmentCount !== 40)
        expect(final.cleanup_errors[0]).toContain("inline-style");
      if (attachmentCount === 80) expect(final.cleanup_errors).toHaveLength(3);
      expect(warnings.join(" ")).not.toContain("final-json:size");
    } finally {
      console.warn = warn;
    }
  });
}

test("a rejecting earlier JSON sink keeps attempted bytes charged and is not retried with a compact body", async ({}, info) => {
  const boundary = cornerBoundary();
  const attempts: Array<{ name: string; bytes: number }> = [];
  const warnings: string[] = [];
  const warn = console.warn;
  console.warn = (...args) => {
    warnings.push(args.map(String).join(" "));
  };
  try {
    const visual = {
      ...cornerInfo(info, async (name, args) => {
        if (name.startsWith("visual-corner-probe-"))
          attempts.push({ name, bytes: Buffer.byteLength(args!.body!) });
        if (name.endsWith("original-evidence"))
          throw new Error("original evidence sink rejected");
        await boundary.attach(name, args);
      })(),
      attachments: Array.from({ length: 40 }, () => ({
        name: "n".repeat(200),
        path: "p".repeat(200),
        contentType: "application/json",
      })),
    };
    await expect(
      runVisualAssertion(
        boundary.page,
        "home-timeline.png",
        rejectAssertion,
        () => visual,
        sqliteProducer,
      ),
    ).rejects.toBe(original);
    expect(attempts).toHaveLength(2);
    expect(attempts[0].name).toContain("original-evidence");
    const final = JSON.parse(boundary.attachments.at(-1)!.body.toString());
    expect(final).toMatchObject({
      record_kind: "post-cleanup",
      json_bytes_attempted_before_final: attempts[0].bytes,
      cleanup: {
        "inline-style": "not-needed",
        scroll: "not-needed",
        stylesheet: "not-needed",
      },
    });
    expect(attempts.reduce((sum, a) => sum + a.bytes, 0)).toBeLessThanOrEqual(
      32768,
    );
    expect(warnings.join(" ")).toContain("original-evidence:attachment");
    expect(warnings.join(" ")).not.toContain("final-json:size");
  } finally {
    console.warn = warn;
  }
});

for (const outcome of ["passed", "failed"] as const) {
  test(`a retained phase checkpoint survives final metadata rejection with original ${outcome} outcome`, async ({}, info) => {
    const boundary = cornerBoundary();
    const warnings: string[] = [];
    const warn = console.warn;
    console.warn = (...args) => {
      warnings.push(args.map(String).join(" "));
    };
    try {
      const attach: TestInfo["attach"] = async (name, args) => {
        if (/^visual-corner-probe-\d+-\d+$/.test(name))
          throw new Error("final sink unavailable");
        const phase = ["zero-before", "integer-snap", "zero-after"].findIndex(
          (p) => name.endsWith(`checkpoint-${p}`),
        );
        if (phase >= 0) {
          expect(boundary.captureTimeouts).toHaveLength(phase + 1);
          expect(boundary.attachments.at(-1)?.contentType).toBe("image/png");
        }
        await boundary.attach(name, args);
      };
      const run = runVisualAssertion(
        boundary.page,
        "home-timeline.png",
        outcome === "failed" ? rejectAssertion : async () => {},
        cornerInfo(info, attach),
        sqliteProducer,
      );
      if (outcome === "failed") await expect(run).rejects.toBe(original);
      else await run;
      const checkpoint = boundary.attachments.find((a) =>
        a.name.includes("checkpoint-zero-before"),
      );
      expect(checkpoint).toBeDefined();
      expect(JSON.parse(checkpoint!.body.toString())).toMatchObject({
        phase: "zero-before",
        outcome: "available",
        attachment_status: "attached",
        source: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        dy: 0,
        before_rect_scroll: [20, 100.25, 72, 32, 0, 0],
        png: { width: 72, height: 32 },
        caveat: "brackets-not-raster;cleanup-unobserved",
      });
      expect(checkpoint!.body.length).toBeLessThanOrEqual(1024);
      expect(warnings.join(" ")).toContain("final-json:attachment");
      expect(warnings.join(" ")).toContain("final sink unavailable");
    } finally {
      console.warn = warn;
    }
  });
}

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
    (a) => a.contentType === "image/png",
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
