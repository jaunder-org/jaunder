import { appendFile, readFile, rename, writeFile } from "node:fs/promises";
import {
  nonBrowserTest as contractTest,
  expect,
  setTestBudget,
  test,
} from "./fixtures";
import {
  createProductionBaseline,
  mintBaselineAppPassword,
  verifyFreshAppPasswordLifecycle,
  verifyProductionBaseline,
  type BaselineState,
  validateBaselineState,
  parseCreatedPost,
  baselineAudience,
  HISTORICAL_COMPOSER,
  composeBaselinePost,
  baselinePostRequest,
  verifySeededManifest,
} from "./production-baseline";
import { allowSecondBoot } from "./bootBudget";
import { goto, login, TEST_PASSWORD } from "./helpers";
import {
  seedSandboxProfileViaTool,
  parseOrdinaryFixture,
  parseOrdinarySeedManifest,
  SEEDED_MEDIA_SHA256,
  type OrdinaryFixture,
} from "./seed";
import { StylingSession, type StylingRequest } from "./styling-qualification";

const statePath = process.env.JAUNDER_PRODUCTION_BASELINE_STATE;
const canaryPath = process.env.JAUNDER_PRODUCTION_BASELINE_CANARY_PATH;
const coordinator = process.env.JAUNDER_PRODUCTION_BASELINE_COORDINATOR;
const PRODUCTION_BASELINE_HARNESS_BUDGET_MS = 30 * 60_000;

type BrowserRequest = {
  sequence: number;
  phase: "create" | "read-only" | "restored" | "close";
  seed_process?: string;
  fixture: OrdinaryFixture;
};

type BrowserResult = {
  sequence: number;
  phase: BrowserRequest["phase"];
  checks: Array<{ id: string; outcome: "passed"; duration_ms: number }>;
  error?: string;
};

async function registerCanaries(values: readonly string[]): Promise<void> {
  if (!canaryPath) return;
  await appendFile(
    canaryPath,
    values
      .filter((value) => value.length > 0)
      .map((value) => `${JSON.stringify(value)}\n`)
      .join(""),
  );
}

async function publish(
  path: string,
  value:
    | BrowserResult
    | Awaited<ReturnType<StylingSession["run"]>>
    | { sequence: number; phase: BrowserRequest["phase"]; error: string }
    | { readonly: true },
): Promise<void> {
  const temporary = `${path}.tmp`;
  await writeFile(temporary, JSON.stringify(value));
  await rename(temporary, path);
}

async function waitForRequest(sequence: number): Promise<BrowserRequest> {
  if (!coordinator) throw new Error("baseline browser coordinator is missing");
  const path = `${coordinator}/request-${sequence}.json`;
  const deadline = Date.now() + PRODUCTION_BASELINE_HARNESS_BUDGET_MS;
  for (;;) {
    try {
      return JSON.parse(await readFile(path, "utf8")) as BrowserRequest;
    } catch (error: unknown) {
      if (!(
        error instanceof Error &&
        "code" in error &&
        error.code === "ENOENT"
      ))
        throw error;
      if (Date.now() >= deadline)
        throw new Error(`timed out waiting for browser request ${sequence}`);
      await new Promise<void>((resolve) => setTimeout(resolve, 50));
    }
  }
}

