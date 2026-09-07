import type { Page } from "@playwright/test";
import { test, expect } from "./fixtures";
import {
  BASE_URL,
  confirmedMutation,
  signInAsNewUser,
  type MutationOutcome,
} from "./helpers";

type CatalogEntry = { id: number; name: string; published: boolean };
type ThemeDraft = {
  manifest: number[];
  stylesheet: number[];
  assets: Array<{ path: string; mime: string; bytes: number[] }>;
};
type ThemePreview = { html: string; css: string };
type ExportedPackage = { filename: string; bytes: number[] };
type ThemePresentation = {
  logo: ThemeBinding | null;
  header: ThemeBinding | null;
  header_pool: ThemePoolInput[];
  shuffle_seed: number[] | null;
};
type ThemeBinding = { kind: string; value?: string };
type ThemePoolInput = { kind: string; value: string };
type ThemeSelection = { kind: "custom"; value: number };
const THEME_ENDPOINTS = {
  create: "/api/themes/create",
  export: "/api/themes/export",
  get_draft: "/api/themes/get_draft",
  get_presentation: "/api/themes/get_presentation",
  get_selection: "/api/themes/get_selection",
  import_css: "/api/themes/import_css",
  import_package: "/api/themes/import_package",
  list: "/api/themes/list",
  preview: "/api/themes/preview",
  publish: "/api/themes/publish",
  remove: "/api/themes/remove",
  rename: "/api/themes/rename",
  replace_binding: "/api/themes/replace_binding",
  replace_css: "/api/themes/replace_css",
  replace_pool: "/api/themes/replace_pool",
  select: "/api/themes/select",
  shuffle: "/api/themes/shuffle",
} as const;
type ThemeEndpoint = keyof typeof THEME_ENDPOINTS;

const MANIFEST = new TextEncoder().encode(
  '{"schema":1,"name":"Transport","style_contract":1,"assets":{},"defaults":{}}',
);

const ASSET_PATH = "assets/private.png";
const ASSET_BYTES = new Uint8Array([
  137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0,
  0, 0, 1, 8, 2, 0, 0, 0, 144, 119, 83, 222, 0, 0, 0, 15, 73, 68, 65, 84, 120,
  1, 1, 4, 0, 251, 255, 0, 18, 52, 86, 0, 248, 0, 157, 248, 215, 100, 140, 0, 0,
  0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
]);
const ASSET_MANIFEST = new TextEncoder().encode(
  `{"schema":1,"name":"Transport Asset","style_contract":1,"assets":{"${ASSET_PATH}":"image/png"},"defaults":{}}`,
);

function draftForm(
  scope: "author" | "site",
  stylesheet: string,
  manifest = MANIFEST,
): URLSearchParams {
  const form = new URLSearchParams({ scope });
  for (const [index, byte] of manifest.entries()) {
    form.append(`draft[manifest][${index}]`, String(byte));
  }
  for (const [index, byte] of new TextEncoder().encode(stylesheet).entries()) {
    form.append(`draft[stylesheet][${index}]`, String(byte));
  }
  return form;
}

function assetDraftForm(
  scope: "author" | "site",
  stylesheet: string,
): URLSearchParams {
  const form = draftForm(scope, stylesheet, ASSET_MANIFEST);
  form.set("draft[assets][0][path]", ASSET_PATH);
  form.set("draft[assets][0][mime]", "image/png");
  for (const [index, byte] of ASSET_BYTES.entries()) {
    form.append(`draft[assets][0][bytes][${index}]`, String(byte));
  }
  return form;
}

function createForm(
  scope: "author" | "site",
  name: string,
  stylesheet: string,
): URLSearchParams {
  const form = draftForm(scope, stylesheet);
  form.set("name", name);
  return form;
}

function cssForm(
  scope: "author" | "site",
  name: string,
  stylesheet: string,
): URLSearchParams {
  const form = new URLSearchParams({ scope, name });
  for (const [index, byte] of new TextEncoder().encode(stylesheet).entries()) {
    form.append(`stylesheet[${index}]`, String(byte));
  }
  return form;
}

function replaceCssForm(
  scope: "author" | "site",
  themeId: number,
  stylesheet: string,
): URLSearchParams {
  const form = new URLSearchParams({ scope, theme_id: String(themeId) });
  for (const [index, byte] of new TextEncoder().encode(stylesheet).entries()) {
    form.append(`stylesheet[${index}]`, String(byte));
  }
  return form;
}

