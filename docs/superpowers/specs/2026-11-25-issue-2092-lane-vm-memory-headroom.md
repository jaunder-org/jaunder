# Lane VM memory headroom with per-worker server instances (#2092)

## Outcome

Every E2E lane VM records how much guest memory it used and whether it ever came
under memory pressure, so a memory-related CI failure can be diagnosed from the
lane's artifacts alone. With that evidence, the Firefox and Chromium lane sizing
is either confirmed for two server instances per VM (#2076) or raised, and the
workers=2 and workers=4 measurement controls are sized to match.

## Approach and tradeoffs

Today a lane records only one kind of memory figure: the "memory peak" that
systemd writes to the journal for each `jaunder-<i>` and collector unit when the
unit stops. Nothing records any of the following:

- the browsers and Playwright, which use most of the guest's memory;
- PostgreSQL, which is never stopped, so systemd writes no peak for it;
- how close the guest came to running out.

#2076 printed per-unit peaks while measuring, then removed them. That removal is
treated as a mistake and reversed.

Every lane built by `mkE2eCombo` gains the two additions below. That covers the
gate shards, the workers=2/workers=4 controls, the single-worker diagnostics and
the upgrade check. Performance producers and the wasm-coverage checks are
unchanged.

1. **`lane-memory-sampler.service`.** A small guest unit that starts before the
   server instances. Once a second it logs the guest's `MemAvailable` (read from
   `/proc/meminfo`) to the system journal, which is already uploaded as
   `system-journal-<lane>.log`.
2. **A memory summary.** After Playwright exits, and before any writer stops,
   the lane writes `lane-memory-<lane>.txt` and prints it to the build log. It
   must run first because `systemctl show` no longer reports the peak of a
   stopped unit. The file is lifted beside the other lane diagnostics and
   contains:
   - the guest's `MemTotal`, and the minimum `MemAvailable` that the sampler
     logged;
   - memory pressure from `/proc/pressure/memory`, the `some` and `full` stall
     totals, which the kernel accumulates since boot;
   - `oom_kill`, `allocstall_*` and `pgscan_direct` from `/proc/vmstat`;
   - `MemoryPeak` for each server instance, the collector, `postgresql.service`
     (when present) and `backdoor.service`. `backdoor.service` is the test
     driver's shell, which runs Playwright, its browsers, seeding and the
     census. These peaks include page cache.

These are diagnostics only. A missing or partial summary never fails a lane, and
no gate, threshold or reconciliation reads it. In particular:

- If the upgrade check's baseline phase fails, it stops before the summary is
  written.
- The upgrade check's instance peak covers only the upgraded phase.

Turning memory pressure into a lane failure would be new machinery that no
observed failure needs.

**Why a sampler is needed.** The minimum `MemAvailable` has no counter in the
kernel, and a cgroup peak is inflated by page cache, so neither answers how
close a lane came. The pressure and vmstat counters are one read at the end of
the run, but they show only that pressure happened, not how much memory was
left. Sampling once a second can miss a brief dip between samples; the stall
counters and OOM count still capture it.

**Sizing rule.** A low `MemAvailable` on its own is not treated as danger.
Browsers size their caches to the memory available, and #2076 saw 126 MiB on a 4
GiB Postgres Firefox lane without an OOM. A size is raised only when a run shows
either:

- an OOM kill (`oom_kill > 0`, or a kernel OOM line in the journal); or
- material reclaim stalls: memory pressure `full` totaling at least 1% of the
  Playwright wall time.

Any raise is 1 GiB, and the measurement is repeated.

The four control packages are each built locally. They change size as follows:

| Package                                           | Guest memory     | vCPU |
| ------------------------------------------------- | ---------------- | ---- |
| `e2e-{sqlite,postgres}-firefox-workers-2-control` | 3 GiB, now 5 GiB | 2    |
| `e2e-{sqlite,postgres}-firefox-workers-4-control` | 6 GiB            | 4    |

The first measured runs triggered the workers=2 raise: at 3 GiB, sqlite reached
57 s of `full` stall over 4,500 s, with active direct reclaim. At 4 GiB,
postgres was OOM-killed 22 minutes in: its two Firefox parents, three Playwright
Node processes and two content processes held about 3.2 GiB, so the control went
up again to 5 GiB. Those same runs also showed that the controls could not
finish the whole unsplit suite, so they get two resources of their own:

- **Disk:** a 4 GiB root disk. The whole suite's temporary files filled both the
  default 1 GiB and the Firefox shards' 2 GiB.
- **Time:** a 2-hour Playwright budget. Clean runs took 23 to 43 minutes, and
  the 25-minute per-shard cap was far too short.

The gate sizes (Firefox 4 GiB, Chromium 3 GiB) stay unless the CI window or the
controls meet the rule above.

**A state-rule bug found by the controls.** `/login` shows a passkey
availability notice, an `.error` element, when the site's `base_url` is unset.
`admin-site.spec.ts` clears `base_url` and, under #2076's state rule, never
restores it. Two `auth.spec.ts` tests then fail on that instance:

- `login page shows form` fails its screenshot comparison;
- `login invalid fields do not dispatch` counts three errors instead of two.

Those are the merge-queue failures earlier attributed to #2095. At the user's
decision the fix is part of this issue, and it uses the state rule's two moves:

- The seeded visual appearance now includes the seeded `base_url`.
- The password-login tests stop counting the passkey card's notice.

## Acceptance

- Proof to execute:
  - On the PR's CI run, every lane artifact contains `lane-memory-<lane>.txt`
    with the fields above, and each `system-journal-<lane>.log` contains the
    sampler's lines.
  - All four control packages pass once locally. Their summaries are posted on
    #2092.
  - A local run of `admin-site.spec.ts` followed by `auth.spec.ts` on a single
    instance fails before the login fix and passes after it.
  - About 10 merge-queue runs after #2076 show no OOM kill and no memory-related
    failure. So far 6 are clean: 96 lanes with no OOM, kill or browser crash.
    Each run is cited on #2092. Runs after this change lands also report their
    stall totals.
- Deliverables to commit:
  - The sampler and the summary.
  - The control resizing: workers=2 at 5 GiB, and both controls with a 4 GiB
    disk and a 2-hour Playwright budget.
  - The login-test state fix.
  - Sizing text describing the instrumentation and any change, in
    `CONTRIBUTING.md` (lane resources) and in the per-worker instances ADR
    draft's consequences.

## Boundaries

- No change to worker counts, vCPUs or shard layout. Gate disk sizes are
  unchanged.
- No memory gate, alert, or host-side parser of the summary.
- No retained measurement reports in the tree. Results live on the issue and the
  PR.
