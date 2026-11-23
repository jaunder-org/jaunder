import { execFileSync } from "node:child_process";
import * as path from "node:path";

/**
 * Resolve a capture-file absolute path by asking the `test-support` binary, so the
 * filename convention lives only in the Rust `host` crate and is never restated in
 * TypeScript. Mirrors `seed.ts`'s `seedPostsViaTool` — `test-support` is on PATH and
 * inherits `JAUNDER_CAPTURE_DIR` in both host and VM runs. Exits non-zero (throwing
 * here) if `JAUNDER_CAPTURE_DIR` is unset, so a misconfigured run fails loudly.
 *
 * `JAUNDER_CAPTURE_DIR` is this worker's server instance's `instance-<i>/`
 * subdirectory (#2076). The shared collector writes `otel` to the capture root,
 * that directory's parent.
 */
export function capturePathViaTool(
  stream: "mail" | "websub" | "diag" | "otel",
): string {
  const instanceDir = process.env.JAUNDER_CAPTURE_DIR;
  const env =
    stream === "otel" && instanceDir !== undefined
      ? { ...process.env, JAUNDER_CAPTURE_DIR: path.dirname(instanceDir) }
      : process.env;
  return execFileSync("test-support", ["capture-path", stream], {
    stdio: "pipe",
    env,
  })
    .toString()
    .trim();
}