function scopedForm(
  scope: "author" | "site",
  themeId?: number,
): URLSearchParams {
  const form = new URLSearchParams({ scope });
  if (themeId !== undefined) form.set("theme_id", String(themeId));
  return form;
}

function enumInput(
  form: URLSearchParams,
  field: string,
  kind: string,
  value?: string | number,
) {
  form.set(`${field}[kind]`, kind);
  if (value !== undefined) form.set(`${field}[value]`, String(value));
}

function seedInput(form: URLSearchParams, field: string, seed: number[]) {
  for (const [index, byte] of seed.entries()) {
    form.append(`${field}[${index}]`, String(byte));
  }
}

async function postForm(
  page: Page,
  endpoint: ThemeEndpoint,
  form: URLSearchParams,
) {
  return page.request.post(`${BASE_URL}${THEME_ENDPOINTS[endpoint]}`, {
    data: form.toString(),
    headers: { "content-type": "application/x-www-form-urlencoded" },
  });
}

test("theme API transport preserves author ownership, private responses, imports, mutations, and preview isolation", async ({
  page,
  tracedContext,
}) => {
  const anonymous = await postForm(page, "list", scopedForm("author"));
  expect(anonymous.status()).toBe(500);
  expect(await anonymous.text()).toContain("unauthorized");

  await signInAsNewUser(page);

  const created = await postForm(
    page,
    "create",
    createForm("author", "Transport Theme", "body { color: navy; }"),
  );
  expect(created.status()).toBe(200);
  const theme = confirmedMutation(
    (await created.json()) as MutationOutcome<CatalogEntry>,
    "themes::create",
  );
  expect(theme).toMatchObject({ name: "Transport Theme", published: false });

  const cssImported = await postForm(
    page,
    "import_css",
    cssForm("author", "CSS Import", "body { color: maroon; }"),
  );
  expect(cssImported.status()).toBe(200);
  expect(cssImported.headers()["cache-control"]).toBe("private, no-store");
  const cssTheme = confirmedMutation(
    (await cssImported.json()) as MutationOutcome<CatalogEntry>,
    "themes::import_css",
  );
  expect(cssTheme).toMatchObject({ name: "CSS Import", published: false });

  const catalog = await postForm(page, "list", scopedForm("author"));
  expect(catalog.status()).toBe(200);
  expect(catalog.headers()["cache-control"]).toBe("private, no-store");
  expect((await catalog.json()) as CatalogEntry[]).toEqual([cssTheme, theme]);

  // import_package takes the existing ID as well as the complete draft.
  const importForm = draftForm("author", "body { color: rebeccapurple; }");
  importForm.set("theme_id", String(theme.id));
  const importResponse = await postForm(page, "import_package", importForm);
  expect(importResponse.status()).toBe(200);
  confirmedMutation(
    (await importResponse.json()) as MutationOutcome<null>,
    "themes::import_package",
  );

  const cssReplaced = await postForm(
    page,
    "replace_css",
    replaceCssForm("author", theme.id, "body { color: darkgreen; }"),
  );
  expect(cssReplaced.status()).toBe(200);
  confirmedMutation(
    (await cssReplaced.json()) as MutationOutcome<null>,
    "themes::replace_css",
  );

  const replacedDraft = await postForm(
    page,
    "get_draft",
    scopedForm("author", theme.id),
  );
  expect(replacedDraft.status()).toBe(200);
  expect(
    new TextDecoder().decode(
      new Uint8Array(((await replacedDraft.json()) as ThemeDraft).stylesheet),
    ),
  ).toBe("body { color: darkgreen; }");

  const renameForm = scopedForm("author", theme.id);
  renameForm.set("name", "Renamed Transport");
  const renamed = await postForm(page, "rename", renameForm);
  expect(renamed.status()).toBe(200);
  confirmedMutation(
    (await renamed.json()) as MutationOutcome<null>,
    "themes::rename",
  );

  const renamedCatalog = await postForm(page, "list", scopedForm("author"));
  expect(renamedCatalog.status()).toBe(200);
  expect((await renamedCatalog.json()) as CatalogEntry[]).toContainEqual({
    id: theme.id,
    name: "Renamed Transport",
    published: false,
  });

  const draft = await postForm(
    page,
    "get_draft",
    scopedForm("author", theme.id),
  );
  expect(draft.status()).toBe(200);
  expect(draft.headers()["cache-control"]).toBe("private, no-store");
  const storedDraft = (await draft.json()) as ThemeDraft;
  expect(
    new TextDecoder().decode(new Uint8Array(storedDraft.manifest)),
  ).toContain('"name":"Transport"');
  expect(new TextDecoder().decode(new Uint8Array(storedDraft.stylesheet))).toBe(
    "body { color: darkgreen; }",
  );
  expect(storedDraft.assets).toEqual([]);

  const presentation = await postForm(
    page,
    "get_presentation",
    scopedForm("author", theme.id),
  );
  expect(presentation.status()).toBe(200);
  expect(presentation.headers()["cache-control"]).toBe("private, no-store");
  expect((await presentation.json()) as ThemePresentation).toEqual({
    logo: null,
    header: null,
    header_pool: [],
    shuffle_seed: null,
  });

  const exported = await postForm(
    page,
    "export",
    scopedForm("author", theme.id),
  );
  expect(exported.status()).toBe(200);
  expect(exported.headers()["cache-control"]).toBe("private, no-store");
  const packageResponse = (await exported.json()) as ExportedPackage;
  expect(packageResponse.filename).toBe("RenamedTransport.zip");
  expect(packageResponse.bytes.length).toBeGreaterThan(0);
});

