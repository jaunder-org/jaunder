import { execFileSync } from "node:child_process";

import type { BrowserContext } from "@playwright/test";

import { withTimedAction } from "./actions";
import { BASE_URL } from "./helpers";

/**
 * The `test-support` seed record (one JSON line on stdout), mapped to
 * camelCase at the parse boundary. Everything a browser context needs to boot
 * authenticated pre-paint: the session cookie and the advisory marker, both
 * built by the server's own Rust primitives so they are never restated here
 * (#791, AC2).
 */
export type SeedRecord = {
  username: string;
  userId: number;
  isOperator: boolean;
  token: string;
  /** Full `Set-Cookie` value from the server's own `session_cookie_header`. */
  setCookie: string;
  /** The localStorage key the marker belongs under (Rust-owned). */
  markerKey: string;
  /** The advisory marker JSON, from `common::session_user::encode_marker`. */
  marker: string;
};

/** The subset `applySeededSession` needs — `fixtures.ts`'s `TestUser` also
 *  satisfies it. */
export type SeededSession = Pick<SeedRecord, "setCookie" | "marker" | "markerKey">;

/** Companion cookie carrying the marker payload to the init script. Named to
 *  stay clear of AC2's `jaunder_auth` rg check — the marker key itself is
 *  never spelled in TypeScript; it arrives in the seed record. */
const SEED_MARKER_COOKIE = "jaunder_seed_marker";
/** localStorage tombstone: the marker value the init script last applied. */
const SEED_APPLIED_KEY = "jaunder_seed_applied";

/** Contexts whose tombstoned init script is already registered — one script
 *  per context (Playwright 1.58.2 cannot remove init scripts, spec D3). */
const scriptedContexts = new WeakSet<BrowserContext>();

/** Run a `test-support` session subcommand and parse its one-line JSON
 *  record. `--db` comes from `JAUNDER_DB` in the environment in both
 *  harnesses, exactly like `seedPostsViaTool` below. A non-zero exit throws
 *  with the tool's stderr, surfacing a seed failure as a test error. */
function runSeedTool(args: string[]): SeedRecord {
  const stdout = execFileSync("test-support", args, {
    stdio: "pipe",
    env: process.env,
    encoding: "utf8",
  });
  const raw = JSON.parse(stdout) as Record<string, unknown>;
  return {
    username: raw.username as string,
    userId: raw.user_id as number,
    isOperator: raw.is_operator as boolean,
    token: raw.token as string,
    setCookie: raw.set_cookie as string,
    markerKey: raw.marker_key as string,
    marker: raw.marker as string,
  };
}

/** Create a fresh account + session out-of-band (real storage path, genuinely
 *  argon2-hashed password) and return the seed record. Page-less and timed
 *  (spec D7): the `user` fixture no longer owns a page. */
export async function seedUserViaTool(
  username: string,
  password: string,
): Promise<SeedRecord> {
  return withTimedAction(null, "tool.users.seed", async () =>
    runSeedTool(["seed-user", "--username", username, "--password", password]),
  );
}

/** Create a session for an EXISTING account (e.g. the harness-seeded
 *  `testoperator`); the record's `isOperator` is read back from the user row,
 *  so the marker matches what a real login would write. */
export async function createSessionViaTool(
  username: string,
): Promise<SeedRecord> {
  return withTimedAction(null, "tool.sessions.create", async () =>
    runSeedTool(["create-session", "--username", username]),
  );
}

/**
 * Inject a seeded session into `context`: the session cookie, plus a readable
 * companion cookie that carries the marker payload to one tombstoned init
 * script registered per context (spec D3). Does NOT navigate (spec D5) — the
 * caller's first `goto` is the cold navigation.
 *
 * The init script runs before the document's own pre-paint `<head>` script on
 * every document load, and applies the companion cookie's marker only when it
 * differs from what it last applied (the tombstone in localStorage). That is
 * what makes it correct without call-site cooperation: later navigations are a
 * no-op (the app owns the marker), a UI logout is respected (the app's removal
 * is not re-applied), and a re-seed as another user replaces the marker.
 */
export async function applySeededSession(
  context: BrowserContext,
  session: SeededSession,
): Promise<void> {
  // Parse the server-emitted Set-Cookie value; only the pair and the Path
  // attribute are read — the other attributes are never restated here (AC2).
  const [pair, ...attrs] = session.setCookie.split("; ");
  const eq = pair.indexOf("=");
  const name = pair.slice(0, eq);
  const value = pair.slice(eq + 1);
  const path =
    attrs
      .find((a) => a.toLowerCase().startsWith("path="))
      ?.slice("path=".length) ?? "/";
  // `addCookies` rejects `url` combined with `domain`/`path`, and the server's
  // header carries no Domain — so the origin's hostname is spelled here.
  const domain = new URL(BASE_URL).hostname;

  await context.addCookies([
    { name, value, domain, path, httpOnly: true, sameSite: "Lax" },
    {
      name: SEED_MARKER_COOKIE,
      value: encodeURIComponent(session.marker),
      domain,
      path: "/",
      httpOnly: false,
      sameSite: "Lax",
    },
  ]);

  if (!scriptedContexts.has(context)) {
    scriptedContexts.add(context);
    await context.addInitScript(`(() => {
  const prefix = ${JSON.stringify(SEED_MARKER_COOKIE)} + "=";
  let want = null;
  for (const part of document.cookie.split("; ")) {
    if (part.startsWith(prefix)) {
      want = decodeURIComponent(part.slice(prefix.length));
      break;
    }
  }
  if (want === null) return;
  if (localStorage.getItem(${JSON.stringify(SEED_APPLIED_KEY)}) === want) return;
  localStorage.setItem(${JSON.stringify(session.markerKey)}, want);
  localStorage.setItem(${JSON.stringify(SEED_APPLIED_KEY)}, want);
})();`);
  }
}

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
export async function seedPostsViaTool(
  username: string,
  count: number,
  bodyPrefix: string,
  opts: { published?: boolean } = {},
): Promise<void> {
  await withTimedAction(null, "tool.posts.seed", async () => {
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
  });
}

/**
 * Set a single site-config key/value via the shipped `jaunder site-config set`
 * subcommand (#8) — the same in-process storage write the canonical e2e seed
 * uses (`devtool seed-e2e` sets `site.registration_policy=open` this way). The
 * running server reads site config live per request, so a test can flip
 * `site.registration_policy` to `invite_only` (or set `site.base_url`) and the
 * next request observes it. There is no UI for the registration policy, so this
 * is the only seam. Global mutation — a spec that calls this must run isolated
 * from parallel specs that read the same key (see playwright.config's `-admin`
 * projects).
 *
 * Resolves bare `jaunder` on PATH: the flake VM adds `jaunderBin` to
 * `systemPackages`, the host loop prepends `target/debug`. It reads the target
 * database from `JAUNDER_DB` (set by the harness). On a non-zero exit
 * `execFileSync` throws with the tool's stderr, surfacing a misconfigured write
 * as a test error.
 */
export async function seedConfigViaTool(
  key: string,
  value: string,
): Promise<void> {
  await withTimedAction(null, "tool.config.set", async () => {
    execFileSync("jaunder", ["site-config", "set", key, value], {
      stdio: "pipe",
      env: process.env,
    });
  });
}