async function runPhase(
  request: BrowserRequest,
  page: Parameters<typeof createProductionBaseline>[0],
  tracedContext: Parameters<typeof createProductionBaseline>[1],
): Promise<BrowserResult> {
  if (!statePath) throw new Error("baseline browser state path is missing");
  const started = performance.now();
  let checks: string[];
  if (request.phase === "create") {
    if (!request.seed_process)
      throw new Error("create phase omitted the seed process path");
    process.env.JAUNDER_E2E_SEED_PROCESS = request.seed_process;
    const seededManifest = await seedSandboxProfileViaTool(
      "demo",
      request.fixture,
    );
    const state = await createProductionBaseline(
      page,
      tracedContext,
      seededManifest,
      request.fixture,
    );
    await registerCanaries([
      TEST_PASSWORD,
      state.appPassword,
      ...state.authorAccess.map((access) => access.appPassword),
      state.aliceSession.token,
      state.aliceSession.setCookie,
      state.aliceSession.marker,
      state.subscriberSession.token,
      state.subscriberSession.setCookie,
      state.subscriberSession.marker,
      state.nonSubscriberSession.token,
      state.nonSubscriberSession.setCookie,
      state.nonSubscriberSession.marker,
    ]);
    await writeFile(statePath, JSON.stringify(state));
    checks = ["browser-create", "atompub", "feeds"];
  } else {
    const state = JSON.parse(
      await readFile(statePath, "utf8"),
    ) as BaselineState;
    validateBaselineState(state, request.fixture);
    const continuityPage = await page.context().newPage();
    try {
      await verifyProductionBaseline(continuityPage, state, tracedContext);
    } finally {
      await continuityPage.close();
    }
    if (request.phase === "restored") {
      const recoveryContext = await tracedContext();
      try {
        const recoveryPage = await recoveryContext.newPage();
        allowSecondBoot(
          recoveryPage,
          "fresh authentication after continuity verification is part of the recovery contract",
        );
        await login(recoveryPage, state.username, TEST_PASSWORD);
        allowSecondBoot(
          recoveryPage,
          "App Password management has no in-app navigation control after fresh login",
        );
        await goto(recoveryPage, "/sessions");
        const fresh = await verifyFreshAppPasswordLifecycle(
          recoveryPage,
          state,
          tracedContext,
        );
        await registerCanaries([fresh]);
      } finally {
        await recoveryContext.close();
      }
    }
    checks = ["browser-read-only", "atompub", "feeds"];
  }
  const durationMs = Math.max(
    1,
    Math.floor((performance.now() - started) / checks.length),
  );
  return {
    sequence: request.sequence,
    phase: request.phase,
    checks: checks.map((id) => ({
      id,
      outcome: "passed",
      duration_ms: durationMs,
    })),
  };
}

function contractManifest(fixture: OrdinaryFixture) {
  const posts: Array<Record<string, unknown>> = [];
  for (const [author, markdown, org] of [
    ["user", "horizon-worth-keeping", "calm-field-note"],
    ["operator", "workshop-checks", "workshop-rhythm"],
    ["alice", "field-paths", "field-margins"],
    ["bob", "night-signals", "night-watch"],
  ]) {
    for (const slug of [
      markdown,
      org,
      "sandbox-markdown-draft",
      "sandbox-org-draft",
      ...Array.from(
        { length: 14 },
        (_, i) => `sandbox-post-${String(i + 2).padStart(2, "0")}`,
      ).filter((slug) => slug !== "sandbox-post-13"),
    ])
      posts.push({
        author,
        slug,
        title: slug,
        body: "native",
        format: "markdown",
        visibility: "public",
        publishedAt: null,
      });
  }
  for (const slug of [
    fixture === "legacy1419" ? "baseline-seeded-html" : "formatted-html-title",
    ...(fixture === "modern"
      ? ["formatted-markdown-title", "formatted-org-title"]
      : []),
    "baseline-seeded-scheduled",
    "baseline-seeded-subscribers",
    "baseline-seeded-private",
    "baseline-seeded-media-reference",
  ])
    posts.push({
      author: "alice",
      slug,
      title: slug,
      body: "native",
      format: "html",
      visibility: "public",
      publishedAt: "2026-09-12T12:00:00Z",
    });
  const sha256 =
    "958e3ce706b2891ab01e58e0f180af8ef646dc9d6368f3a2ba6d6cbff3321c5e";
  const text = {
    author: "alice",
    filename: "baseline-seeded.txt",
    sha256,
    sizeBytes: 22,
    contentUrl: `/media/upload/95/8e/${sha256}/baseline-seeded.txt`,
  };
  const svg = [
    ["user", "blue-horizon.svg"],
    ["operator", "warm-workshop.svg"],
    ["alice", "green-field.svg"],
    ["bob", "violet-night.svg"],
  ].map(([author, filename]) => {
    const digest = SEEDED_MEDIA_SHA256[filename];
    return {
      author,
      filename,
      sha256: digest,
      sizeBytes: 1,
      contentUrl: `/media/upload/${digest.slice(0, 2)}/${digest.slice(2, 4)}/${digest}/${filename}`,
    };
  });
  return {
    version: fixture === "legacy1419" ? 1 : 2,
    posts,
    media: fixture === "legacy1419" ? text : [text, ...svg],
  };
}

