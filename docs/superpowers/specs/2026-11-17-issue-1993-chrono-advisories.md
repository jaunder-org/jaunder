# Issue #1993 — accept the chrono / chrono-tz unmaintained advisories

## Outcome

`cargo deny check` passes on `main` again. Two new RustSec "unmaintained"
advisories caused the failure: RUSTSEC-2026-0543 for `chrono` and
RUSTSEC-2026-0555 for `chrono-tz`. With the failure gone, the pre-commit gate
and PR merge checks stop failing. Each exception records why it exists and what
lets us remove it.

## Approach and tradeoffs

Add two entries to `deny.toml`'s `[advisories] ignore` list, using the existing
`{ id, reason }` form. Each reason states three things:

- The crate is transitive only. No first-party crate depends on `chrono` or
  `chrono-tz`, because the workspace already uses `jiff`.
- The advisory itself says no safe upgrade exists.
- The removal trigger, and the follow-up issue that tracks it.

The crates that pull in `chrono` (from `cargo tree -i chrono`):

| Crate                                  | Reached through                         |
| -------------------------------------- | --------------------------------------- |
| `atom_syndication` 0.12.10             | `rss` 2.1.1 → `host`                    |
| `atom_syndication` 0.13.0 (our fork)   | `host` (also a build-dependency)        |
| `axum-embed`                           | `jaunder` (server)                      |
| `croner` 4.0.0                         | `common`, `storage`, `jaunder` (direct) |
| `croner` 3.0.1, `tokio-cron-scheduler` | `jaunder`                               |
| `diligent-date-parser`                 | `atom_syndication` (both versions)      |
| `chrono-tz` (→ `chrono`)               | `tokio-cron-scheduler` → `jaunder`      |

Each entry has its own removal trigger:

- **`chrono`:** none of the crates above needs `chrono` any longer, because
  upstream migrated or because we replaced or forked the crate.
- **`chrono-tz`:** `tokio-cron-scheduler` no longer needs it, because it is
  upgraded or replaced. It is the only crate that brings in `chrono-tz`.

When a trigger is met, `cargo deny` warns that the ignore is unused, and we
delete the entry. That warning does not fail the gate, so the follow-up issue is
what keeps track of it.

Then file one follow-up issue through `jaunder-issues` for those triggers. It
lists the dependents above and notes two candidates we control or can replace:

- **Our `jaunder-org/atom` fork.**
- **Our direct `croner` use.** croner 4.0.2 makes chrono an optional default
  feature, and we only use its parser. Setting `default-features = false` alone
  would drop chrono from `common`.

The follow-up records the triggers. It does not schedule any migration.

**Rejected alternative:** setting `[advisories] unmaintained = "workspace"`. It
would also make the gate pass, but it silences every future transitive
"unmaintained" advisory without review. Per-ID ignores keep each exception
deliberate and visible, which matches the existing RUSTSEC-2026-0173 entry.

## Acceptance

- `traced-run -- cargo deny check` exits 0 on the branch. Today
  `cargo deny check advisories` fails on both IDs at `origin/main` 3a4fb513d.
- Both `deny.toml` entries name the reason, the removal trigger and the
  follow-up issue number.
- The follow-up issue exists, lists the dependents, and states the removal
  trigger.
- The pre-commit hook passes on the commit.

## Boundaries

- No dependency upgrades, replacements or fork changes. `Cargo.lock` stays
  unchanged.
- No changes to other `deny.toml` sections or other ignores.
- Rebasing open PRs is out of scope. They pick up the fix when they rebase.
