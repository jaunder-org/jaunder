import { execFileSync } from "node:child_process";

/**
 * Resolve a capture-file absolute path by asking the `test-support` binary, so the
 * filename convention lives only in the Rust `host` crate and is never restated in
 * TypeScript. Mirrors `seed.ts`'s `seedPostsViaTool` — `test-support` is on PATH and
 * inherits `JAUNDER_CAPTURE_DIR` in both host and VM runs. Exits non-zero (throwing
 * here) if `JAUNDER_CAPTURE_DIR` is unset, so a misconfigured run fails loudly.
 */
export function capturePathViaTool(stream: "mail" | "websub"): string {
  return execFileSync("test-support", ["capture-path", stream], {
    stdio: "pipe",
    env: process.env,
  })
    .toString()
    .trim();
}