contractTest(
  "ordinary fixture contracts reject swapped or incomplete manifests",
  () => {
    for (const fixture of ["modern", "legacy1419"] as const) {
      const manifest = contractManifest(fixture);
      expect(parseOrdinarySeedManifest(fixture, manifest)).toBe(manifest);
      expect(() =>
        parseOrdinarySeedManifest(
          fixture === "modern" ? "legacy1419" : "modern",
          manifest,
        ),
      ).toThrow();
      for (const mutate of [
        (value: typeof manifest) => value.posts.pop(),
        (value: typeof manifest) => {
          value.posts[1] = value.posts[0];
        },
        (value: typeof manifest) => {
          delete value.posts[0].body;
        },
        (value: typeof manifest) => {
          value.media = null as unknown as typeof value.media;
        },
        (value: typeof manifest) => {
          const media = Array.isArray(value.media)
            ? value.media[0]
            : value.media;
          media.sha256 = "0".repeat(64);
        },
        (value: typeof manifest) => {
          value.posts[0].format = "unknown";
        },
        (value: typeof manifest) => {
          value.posts[0].publishedAt = "invalid";
        },
      ]) {
        const malformed = structuredClone(manifest);
        mutate(malformed);
        expect(() => parseOrdinarySeedManifest(fixture, malformed)).toThrow();
      }
      for (const [field, valid] of [
        ["format", "markdown"],
        ["visibility", "public"],
      ]) {
        for (const malformedType of [[valid], { value: valid }, null]) {
          const malformed = structuredClone(manifest);
          malformed.posts[0][field] = malformedType;
          expect(() => parseOrdinarySeedManifest(fixture, malformed)).toThrow();
        }
      }
      if (fixture === "modern") {
        const missingMedia = structuredClone(manifest);
        (missingMedia.media as unknown[]).pop();
        expect(() =>
          parseOrdinarySeedManifest(fixture, missingMedia),
        ).toThrow();
        const missingTitle = structuredClone(manifest);
        missingTitle.posts = missingTitle.posts.filter(
          (post) => post.slug !== "formatted-org-title",
        );
        expect(() =>
          parseOrdinarySeedManifest(fixture, missingTitle),
        ).toThrow();
      }
      const state = {
        fixture,
        seededManifest: manifest,
      } as unknown as BaselineState;
      validateBaselineState(state, fixture);
      expect(() =>
        validateBaselineState(
          state,
          fixture === "modern" ? "legacy1419" : "modern",
        ),
      ).toThrow();
    }
    expect(() => parseOrdinaryFixture("v1")).toThrow();
  },
);

contractTest(
  "seeded verification uses the selected audience response contract",
  async () => {
    const posts = (["public", "subscribers", "private"] as const).map(
      (visibility, i) => ({
        author: "alice",
        title: `Post ${i + 1}`,
        slug: `post-${i + 1}`,
        body: "Native body",
        format: "markdown" as const,
        publishedAt: null,
        visibility,
      }),
    );
    const document = posts
      .map(
        (post, i) =>
          `<entry><id>https://localhost/atompub/alice/posts/${i + 1}</id><title>${post.title}</title><j:slug>${post.slug}</j:slug><content type="text/markdown">${post.body}</content><app:draft>yes</app:draft></entry>`,
      )
      .join("");
    for (const fixture of ["legacy1419", "modern"] as const) {
      const state = {
        fixture,
        seededManifest: { posts },
        authorAccess: [
          {
            username: "alice",
            appPassword: "test-only",
            postsCollectionUrl: "/atompub/alice/posts",
          },
        ],
      } as unknown as BaselineState;
      let verified = 0;
      let closed = 0;
      let wrongContract = false;
      const tracedContext = (async () => ({
        request: {
          get: async () => ({ status: () => 200, text: async () => document }),
          post: async (url: string, options: { form: { post_id: string } }) => {
            expect(url).toContain("/api/posts/get_audience_selection");
            const visibility =
              posts[Number(options.form.post_id) - 1].visibility;
            verified += 1;
            const historical = wrongContract
              ? fixture !== "legacy1419"
              : fixture === "legacy1419";
            return {
              status: () => 200,
              json: async () =>
                historical
                  ? { base: visibility, named: [] }
                  : {
                      public: visibility === "public",
                      subscribers: visibility === "subscribers",
                      named: [],
                    },
            };
          },
        },
        close: async () => {
          closed += 1;
        },
      })) as unknown as Parameters<typeof verifySeededManifest>[1];
      await verifySeededManifest(state, tracedContext);
      expect(verified).toBe(3);
      expect(closed).toBe(1);
      wrongContract = true;
      await expect(
        verifySeededManifest(state, tracedContext),
      ).rejects.toThrow();
      expect(closed).toBe(2);
    }
  },
);

