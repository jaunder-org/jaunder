import { expect, type Page } from "@playwright/test";
import { BASE_URL, confirmedMutation, type MutationOutcome } from "./helpers";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";

export const imageFixtureDirectory =
  process.env.JAUNDER_TEST_IMAGE_DIR ??
  resolve(__dirname, "../../server/tests/fixtures/image-privacy");
// This is an already-clean INPUT, not a golden output of the native editor.
// Read only during a test: hermetic census imports run before fixture binding.
export function cleanImage(): Buffer {
  return readFileSync(resolve(imageFixtureDirectory, "jpeg-clean.jpg"));
}

export type UploadedMedia = { url: string; filename: string };

/** Uploads `name` and returns the upload response (`url`, canonical `filename`). */
export async function uploadMedia(
  page: Page,
  name: string,
  content: Buffer = cleanImage(),
  mimeType = "image/jpeg",
): Promise<UploadedMedia> {
  const response = await page.request.post(BASE_URL + "/api/media/upload", {
    multipart: {
      file: {
        name,
        mimeType,
        buffer: content,
      },
    },
  });
  expect(response.status()).toBe(200);
  return confirmedMutation(
    (await response.json()) as MutationOutcome<UploadedMedia>,
    "media::upload",
  );
}