test("theme API transport preserves package bindings, deterministic pools, and deletion fallback", async ({
  page,
  tracedContext,
}) => {
  await signInAsNewUser(page);

  const assetForm = assetDraftForm("author", "body { color: teal; }");
  assetForm.set("name", "Private Asset");

  const assetCreated = await postForm(page, "create", assetForm);
  expect(assetCreated.status()).toBe(200);
  const assetTheme = confirmedMutation(
    (await assetCreated.json()) as MutationOutcome<CatalogEntry>,
    "themes::create",
  );
  const assetPublished = await postForm(
    page,
    "publish",
    scopedForm("author", assetTheme.id),
  );
  expect(assetPublished.status()).toBe(200);
  confirmedMutation(
    (await assetPublished.json()) as MutationOutcome<null>,
    "themes::publish",
  );
  const bindingForm = scopedForm("author", assetTheme.id);
  bindingForm.set("role", "header");
  enumInput(bindingForm, "input", "package_asset", ASSET_PATH);
  const bindingReplaced = await postForm(page, "replace_binding", bindingForm);
  expect(bindingReplaced.status()).toBe(200);
  confirmedMutation(
    (await bindingReplaced.json()) as MutationOutcome<null>,
    "themes::replace_binding",
  );

  const preview = await postForm(
    page,
    "preview",
    scopedForm("author", assetTheme.id),
  );
  expect(preview.status()).toBe(200);
  expect(preview.headers()["cache-control"]).toBe("private, no-store");
  const previewResponse = (await preview.json()) as ThemePreview;
  expect(previewResponse.html).toContain("data-jaunder-theme-surface");
  expect(previewResponse.css).toContain("teal");
  expect(previewResponse.html).toContain(
    `/themes/draft/${assetTheme.id}/${ASSET_PATH}`,
  );

  const selectionAfterPreview = await postForm(
    page,
    "get_selection",
    scopedForm("author"),
  );
  expect(selectionAfterPreview.status()).toBe(200);
  expect(await selectionAfterPreview.json()).toBeNull();

  const boundPresentation = await postForm(
    page,
    "get_presentation",
    scopedForm("author", assetTheme.id),
  );
  expect(boundPresentation.status()).toBe(200);
  expect((await boundPresentation.json()) as ThemePresentation).toEqual({
    logo: null,
    header: { kind: "package_asset", value: ASSET_PATH },
    header_pool: [],
    shuffle_seed: null,
  });

  const initialPoolSeed = Array.from({ length: 32 }, (_, index) => index);
  const poolForm = scopedForm("author", assetTheme.id);
  enumInput(poolForm, "inputs[0]", "package_asset", ASSET_PATH);
  seedInput(poolForm, "shuffle_seed", initialPoolSeed);
  const poolReplaced = await postForm(page, "replace_pool", poolForm);
  expect(poolReplaced.status()).toBe(200);
  confirmedMutation(
    (await poolReplaced.json()) as MutationOutcome<null>,
    "themes::replace_pool",
  );

  const pooledPresentation = await postForm(
    page,
    "get_presentation",
    scopedForm("author", assetTheme.id),
  );
  expect(pooledPresentation.status()).toBe(200);
  expect((await pooledPresentation.json()) as ThemePresentation).toEqual({
    logo: null,
    header: null,
    header_pool: [{ kind: "package_asset", value: ASSET_PATH }],
    shuffle_seed: initialPoolSeed,
  });

  const shuffledSeed = Array.from({ length: 32 }, (_, index) => 31 - index);
  const shuffleForm = scopedForm("author", assetTheme.id);
  seedInput(shuffleForm, "seed", shuffledSeed);
  const shuffled = await postForm(page, "shuffle", shuffleForm);
  expect(shuffled.status()).toBe(200);
  confirmedMutation(
    (await shuffled.json()) as MutationOutcome<null>,
    "themes::shuffle",
  );

  const shuffledPresentation = await postForm(
    page,
    "get_presentation",
    scopedForm("author", assetTheme.id),
  );
  expect(shuffledPresentation.status()).toBe(200);
  expect(
    (await shuffledPresentation.json()) as ThemePresentation,
  ).toMatchObject({
    header_pool: [{ kind: "package_asset", value: ASSET_PATH }],
    shuffle_seed: shuffledSeed,
  });
  const assetUrl = `${BASE_URL}/themes/draft/${assetTheme.id}/${ASSET_PATH}`;

  const ownerAsset = await page.request.get(assetUrl);
  expect(ownerAsset.status()).toBe(200);
  expect(ownerAsset.headers()["content-type"]).toBe("image/png");
  expect(ownerAsset.headers()["cache-control"]).toBe("private, no-store");
  expect(ownerAsset.headers()["x-content-type-options"]).toBe("nosniff");
  expect(await ownerAsset.body()).toEqual(Buffer.from(ASSET_BYTES));

  const anonymousContext = await tracedContext();
  try {
    const anonymousAsset = await anonymousContext.request.get(assetUrl);
    expect(anonymousAsset.status()).toBe(401);
  } finally {
    await anonymousContext.close();
  }

  const strangerContext = await tracedContext();
  try {
    const strangerPage = await strangerContext.newPage();
    await signInAsNewUser(strangerPage);
    const strangerAsset = await strangerPage.request.get(assetUrl);
    expect(strangerAsset.status()).toBe(404);
  } finally {
    await strangerContext.close();
  }

  const assetSelection = scopedForm("author");
  enumInput(assetSelection, "selection", "custom", assetTheme.id);
  const selectedAsset = await postForm(page, "select", assetSelection);
  expect(selectedAsset.status()).toBe(200);
  confirmedMutation(
    (await selectedAsset.json()) as MutationOutcome<null>,
    "themes::select",
  );

  const selectedAssetTheme = await postForm(
    page,
    "get_selection",
    scopedForm("author"),
  );
  expect(selectedAssetTheme.status()).toBe(200);
  expect((await selectedAssetTheme.json()) as ThemeSelection).toEqual({
    kind: "custom",
    value: assetTheme.id,
  });

  const removed = await postForm(
    page,
    "remove",
    scopedForm("author", assetTheme.id),
  );
  expect(removed.status()).toBe(200);
  confirmedMutation(
    (await removed.json()) as MutationOutcome<null>,
    "themes::remove",
  );

  const selectionAfterRemoval = await postForm(
    page,
    "get_selection",
    scopedForm("author"),
  );
  expect(selectionAfterRemoval.status()).toBe(200);
  expect(await selectionAfterRemoval.json()).toBeNull();

  const catalogAfterRemoval = await postForm(
    page,
    "list",
    scopedForm("author"),
  );
  expect(catalogAfterRemoval.status()).toBe(200);
  expect(
    (await catalogAfterRemoval.json()) as CatalogEntry[],
  ).not.toContainEqual(expect.objectContaining({ id: assetTheme.id }));

  const malformedZip = await page.request.post(
    `${BASE_URL}/api/themes/import_zip`,
    {
      multipart: {
        scope: "author",
        name: "Malformed import",
        archive: {
          name: "malformed.zip",
          mimeType: "application/zip",
          buffer: Buffer.from("not a zip"),
        },
      },
    },
  );
  expect(malformedZip.status()).toBe(500);
  expect(await malformedZip.text()).toContain("invalid theme package");
});
