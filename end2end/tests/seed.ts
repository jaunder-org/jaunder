import { execFileSync } from "node:child_process";

/**
 * Seed `count` published posts for `username` via the `test-support` binary
 * (ADR-0045): one in-process storage write per post, no HTTP round-trip. Post
 * `i` renders an article H1 of `"${bodyPrefix} ${i}"`, so the timeline
 * assertions that key on post titles still hold. Runs synchronously; the tool
 * reads the target database from `JAUNDER_DB` in the environment (set by the nix
 * e2e harness, pointing at the same DB the server uses).
 *
 * On a non-zero exit `execFileSync` throws with the tool's stderr, surfacing a
 * seed failure as a test error rather than a silently empty timeline.
 */
export function seedPostsViaTool(
  username: string,
  count: number,
  bodyPrefix: string,
  opts: { published?: boolean } = {},
): void {
  const args = [
    "seed-posts",
    "--username",
    username,
    "--count",
    String(count),
    "--body-prefix",
    bodyPrefix,
  ];
  if (opts.published ?? true) args.push("--published");
  execFileSync("test-support", args, { stdio: "pipe", env: process.env });
}
