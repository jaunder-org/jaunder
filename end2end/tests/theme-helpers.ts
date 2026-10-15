import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";

import { expect, type Page } from "@playwright/test";

import {
  BASE_URL,
  confirmedMutation,
  goto,
  type MutationOutcome,
} from "./helpers";

export type ThemeAsset = { path: string; mime: string; bytes: Uint8Array };
export type ThemePackage = {
  manifest: Record<string, unknown>;
  stylesheet: string;
  assets: ThemeAsset[];
};

type CatalogEntry = { id: number; name: string; published: boolean };

const FONT_PATH = "assets/conformance.woff2";
const LOGO_PATH = "assets/logo.png";
const HEADER_PATH = "assets/header.png";
const THEME_ENDPOINTS = {
  create: "/api/themes/create",
  publish: "/api/themes/publish",
  select: "/api/themes/select",
} as const;

const PIXEL_PNG = new Uint8Array([
  137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0,
  0, 0, 1, 8, 2, 0, 0, 0, 144, 119, 83, 222, 0, 0, 0, 15, 73, 68, 65, 84, 120,
  1, 1, 4, 0, 251, 255, 0, 18, 52, 86, 0, 248, 0, 157, 248, 215, 100, 140, 0, 0,
  0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
]);

// This local fixture is copied from the compiler's existing Roboto validation
// fixture; its provenance is recorded beside the checked-in test asset.
const TEST_FONT = readFileSync(
  resolve(__dirname, "fixtures/theme-package/roboto-regular.woff2"),
);

/** A deterministic schema-1 package that exercises semantic presentation only. */
export function conformanceThemePackage(): ThemePackage {
  return {
    manifest: {
      schema: 1,
      name: "Conformance",
      style_contract: 1,
      assets: {
        [FONT_PATH]: "font/woff2",
        [LOGO_PATH]: "image/png",
        [HEADER_PATH]: "image/png",
      },
      defaults: { logo: LOGO_PATH, header: [HEADER_PATH] },
    },
    stylesheet: `@font-face { font-family: "Conformance Sans"; src: url(${FONT_PATH}) format("woff2"); }
[data-jaunder-part="main"] { background: rgb(255, 255, 255); color: rgb(30, 41, 59); }
[data-jaunder-part="post"] { border-block-end: 2px solid rgb(148, 163, 184); }
[data-jaunder-part="post-body"] { font-family: "Conformance Sans", sans-serif; }
@media (prefers-color-scheme: dark) {
  [data-jaunder-part="main"] { background: rgb(16, 24, 32); color: rgb(241, 245, 249); }
  [data-jaunder-part="post"] { border-block-end-color: rgb(143, 163, 184); }
}`,
    assets: [
      { path: FONT_PATH, mime: "font/woff2", bytes: TEST_FONT },
      { path: LOGO_PATH, mime: "image/png", bytes: PIXEL_PNG },
      { path: HEADER_PATH, mime: "image/png", bytes: PIXEL_PNG },
    ],
  };
}

function appendBytes(form: URLSearchParams, prefix: string, bytes: Uint8Array) {
  for (const [index, byte] of bytes.entries())
    form.append(`${prefix}[${index}]`, String(byte));
}

/** Opt into the source/package/media authoring controls without weakening their assertions. */
export async function openThemeAdvanced(page: Page): Promise<void> {
  const disclosure = page.getByRole("button", {
    name: "Advanced",
    exact: true,
  });
  if ((await disclosure.getAttribute("aria-expanded")) !== "true")
    await disclosure.click();
  await expect(disclosure).toHaveAttribute("aria-expanded", "true");
}

/** Drive the default import form with a real exporter-backed package. */
export async function importMinimalTheme(page: Page): Promise<void> {
  const archive = await minimalThemeZip(page);
  await goto(page, "/themes");
  await page.getByLabel("Theme Package ZIP").setInputFiles({
    name: "filename-disagrees.zip",
    mimeType: "application/zip",
    buffer: archive,
  });
  await page
    .getByRole("button", { name: "Import ZIP draft", exact: true })
    .click();
  await expect(
    page.getByRole("button", { name: "Intake fixture (draft)", exact: true }),
  ).toHaveAttribute("aria-pressed", "true");
}

/** Build a minimal import fixture with the maintained exporter, then remove its source draft. */
export async function minimalThemeZip(
  page: Page,
  manifestName = "Intake fixture",
): Promise<Buffer> {
  const form = new URLSearchParams({ scope: "author", name: "Fixture source" });
  appendBytes(
    form,
    "draft[manifest]",
    new TextEncoder().encode(
      `{"schema": 1, "name": ${JSON.stringify(manifestName)}, "style_contract": 1, "assets": {}, "defaults": {}}`,
    ),
  );
  appendBytes(
    form,
    "draft[stylesheet]",
    new TextEncoder().encode(
      '[data-jaunder-part="main"] { outline: 2px solid rgb(1, 2, 3); }',
    ),
  );
  const created = await page.request.post(`${BASE_URL}/api/themes/create`, {
    data: form.toString(),
    headers: { "content-type": "application/x-www-form-urlencoded" },
  });
  expect(created.status()).toBe(200);
  const entry = confirmedMutation(
    (await created.json()) as MutationOutcome<CatalogEntry>,
    "fixture create",
  );
  const target = new URLSearchParams({
    scope: "author",
    theme_id: String(entry.id),
  });
  const exported = await page.request.post(`${BASE_URL}/api/themes/export`, {
    data: target.toString(),
    headers: { "content-type": "application/x-www-form-urlencoded" },
  });
  expect(exported.status()).toBe(200);
  const archive = Buffer.from((await exported.json()).bytes as number[]);
  // The live API canonicalizes the manifest; use its exporter rather than hand-rolled ZIP headers.
  expect(archive.length).toBeGreaterThan(0);
  const removed = await page.request.post(`${BASE_URL}/api/themes/remove`, {
    data: target.toString(),
    headers: { "content-type": "application/x-www-form-urlencoded" },
  });
  expect(removed.status()).toBe(200);
  confirmedMutation(
    (await removed.json()) as MutationOutcome<null>,
    "fixture remove",
  );
  return archive;
}

