import { execFileSync } from "node:child_process";

/**
 * Seed `count` published posts for `username` via the `test-support` binary
 * (ADR-0046): one in-process storage write per post, no HTTP round-trip. Post
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

/**
 * Set a single site-config key/value via the `test-support set-site-config`
 * subcommand (ADR-0046) — the same in-process storage write the canonical e2e
 * seed uses (`devtool seed-e2e` sets `site.registration_policy=open` this way).
 * The running server reads site config live per request, so a test can flip
 * `site.registration_policy` to `invite_only` (or set `site.base_url`) and the
 * next request observes it. There is no UI for the registration policy, so this
 * is the only seam. Global mutation — a spec that calls this must run isolated
 * from parallel specs that read the same key (see playwright.config's `-admin`
 * projects).
 *
 * On a non-zero exit `execFileSync` throws with the tool's stderr, surfacing a
 * misconfigured write as a test error.
 */
export function seedConfigViaTool(key: string, value: string): void {
  execFileSync(
    "test-support",
    ["set-site-config", "--key", key, "--value", value],
    {
      stdio: "pipe",
      env: process.env,
    },
  );
}