contractTest(
  "historical composer preserves exact inputs without modern disclosures",
  async () => {
    const actions: unknown[] = [];
    const page = {
      locator: (selector: string) => ({
        waitFor: async () => {
          actions.push(["wait", selector]);
        },
      }),
      fill: async (selector: string, value: string) => {
        actions.push(["fill", selector, value]);
      },
      click: async (selector: string) => {
        actions.push(["click", selector]);
      },
      selectOption: async (selector: string, value: string) => {
        actions.push(["select", selector, value]);
      },
    } as unknown as Parameters<typeof composeBaselinePost>[0];
    const body = "* Baseline browser Org\n\nCanonical browser-created Org.";
    await composeBaselinePost(page, "legacy1419", {
      body,
      format: "org",
      slug: "baseline-browser-org",
      audience: "subscribers",
      publish: true,
    });
    expect(actions).toEqual([
      ["wait", HISTORICAL_COMPOSER.body],
      ["fill", HISTORICAL_COMPOSER.body, body],
      ["click", HISTORICAL_COMPOSER.format],
      ["fill", HISTORICAL_COMPOSER.slug, "baseline-browser-org"],
      ["select", "#audience-base", "subscribers"],
      ["click", HISTORICAL_COMPOSER.publish],
      ["wait", HISTORICAL_COMPOSER.summary],
    ]);
    expect(
      baselinePostRequest("legacy1419", {
        body,
        format: "org",
        audience: "subscribers",
        slug: "native-body",
        publishAt: "2035-01-01T00:00:00Z",
      }),
    ).toEqual({
      post: {
        body,
        format: "org",
        audience: { base: "subscribers", named: [] },
        slug_override: "native-body",
        publish: true,
        publish_at: "2035-01-01T00:00:00Z",
      },
    });
  },
);

contractTest("App Password mint controls are fixture-owned", async () => {
  for (const fixture of ["legacy1419", "modern"] as const) {
    const actions: unknown[] = [];
    let failure: "click" | "wait" | "read" | undefined;
    const create =
      fixture === "legacy1419"
        ? '.j-app-passwords button:has-text("Create app password")'
        : '[data-app-passwords] button:has-text("Create app password")';
    const token =
      fixture === "legacy1419"
        ? ".j-app-password-token code"
        : "[data-app-password-token] code";
    const page = {
      url: () => "https://localhost/sessions",
      fill: async (selector: string, value: string) => {
        actions.push(["fill", selector, value]);
      },
      click: async (selector: string) => {
        expect(selector).toBe(create);
        actions.push(["click", selector]);
        if (failure === "click") throw new Error("creation failed");
      },
      locator: (selector: string) => {
        expect(selector).toBe(token);
        return {
          waitFor: async (options: unknown) => {
            actions.push(["wait", selector, options]);
            if (failure === "wait") throw new Error("token wait failed");
          },
          textContent: async () => {
            actions.push(["read", selector]);
            if (failure === "read") throw new Error("token read failed");
            return "  synthetic-not-a-credential  ";
          },
        };
      },
    } as unknown as Parameters<typeof mintBaselineAppPassword>[0];
    expect(await mintBaselineAppPassword(page, fixture, "Contract label")).toBe(
      "synthetic-not-a-credential",
    );
    expect(actions).toEqual([
      ["fill", 'input[name="label"]', "Contract label"],
      ["click", create],
      ["wait", token, { state: "visible", timeout: 15_000 }],
      ["read", token],
    ]);
    await expect(
      mintBaselineAppPassword(
        page,
        fixture === "modern" ? "legacy1419" : "modern",
        "Contract label",
      ),
    ).rejects.toThrow();
    for (const stage of ["click", "wait", "read"] as const) {
      failure = stage;
      await expect(
        mintBaselineAppPassword(page, fixture, "Contract label"),
      ).rejects.toThrow(
        stage === "click" ? "creation failed" : `token ${stage} failed`,
      );
    }
  }
});

