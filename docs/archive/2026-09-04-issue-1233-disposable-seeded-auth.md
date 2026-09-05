# Disposable Seeded-Auth Init Scripts

## Outcome

Seeded e2e authentication uses Playwright's disposable context init scripts
instead of a companion cookie and permanent dispatcher script. Re-seeding a
browser context deterministically replaces the prior identity while UI logout
continues to survive later document loads.

## Load-bearing decisions

- Track the current seeded-auth init script in a
  `WeakMap<BrowserContext, Disposable>`; one context owns at most one live
  seeded-auth script.
- Every seed disposes the prior script before installing its replacement. Keep
  the prior map entry when disposal rejects. After disposal succeeds, remove
  that entry before installation; if installation rejects, leave the context
  untracked. Store the new disposable immediately after installation succeeds.
  No fallback or stacked script is permitted.
- Bake the Rust-produced marker key and marker value, plus a fresh per-seed
  nonce, into each replacement script. Remove the readable companion cookie and
  its parsing path entirely.
- Retain the localStorage tombstone. The replacement applies its marker only
  when its nonce differs from the tombstone, then records that nonce. This keeps
  later loads after UI logout from restoring stale authenticated chrome while a
  same-user re-seed still reapplies the marker.
- Track a successfully installed replacement before injecting the session
  cookie. If cookie injection fails, reject while retaining ownership of the new
  disposable so the next seed can dispose it; do not add rollback behavior that
  can obscure the original failure.
- Browser-context closure remains Playwright's resource boundary. Add no close
  wrapper or explicit context teardown hook.
- Preserve ADR-0098's real register/login holdout matrix and update that ADR's
  seeded-script description for the now-pinned Playwright API.

## Acceptance

- `applySeededSession` contains no companion marker cookie or `WeakSet`; it owns
  the current Playwright `Disposable` through a `WeakMap`.
- First seed, same-user re-seed after UI logout, and post-logout full navigation
  retain their current pre-paint behavior.
- A real-browser test seeds one context as user A, re-seeds it as user B, and
  proves both a later navigation and a subsequently created page boot only as
  user B.
- Focused helper-level tests use controllable context operations to prove
  disposal → installation → cookie ordering, exactly one retained live handle,
  error propagation, and the specified map state and retry behavior after
  disposal, installation, and session-cookie failures.
- ADR-0098 describes the disposable replacement design while retaining its
  holdout table unchanged.
- `devtool run -- cargo xtask e2e-local authed-flash.spec.ts` passes.

## Boundaries

- No Playwright or Nix dependency update; issue #815 already owns and delivered
  version alignment.
- No production authentication, session-cookie, marker codec, or browser-client
  behavior changes.
- No second context lifecycle wrapper, generic disposable registry, or seeded
  auth teardown API.
- No removal or substitution of ADR-0098's genuine register/login flow holdouts.