/** Import and publish a complete local package through the live API. */
export async function publishTheme(
  page: Page,
  themePackage = conformanceThemePackage(),
  scope: "author" | "site" = "author",
): Promise<number> {
  const form = new URLSearchParams({ scope, name: "Conformance" });
  appendBytes(
    form,
    "draft[manifest]",
    new TextEncoder().encode(JSON.stringify(themePackage.manifest)),
  );
  appendBytes(
    form,
    "draft[stylesheet]",
    new TextEncoder().encode(themePackage.stylesheet),
  );
  for (const [index, asset] of themePackage.assets.entries()) {
    form.set(`draft[assets][${index}][path]`, asset.path);
    form.set(`draft[assets][${index}][mime]`, asset.mime);
    appendBytes(form, `draft[assets][${index}][bytes]`, asset.bytes);
  }
  const created = await page.request.post(
    `${BASE_URL}${THEME_ENDPOINTS.create}`,
    {
      data: form.toString(),
      headers: { "content-type": "application/x-www-form-urlencoded" },
    },
  );
  expect(created.status()).toBe(200);
  const theme = confirmedMutation(
    (await created.json()) as MutationOutcome<CatalogEntry>,
    "themes::create",
  );

  const mutation = new URLSearchParams({
    scope,
    theme_id: String(theme.id),
  });
  const response = await page.request.post(
    `${BASE_URL}${THEME_ENDPOINTS.publish}`,
    {
      data: mutation.toString(),
      headers: { "content-type": "application/x-www-form-urlencoded" },
    },
  );
  expect(response.status()).toBe(200);
  confirmedMutation(
    (await response.json()) as MutationOutcome<null>,
    "themes::publish",
  );
  return theme.id;
}

/** Import, publish, and select a complete local package through the live API. */
export async function publishAndSelectTheme(
  page: Page,
  themePackage = conformanceThemePackage(),
  scope: "author" | "site" = "author",
): Promise<number> {
  const themeId = await publishTheme(page, themePackage, scope);
  const mutation = new URLSearchParams({
    scope,
    "selection[kind]": "custom",
    "selection[value]": String(themeId),
  });
  const response = await page.request.post(
    `${BASE_URL}${THEME_ENDPOINTS.select}`,
    {
      data: mutation.toString(),
      headers: { "content-type": "application/x-www-form-urlencoded" },
    },
  );
  expect(response.status()).toBe(200);
  confirmedMutation(
    (await response.json()) as MutationOutcome<null>,
    "themes::select",
  );
  return themeId;
}

/** Read a ZIP member without extracting it, keeping export checks portable. */
export function packageMember(path: string, member: string): Buffer {
  return execFileSync("unzip", ["-p", path, member]);
}

/** Stable package member names and SHA-256 digests for export assertions. */
export function packageMemberDigests(path: string): Map<string, string> {
  const members = execFileSync("unzip", ["-Z1", path], { encoding: "utf8" })
    .split("\n")
    .filter(Boolean)
    .sort();
  return new Map(
    members.map((member) => [
      member,
      createHash("sha256").update(packageMember(path, member)).digest("hex"),
    ]),
  );
}

export function rgbChannels(color: string): [number, number, number] {
  const rgb = /^rgb\((\d+), (\d+), (\d+)\)$/.exec(color);
  if (rgb) return [Number(rgb[1]), Number(rgb[2]), Number(rgb[3])];

  // Computed color-mix() values serialize as color(srgb ...) in Chromium and
  // Firefox, rather than rounding to rgb() bytes.
  const srgb = /^color\(srgb ([\d.]+) ([\d.]+) ([\d.]+)\)$/.exec(color);
  expect(
    srgb,
    `expected an opaque computed sRGB color, got ${color}`,
  ).not.toBeNull();
  return [
    Number(srgb![1]) * 255,
    Number(srgb![2]) * 255,
    Number(srgb![3]) * 255,
  ];
}

function luminance([red, green, blue]: [number, number, number]): number {
  const linear = [red, green, blue].map((channel) => {
    const value = channel / 255;
    return value <= 0.03928 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * linear[0] + 0.7152 * linear[1] + 0.0722 * linear[2];
}

/** WCAG contrast ratio for opaque computed colors. */
export function contrastRatio(foreground: string, background: string): number {
  const [lighter, darker] = [
    luminance(rgbChannels(foreground)),
    luminance(rgbChannels(background)),
  ].sort((a, b) => b - a);
  return (lighter + 0.05) / (darker + 0.05);
}