contractTest(
  "historical App Password DOM has no modern hooks",
  async ({ page }) => {
    await page.setContent(`<h1>Sessions</h1><section class="j-app-passwords">
    <h2>App passwords</h2><label>Label <input name="label"></label>
    <button>Create app password</button></section>
    <p class="j-app-password-token"><code>synthetic-not-a-credential</code></p>`);
    expect(await page.locator("[data-app-passwords]").count()).toBe(0);
    expect(await page.locator("[data-app-password-token]").count()).toBe(0);
    const sessionsPage = {
      url: () => "https://localhost/sessions",
      fill: page.fill.bind(page),
      click: page.click.bind(page),
      locator: page.locator.bind(page),
    } as Parameters<typeof mintBaselineAppPassword>[0];
    expect(
      await mintBaselineAppPassword(
        sessionsPage,
        "legacy1419",
        "Contract label",
      ),
    ).toBe("synthetic-not-a-credential");
    await expect(page.locator('input[name="label"]')).toHaveValue(
      "Contract label",
    );
  },
);

contractTest(
  "ordinary fixture reaches all three App Password consumers",
  async () => {
    // Wiring proof complements actual control execution without seeding a product
    // or claiming a synthetic session is a real guest/browser lifecycle.
    const source = (
      await readFile(`${__dirname}/production-baseline.ts`, "utf8")
    ).replace(/\s+/g, " ");
    for (const call of [
      'mintBaselineAppPassword( page, fixture, "Production baseline primary", )',
      "mintBaselineAppPassword( page, fixture, `Production baseline manifest ${username}`, )",
      "createAuthorAccess(tracedContext, author, fixture)",
    ])
      expect(source).toContain(call);
    expect(source.match(/await mintBaselineAppPassword\(/g)).toHaveLength(3);
  },
);

contractTest(
  "fresh App Password mint use revoke retains the selected fixture",
  async ({ page: dom }) => {
    for (const fixture of ["legacy1419", "modern"] as const) {
      let revoked = false;
      let created = false;
      let closed = 0;
      const controls =
        fixture === "legacy1419" ? ".j-app-passwords" : "[data-app-passwords]";
      await dom.setContent(`<section ${fixture === "legacy1419" ? 'class="j-app-passwords"' : "data-app-passwords"}>
      <input name="label"><button>Create app password</button></section>
      <p ${fixture === "legacy1419" ? 'class="j-app-password-token"' : "data-app-password-token"}><code>synthetic-not-a-credential</code></p>
      <li>Production baseline revocable<button onclick="this.parentElement.remove()">Revoke</button></li>`);
      const page = {
        url: () => "https://localhost/sessions",
        fill: async (selector: string, value: string) => {
          expect(selector).toBe('input[name="label"]');
          expect(value).toBe("Production baseline revocable");
          await dom.fill(selector, value);
        },
        click: async (selector: string) => {
          if (!created) {
            expect(selector).toBe(
              `${controls} button:has-text("Create app password")`,
            );
            created = true;
          } else {
            expect(selector).toBe(
              'li:has-text("Production baseline revocable") button:has-text("Revoke")',
            );
            revoked = true;
          }
          await dom.click(selector);
        },
        locator: dom.locator.bind(dom),
      } as unknown as Parameters<typeof verifyFreshAppPasswordLifecycle>[0];
      const tracedContext = (async () => ({
        request: {
          get: async (
            _url: string,
            options: { headers: { authorization: string } },
          ) => {
            expect(options.headers.authorization).toBe(
              `Basic ${Buffer.from("contract-user:synthetic-not-a-credential").toString("base64")}`,
            );
            return { status: () => (revoked ? 401 : 200) };
          },
        },
        close: async () => {
          closed += 1;
        },
      })) as unknown as Parameters<typeof verifyFreshAppPasswordLifecycle>[2];
      expect(
        await verifyFreshAppPasswordLifecycle(
          page,
          { fixture, username: "contract-user" } as BaselineState,
          tracedContext,
        ),
      ).toBe("synthetic-not-a-credential");
      expect(revoked).toBe(true);
      expect(closed).toBe(2);
    }
  },
);

contractTest("Post create parsing and audience are fixture-owned", () => {
  const saved = {
    post_id: 1,
    slug: "baseline-html",
    published_at: "2026-09-12T12:00:00Z",
    permalink: "/~alice/2026/09/12/baseline-html",
  };
  const legacy = { Confirmed: saved };
  const modern = { Confirmed: { post: saved, publication: "published" } };
  expect(parseCreatedPost("legacy1419", legacy)).toEqual(saved);
  expect(parseCreatedPost("modern", modern)).toEqual(saved);
  for (const wrong of [
    modern,
    saved,
    { Confirmed: { post: saved } },
    { Indeterminate: saved },
    { Confirmed: { ...saved, post_id: "1" } },
    { Confirmed: { ...saved, permalink: "https://example.invalid/post" } },
  ])
    expect(() => parseCreatedPost("legacy1419", wrong)).toThrow();
  expect(() => parseCreatedPost("modern", legacy)).toThrow();
  for (const publication of [["published"], { value: "published" }, null]) {
    expect(() =>
      parseCreatedPost("modern", { Confirmed: { post: saved, publication } }),
    ).toThrow();
  }
  expect(baselineAudience("legacy1419", "subscribers")).toEqual({
    base: "subscribers",
    named: [],
  });
  expect(baselineAudience("modern", "subscribers")).toEqual({
    public: false,
    subscribers: true,
    named: [],
  });
  expect(HISTORICAL_COMPOSER.audience).toBe("#audience-base");
  expect(HISTORICAL_COMPOSER.format).not.toContain("control-summary");
});

test("production baseline host bridge preserves one browser context", async ({
  page,
  tracedContext,
}) => {
  setTestBudget(PRODUCTION_BASELINE_HARNESS_BUDGET_MS);
  test.skip(
    !statePath || !coordinator,
    "only xtask supplies restricted baseline coordinator",
  );
  if (!statePath || !coordinator) return;
  const styling =
    process.env.JAUNDER_STYLING_QUALIFICATION === "1"
      ? await StylingSession.start(tracedContext, registerCanaries)
      : null;
  let ordinaryFixture: OrdinaryFixture | undefined;
  try {
    await writeFile(`${coordinator}/ready`, "");
    for (let sequence = 1; ; sequence += 1) {
      const request = await waitForRequest(sequence);
      if (request.sequence !== sequence)
        throw new Error(
          `unexpected browser request sequence ${request.sequence}`,
        );
      if (!styling) {
        request.fixture = parseOrdinaryFixture(request.fixture);
        if (
          ordinaryFixture !== undefined &&
          ordinaryFixture !== request.fixture
        )
          throw new Error("ordinary request fixture changed");
        ordinaryFixture = request.fixture;
        if (
          !["create", "read-only", "restored", "close"].includes(request.phase)
        )
          throw new Error("unknown ordinary phase");
        if (sequence === 1 && request.phase !== "create")
          throw new Error("ordinary session must begin with creation");
        if (sequence !== 1 && request.phase === "create")
          throw new Error("ordinary creation cannot repeat");
      }
      if (request.phase === "close") {
        await publish(`${coordinator}/closed`, { readonly: true });
        return;
      }
      try {
        await publish(
          `${coordinator}/result-${sequence}.json`,
          styling
            ? await styling.run(request as unknown as StylingRequest)
            : await runPhase(request, page, tracedContext),
        );
      } catch (error) {
        const failure = {
          sequence,
          phase: request.phase,
          error:
            error instanceof Error
              ? (error.stack ?? error.message)
              : String(error),
        };
        await publish(
          `${coordinator}/result-${sequence}.json`,
          styling ? failure : { ...failure, checks: [] },
        );
        throw error;
      }
    }
  } finally {
    await styling?.close();
  }
});
