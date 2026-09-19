import { expect, type Page } from "@playwright/test";
import { BASE_URL, confirmedMutation, type MutationOutcome } from "./helpers";

export type UploadedMedia = { url: string; filename: string };

/** Uploads `name` and returns the upload response (`url`, canonical `filename`). */
export async function uploadMedia(
  page: Page,
  name: string,
  content: Buffer = Buffer.from("delete guard content"),
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
