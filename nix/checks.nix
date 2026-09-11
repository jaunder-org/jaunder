{ self, system, pkgs, nixosInternals, packageInternals }:
let
  inherit (nixosInternals) captureEnv e2eOtelCollectorEnv;
  inherit (packageInternals)
    visualFontConfig
    toolchain
    craneLib
    commonArgs
    hostArgs
    wasmTestSrc
    siteSrc
    appOfflineCargoHome
    workspaceMembers
    cargoMemberSource
    cargoPackageClosure
    toolsOfflineCargoHome
    cargoArtifacts
    leanTestProfile
    leanDevAndTestProfile
    jaunderBin
    testSupportBin
    devtoolBin
    cargo-crap
    wasm-bindgen-cli
    wasmTestWebdriverConfig
    leptosfmt
    csrWasmBundle
    e2ePackage
    diagnosticJaunderBin
    diagnosticBaselineJaunderBin
    diagnosticCsrWasmBundle
    diagnosticBaselineCsrWasmBundle
    emacsSrc
    emacsForCi
    ;
  # The root workspace remains the coverage population. Its Cargo manifests
  # define the recursively discovered local path package build closure.
  coverageMembers = cargoPackageClosure workspaceMembers;
  # Coverage source remains bounded to Cargo-recognized package inputs plus the
  # explicit nextest profile, SQLx migration trees and rust-embed assets consumed
  # at compile time, and the immutable backup compatibility corpus consumed at
  # runtime through CARGO_MANIFEST_DIR.
  coverageAuxiliarySource =
    relative:
    relative == ".config/nextest.toml"
    || pkgs.lib.hasPrefix "server/assets/" relative
    || pkgs.lib.hasPrefix "storage/migrations/" relative
    || pkgs.lib.hasPrefix "server/tests/misc/backup_corpus/" relative;
  coverageSrc =
    # Pure source-filter negative case: excluded auxiliary assets cannot perturb
    # coverage source identity.
    assert !(coverageAuxiliarySource "tools/devtool/fixture.css");
    assert !(coverageAuxiliarySource "server/notes.txt");
    assert (coverageAuxiliarySource "server/tests/misc/backup_corpus/index.json");
    assert (coverageAuxiliarySource "storage/migrations/sqlite/0001_create_site_config.sql");
    assert (coverageAuxiliarySource "server/assets/jaunder.css");
    assert builtins.elem "tools/csr_bundle" coverageMembers;
    assert !(builtins.elem "xtask" coverageMembers);
    assert !(builtins.elem "tools/devtool" coverageMembers);
    assert !(builtins.elem "tools/doctests" coverageMembers);
    assert !(builtins.elem "tools/diagnostic-coverage-runtime" coverageMembers);
    pkgs.lib.cleanSourceWith {
      src = craneLib.path ../.;
      filter =
        path: type:
        let
          relative = pkgs.lib.removePrefix "${toString ../.}/" (toString path);
        in
        # Nix assembly is not coverage source; exclude only its top-level root.
        !(type == "directory" && path == "${toString (craneLib.path ../.)}/nix")
        && !(pkgs.lib.hasInfix "/xtask/" path)
        && !(pkgs.lib.hasInfix "/docs/" path)
        && !(pkgs.lib.hasInfix "/.github/" path)
        && !(pkgs.lib.hasInfix "/elisp/" path)
        && !(pkgs.lib.hasSuffix ".md" path)
        && (
          coverageAuxiliarySource relative
          || cargoMemberSource coverageMembers path type
        );
    };

# #93 / ADR-0032: shared zero-panic gate appended to each e2e testScript.
# A server Rust panic is isolated (tests still pass), so without this it
# gets cached green and stays invisible. Dump the service journal and copy it
# before running the shared Rust verifier from `test-support`. The caller
# records the verifier result before asserting it, so the timing sidecar remains
# recoverable without changing the established panic-before-Playwright failure
# order. It scans raw bytes from the union of the scoped diagnostic stream
# (#144/#227) and the journal fallback, de-duplicates by panic location with the
# scoped record winning, and owns the default-empty source-controlled allowlist.
# The CLI receives the capture directory rather than restating the diagnostic
# filename defined by `host::capture`.
e2ePanicGate = backend: ''
  machine.succeed("journalctl -u jaunder.service --no-pager -o cat > /tmp/jaunder-journal-${backend}.log")
  # copy_from_machine's 2nd arg is a target *directory*; "" lands the file
  # flat at $out/jaunder-journal-${backend}.log (the per-backend name comes
  # from the source).
  machine.copy_from_machine("/tmp/jaunder-journal-${backend}.log", "")
  panic_status, panic_out = machine.execute(
      "test-support verify-no-panics"
      + " --capture-dir /var/lib/jaunder/capture"
      + " --server-log /tmp/jaunder-journal-${backend}.log"
  )
  print(panic_out)
'';

# The two e2e time budgets, which must stay ordered:
# `e2ePlaywrightTimeout` < `e2eGlobalTimeout`.
#
# Playwright runs under `machine.execute`, whose driver default is
# `timeout=900` — passing no `timeout=` silently caps the Playwright step
# at 15 min. Both budgets are named here so the cap is explicit and the
# ordering is checkable (#130).
#
# The ordering is load-bearing, not cosmetic: when Playwright is the thing
# that expires, `machine.execute` returns 124 and the artifact copies below
# still run (that is why this uses `execute`, not `succeed`). If the driver's
# `globalTimeout` expired first it would kill the VM outright and take every
# diagnostic with it — the exact failure #123/#49 built this path to avoid.
# The difference is the boot + seed + copy allowance; measured overhead is
# ~40 s, so 180 s is ~4x headroom. Firefox 151 completes the 258-test suite in
# 12.5 min alone but exhausted 17 min under the full validation matrix, so the
# inner budget allows 25 min for that supported concurrent execution path.
e2ePlaywrightTimeout = 1500;
e2eGlobalTimeout = 1680;

# #123/#49: run Playwright capturing its exit (NOT machine.succeed, which
# would abort before we copy diagnostics), stream its line-reporter output
# to the build log, copy ALL artifacts out of the VM unconditionally, then
# fail the check only after the copies are safe. On success the copies land
# in $out; on failure they live in the --keep-failed build dir for xtask's
# rescue_diagnostics to recover. Shared by both backends so they can't drift.
# VM-local OTel test glue shared by both backends. A systemd unit becomes
# active when the collector process is spawned, before its receivers are
# necessarily listening (#1243), so every initial start and restart probes
# the actual endpoints before any exporter runs. Seed-span verification
# then stops the collector to flush short-lived process spans into the
# JSONL file the VM owns.
e2eOtelTestHelpers = ''
  def wait_for_otel_receivers():
    machine.wait_for_open_port(4317, timeout=30)
    machine.wait_for_open_port(4318, timeout=30)

  def assert_seed_storage_spans():
    import json
    machine.succeed("systemctl stop otel-collector.service")
    raw = machine.succeed("test -s /var/lib/jaunder/capture/otel-traces.jsonl && cat /var/lib/jaunder/capture/otel-traces.jsonl")
    wanted = {"e2e.seed.jaunder", "e2e.seed.test-support"}
    seen = set()
    for line_number, line in enumerate(raw.splitlines(), 1):
      try:
        record = json.loads(line)
      except json.JSONDecodeError as error:
        raise AssertionError("malformed seed otel-traces.jsonl line %d: %s" % (line_number, error)) from error
      for resource_span in record.get("resourceSpans", []):
        attrs = {
          attr.get("key"): attr.get("value", {}).get("stringValue", "")
          for attr in resource_span.get("resource", {}).get("attributes", [])
        }
        process = attrs.get("jaunder.e2e.seed_process")
        if process not in wanted:
          continue
        for scope_span in resource_span.get("scopeSpans", []):
          if any(span.get("name", "").startswith("storage.") for span in scope_span.get("spans", [])):
            seen.add(process)
    missing = sorted(wanted - seen)
    assert not missing, "seed trace lacks storage spans for: %s" % ", ".join(missing)
    machine.succeed("systemctl start otel-collector.service")
    machine.wait_for_unit("otel-collector.service", timeout=60)
    wait_for_otel_receivers()
    machine.succeed("systemctl start jaunder.service")
    machine.wait_for_unit("jaunder.service", timeout=60)
    machine.wait_for_open_port(3000, timeout=30)
'';

# The NixOS test driver is a Python process, so monotonic timing is available
# without adding a guest dependency. Nix evaluation and realization happen
# before that process starts: preserve those required vocabulary entries as
# explicit unavailable evidence instead of inventing a boundary the VM cannot
# observe. The sidecar lives in the VM long enough to use the established
# diagnostic copy path, keeping successful and --keep-failed outputs identical.
e2ePhaseTimingHelpers = backend: browser: ''
  import base64
  import json
  import shlex
  import time

  e2e_phases = [
    {
      "name": "nix-evaluation",
      "duration_ms": None,
      "outcome": "unavailable",
      "detail": "Nix evaluation completes before the NixOS test driver starts.",
      "nix": {
        "classification": "unknown",
        "detail": "The VM cannot observe Nix evaluation.",
      },
    },
    {
      "name": "nix-substitution",
      "duration_ms": None,
      "outcome": "unavailable",
      "detail": "Nix substitution completes before the NixOS test driver starts.",
      "nix": {
        "classification": "unknown",
        "detail": "The VM cannot observe Nix substitution.",
      },
    },
    {
      "name": "nix-local-build",
      "duration_ms": None,
      "outcome": "unavailable",
      "detail": "Nix realization completes before the NixOS test driver starts.",
      "nix": {
        "classification": "unknown",
        "detail": "The VM cannot distinguish a local build from another realization path.",
      },
    },
  ]

  def record_e2e_phase(name, started_at, outcome, detail):
    e2e_phases.append(
      {
        "name": name,
        "duration_ms": int((time.monotonic() - started_at) * 1000),
        "outcome": outcome,
        "detail": detail,
      }
    )

  def write_e2e_phase_manifest():
    payload = base64.b64encode(
      json.dumps(
        {
          "schema_version": 1,
          "backend": "${backend}",
          "browser": "${browser}",
          "phases": e2e_phases,
        },
        separators=(",", ":"),
      ).encode()
    ).decode()
    # Capturing timing must never become the failure reported instead of a
    # Playwright or panic failure. The following unconditional lift still
    # exposes a successfully written sidecar in either output location.
    manifest_status, manifest_out = machine.execute(
      "printf %s " + shlex.quote(payload)
      + " | base64 -d > /tmp/e2e-phase-${backend}.json"
    )
    if manifest_status != 0:
      print("failed to write e2e phase manifest: " + manifest_out)
'';

e2eRunAndCapture =
  {
    backend,
    browser,
    traceId,
    traceParent,
    # The same DB the running server uses, exported into the Playwright
    # process env so the `test-support` seed helper it spawns points at
    # that DB (it reads `JAUNDER_DB`). Backend-specific; see each check.
    jaunderDb,
    extraEnv ? "",
  }:
  ''
    gate_started_at = time.monotonic()
    pw_status, pw_out = machine.execute(
      "cd /tmp/e2e"
      + " && PLAYWRIGHT_BROWSERS_PATH=${pkgs.playwright-driver.browsers}"
      + " PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD=1"
      + " FONTCONFIG_FILE=${visualFontConfig}"
      + "${extraEnv}"
      + " JAUNDER_CAPTURE_DIR=/var/lib/jaunder/capture"
      + " JAUNDER_DB=${jaunderDb}"
      + " JAUNDER_STORAGE_PATH=/var/lib/jaunder/data"
      + " JAUNDER_E2E_TRACE_ID=${traceId}"
      + " JAUNDER_E2E_TRACEPARENT=${traceParent}"
      + " JAUNDER_E2E_OTLP_HTTP_ENDPOINT=http://127.0.0.1:4318/v1/traces"
      + " ${pkgs.nodejs}/bin/node node_modules/.bin/playwright test"
      + " --config playwright.config.ts"
      + " --project ${browser} --project ${browser}-admin",
      timeout=${toString e2ePlaywrightTimeout},
    )
    record_e2e_phase(
      "gate-execution",
      gate_started_at,
      "success" if pw_status == 0 else "failed",
      "${backend}/${browser}: Playwright exited with status %d." % pw_status,
    )
    # Stream the Playwright line-reporter output into the build log (-L), so
    # the failing test + assertion are recoverable from build.log alone,
    # even on failure and without --keep-failed.
    print(pw_out)

    # Stop otel so its trace flushes; ignore status (best-effort capture).
    machine.execute("systemctl stop otel-collector.service")

    result_lift_started_at = time.monotonic()

    # Copy every diagnostic UNCONDITIONALLY, each guarded so a missing file
    # (e.g. an early crash) never aborts the remaining copies.
    # copy_from_machine's 2nd arg is a target *dir*; "" lands the file flat
    # under the per-backend name carried by the source.
    def _grab(path):
        if machine.execute("test -e " + path)[0] == 0:
            machine.copy_from_machine(path, "")

    machine.execute("test -s /tmp/e2e/test-results/results.json && cp /tmp/e2e/test-results/results.json /tmp/playwright-report-${backend}.json")
    _grab("/tmp/playwright-report-${backend}.json")
    machine.execute("test -s /tmp/e2e/test-results/duration-budget-manifest.json && cp /tmp/e2e/test-results/duration-budget-manifest.json /tmp/duration-budget-manifest-${backend}.json")
    _grab("/tmp/duration-budget-manifest-${backend}.json")

    machine.execute("tar czf /tmp/playwright-artifacts-${backend}.tar.gz -C /tmp/e2e test-results 2>/dev/null || true")
    _grab("/tmp/playwright-artifacts-${backend}.tar.gz")

    machine.execute("journalctl --no-pager -o short-precise > /tmp/system-journal-${backend}.log")
    _grab("/tmp/system-journal-${backend}.log")

    # Capture-dir contract (#227, #332): tar the whole capture dir out per combo as
    # capture-${backend}.tar.gz — a file copy mirroring the playwright-artifacts
    # tarball (the proven copy_from_machine shape). Holds diag.log, the collector's
    # otel-traces.jsonl (#332 — the collector is stopped above, so its file export is
    # flushed), plus any written mail.jsonl/websub.jsonl. The in-VM zero-panic gate
    # reads diag.log directly, so it does not depend on this lift.
    machine.execute("test -d /var/lib/jaunder/capture && tar czf /tmp/capture-${backend}.tar.gz -C /var/lib/jaunder capture 2>/dev/null || true")
    _grab("/tmp/capture-${backend}.tar.gz")
    record_e2e_phase(
      "result-lift",
      result_lift_started_at,
      "success",
      "${backend}/${browser}: copied available Playwright and service diagnostics.",
    )

    post_gate_started_at = time.monotonic()
    ${e2ePanicGate backend}
    record_e2e_phase(
      "post-gate-checks",
      post_gate_started_at,
      "success" if panic_status == 0 else "failed",
      "${backend}/${browser}: zero-panic verifier exited with status %d." % panic_status,
    )
    write_e2e_phase_manifest()
    # The sidecar follows the established unconditional diagnostic lift. If a
    # guest-side write failed, _grab deliberately does not hide the original
    # Playwright or panic verdict while still preserving every other artifact.
    _grab("/tmp/e2e-phase-${backend}.json")

    # Preserve ADR-0032's panic assertion before the Playwright assertion.
    assert panic_status == 0, "e2e zero-panic gate failed (exit %d) for ${backend}/${browser}; see jaunder-journal-${backend}.log + e2e-phase-${backend}.json + build.log" % panic_status

    # Fail the check now — after all artifacts are safely copied out.
    assert pw_status == 0, "e2e Playwright failed (exit %d) for ${backend}/${browser}; see playwright-report-${backend}.json + duration-budget-manifest-${backend}.json + playwright-artifacts-${backend}.tar.gz + e2e-phase-${backend}.json + build.log" % pw_status
  '';

mkE2eCheck =
  {
    backend,
    checkName,
    browser,
    traceId,
    traceParent,
    extraEnv ? "",
    vmMemory ? 2048,
    vmCores ? null,
  }:
  let
    backendPolicy =
      if backend == "sqlite" then
        {
          package = pkgs.sqlite;
          jaunderDb = "sqlite:/var/lib/jaunder/data/jaunder.db";
          nodeConfig = _: { };
          setupBeforeJaunder = "";
          seedBeforeStart = true;
          seedComments = [
            "  # Seed the fresh VM's already-migrated DB. This VM is single-use and"
            "  # jaunder.service's boot preStart (`jaunder init`) has already created"
            "  # and migrated an empty DB (incl. migration 0018 reference data);"
            "  # nothing writes user data before this point, so no wipe is needed"
            "  # (#271). Seeding runs against the running boot service."
          ];
        }
      else if backend == "postgres" then
        {
          package = pkgs.postgresql_18;
          jaunderDb = "postgres://jaunder:testpassword@127.0.0.1/jaunder";
          nodeConfig = lib: {
            services.postgresql = {
              enable = true;
              package = pkgs.postgresql_18;
              authentication = ''
                local all all trust
                host all all 0.0.0.0/0 trust
              '';
              settings = {
                listen_addresses = lib.mkForce "*";
              };
            };
            services.jaunder.db = "postgres://jaunder:testpassword@127.0.0.1/jaunder";
          };
          setupBeforeJaunder = ''
            machine.wait_for_unit("postgresql.service", timeout=60)

            machine.succeed(
              "${jaunderBin}/bin/jaunder create-pg-db"
              + " --bootstrap-db postgres://postgres@127.0.0.1/postgres"
              + " --app-db postgres://jaunder@127.0.0.1/jaunder"
              + " --app-role-password testpassword"
            )
          '';
          seedBeforeStart = false;
          seedComments = [
            "  # Seed the fresh VM's already-migrated DB. This VM is single-use;"
            "  # create-pg-db + the delayed jaunder.service boot preStart"
            "  # (`jaunder init`) have already created and migrated an empty DB"
            "  # (incl. migration 0018 reference data), and nothing writes user data"
            "  # before this point, so no TRUNCATE is needed (#271)."
          ];
        }
      else
        throw "unsupported e2e backend `${backend}`";
    seedDefinition = pkgs.lib.concatStringsSep "\n" (
      [
        "def seed_db():"
      ]
      ++ backendPolicy.seedComments
      ++ [
        "  machine.succeed("
        "    \"JAUNDER_CAPTURE_DIR=/var/lib/jaunder/capture JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT=http://127.0.0.1:4317 devtool seed-e2e\""
        "    + \" --db ${backendPolicy.jaunderDb}\""
        "    + \" --test-support-bin test-support\""
        "    + \" --jaunder-bin jaunder\""
        "  )"
        "  assert_seed_storage_spans()"
      ]
    );
    # These separators preserve the existing generated Python byte-for-byte:
    # SQLite defines its seed helper before VM start, while PostgreSQL does so
    # after package copy. Keeping the bytes stable keeps all eight derivation
    # paths stable, which proves this refactor changes no NixOS-test input.
    beforeMachineStart = if backendPolicy.seedBeforeStart then "\n\n${seedDefinition}\n\n\n" else "\n";
    setupBeforeJaunder =
      if backendPolicy.setupBeforeJaunder == "" then
        "\n"
      else
        "\n${pkgs.lib.removeSuffix "\n" backendPolicy.setupBeforeJaunder}\n\n";
    afterPackageCopy =
      if backendPolicy.seedBeforeStart then "\n\n" else "\n\n\n${seedDefinition}\n\n\n";
  in
  pkgs.testers.nixosTest {
    name = checkName;

    # Cap the test-driver budget (default is 3600 s) so a boot/infra hang
    # fails near 28 min instead of burning the full hour. See issue #130.
    # This is the OUTER budget: `e2ePlaywrightTimeout` above expires first
    # and is sized against the slowest supported concurrent validation path.
    globalTimeout =
      assert e2ePlaywrightTimeout < e2eGlobalTimeout;
      e2eGlobalTimeout;

    nodes.machine =
      { pkgs, lib, ... }:
      {
        imports = [
          self.nixosModules.jaunder
          (backendPolicy.nodeConfig lib)
        ];

        virtualisation.memorySize = vmMemory;
        # Default (null) leaves the nixosTest core count alone. The gate
        # sets 2 to match its worker count: one vCPU would under-stress
        # SQLite write contention and starve the PostgreSQL client render.
        virtualisation.cores = lib.mkIf (vmCores != null) vmCores;
        environment.systemPackages = [
          backendPolicy.package
          pkgs.opentelemetry-collector-contrib
          testSupportBin
          devtoolBin
          # `jaunder site-config set` seed steps resolve bare `jaunder` here.
          jaunderBin
        ];
        environment.etc."jaunder-otel-collector.yaml".source = ../end2end/otel-collector.yaml;

        systemd.tmpfiles.rules = [ "d /var/lib/jaunder/capture 0755 jaunder jaunder -" ];
        systemd.services.otel-collector = {
          description = "Jaunder e2e OTel Collector";
          wantedBy = [ "multi-user.target" ];
          after = [ "network.target" ];
          # The collector configuration reads these runtime endpoints and capture
          # directory through its environment providers.
          environment = e2eOtelCollectorEnv;
          serviceConfig = {
            ExecStart = "${pkgs.opentelemetry-collector-contrib}/bin/otelcol-contrib --config /etc/jaunder-otel-collector.yaml";
            Restart = "on-failure";
            RestartSec = "2s";
          };
        };

        services.jaunder.enable = true;
        services.jaunder.bind = "127.0.0.1:3000";
        # The test script starts Jaunder only after the collector receivers
        # and any backend-specific database setup are ready.
        systemd.services.jaunder.wantedBy = lib.mkForce [ ];
        systemd.services.jaunder.after = [ "otel-collector.service" ];
        systemd.services.jaunder.requires = [ "otel-collector.service" ];
        systemd.services.jaunder.environment = captureEnv // {
          RUST_LOG = "info";
          JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT = "http://127.0.0.1:4317";
        };
      };

    testScript = ''
      ${e2ePhaseTimingHelpers backend browser}${e2eOtelTestHelpers}${beforeMachineStart}vm_startup_started_at = time.monotonic()
      machine.start()
      machine.wait_for_unit("otel-collector.service", timeout=60)
      # `active` precedes the OTLP receiver binds; seeding immediately can
      # export into that gap and leave no trace population to verify.
      wait_for_otel_receivers()${setupBeforeJaunder}machine.succeed("systemctl start jaunder.service")
      machine.wait_for_unit("jaunder.service", timeout=60)
      machine.wait_for_open_port(3000, timeout=30)
      record_e2e_phase(
        "vm-startup-readiness",
        vm_startup_started_at,
        "success",
        "${backend}/${browser}: VM booted and the Jaunder HTTP readiness port opened.",
      )

      machine.succeed("cp -r ${e2ePackage} /tmp/e2e && chmod -R u+w /tmp/e2e")${afterPackageCopy}# Seed a fresh DB and run the one browser this derivation targets.
      # Browsers run as separate derivations (one VM each) so their state
      # mutations cannot interfere; that also lets CI fan them out.
      seed_db()
      ${e2eRunAndCapture {
        inherit
          backend
          browser
          traceId
          traceParent
          extraEnv
          ;
        jaunderDb = backendPolicy.jaunderDb;
      }}
    '';
  };

# Cache-busting salt for e2e measurement runs (#792). Nix caches the e2e
# check derivations, so a repeated `cargo xtask traces run` returns a
# CACHED result rather than re-running the suite — silently handing back
# traces from whenever it was last built, possibly on a CI runner under
# unknown load. Set this to a distinct value per measurement run to force
# a fresh build; REVERT TO "" BEFORE COMMITTING. Empty is a byte-exact
# no-op: it must not change any e2e derivation hash.
e2eSalt = "";

# Enforced by xtask's `e2e-scaffold` static check: a committed non-empty
# salt costs every CI e2e job its cache, and the only symptom is "CI got
# slow" — nothing fails on its own, which is exactly why the guard exists.

# All e2e {backend}×{browser} combos. backend picks the VM builder;
# browser picks the Playwright --project; traceDigit gives each combo a
# distinct OTel trace id (the 1/2/3/4 mapping preserves the historical
# per-combo ids). Add a row here and the gate checks, the single-worker
# diagnostic packages, and the `e2e-checks` aggregate all extend
# automatically.
e2eCombos = [
  {
    backend = "sqlite";
    browser = "chromium";
    traceDigit = "1";
  }
  {
    backend = "sqlite";
    browser = "firefox";
    traceDigit = "2";
  }
  {
    backend = "postgres";
    browser = "chromium";
    traceDigit = "3";
  }
  {
    backend = "postgres";
    browser = "firefox";
    traceDigit = "4";
  }
];

mkE2eCombo =
  {
    backend,
    browser,
    traceDigit,
    nameSuffix ? "",
    extraEnv ? "",
    vmMemory ? 2048,
    vmCores ? null,
  }:
  let
    traceId = pkgs.lib.concatStrings (pkgs.lib.genList (_: traceDigit) 32);
    traceParent = "00-${traceId}-${pkgs.lib.concatStrings (pkgs.lib.genList (_: traceDigit) 16)}-01";
  in
  mkE2eCheck {
    checkName = "jaunder-e2e-${backend}-${browser}${nameSuffix}";
    # The salt rides the combo's generic extra-env string, which is
    # interpolated into the VM testScript above — so it reaches the
    # derivation hash. The variable itself is inert: nothing reads
    # JAUNDER_E2E_SALT. Changing the hash is its whole job. Spliced here
    # rather than per-family so every combo salts alike.
    extraEnv = extraEnv + pkgs.lib.optionalString (e2eSalt != "") " JAUNDER_E2E_SALT=${e2eSalt}";
    inherit backend;
    inherit
      browser
      traceId
      traceParent
      vmMemory
      vmCores
      ;
  };

# attr name -> gate check, e.g. { "e2e-sqlite-chromium" = <drv>; ... }
# The gate runs at workers=2 (#155, see playwright.config.ts), so the
# VMs are sized 3 GB / 2 vCPU: cores >= workers avoids in-guest CPU
# starvation, and with the Firefox process-slimming prefs 3 GB clears the
# OOM that heavier VMs hit (#61). #828's full CI factorial found no
# admissible 3-worker arm: 3 vCPU / 3 GB OOMed; the faster 4 vCPU / 4 GB
# arm increased SQLite flakiness. See docs/observability.md #828.
e2eGateChecks = pkgs.lib.listToAttrs (
  map (c: {
    name = "e2e-${c.backend}-${c.browser}";
    value = mkE2eCombo (
      c
      // {
        # RETRIES=1: the gate reports a fail-then-pass as `flaky` (exit 0)
        # rather than failing the combo check, containing timeout flakiness
        # (Firefox 5s `expect` races) while results.json records it.
        extraEnv = " JAUNDER_E2E_RETRIES=1";
        vmMemory = 3072;
        vmCores = 2;
      }
    );
  }) e2eCombos
);

# Single-worker variants: same combos as the gate checks but pinned to
# workers=1, so per-navigation timings are free of worker contention.
# That isolation is their whole purpose — use them when the question is
# "what does one navigation cost", not "what does the suite cost". The
# worker count is the ONLY difference from the gate combos (#792), and the
# name says so.
#
# NOT part of the gate — built on demand by
# `cargo xtask traces run --single-worker` (see docs/observability.md).
# They keep the default 2 GB VM, since one worker fits where two Firefox
# workers would OOM it (#61). Note workers=1 also drops chromium's
# whole-test scale to 1.0 from the gate's 1.5 (firefox takes
# max(2.2, contention), so it is unaffected) — see DEFAULT_TEST_BUDGET_MS
# in end2end/tests/fixtures.ts (#270).
e2eSingleWorkerPackages = pkgs.lib.listToAttrs (
  map (c: {
    name = "e2e-${c.backend}-${c.browser}-single-worker";
    value = mkE2eCombo (
      c
      // {
        nameSuffix = "-single-worker";
        extraEnv = " JAUNDER_E2E_WORKERS=1";
      }
    );
  }) e2eCombos
);


# Each producer owns one browser and one SQLite VM.  They deliberately are
# separate derivations: evaluating Chromium must neither short-circuit Firefox
# nor share an artifact directory with it.
mkWasmCoverageProducer =
  {
    browser,
    failure ? "",
  }:
  pkgs.testers.nixosTest {
    name = "jaunder-wasm-coverage-${browser}${pkgs.lib.optionalString (failure != "") "-${failure}-failure"}";
    nodes.machine = { lib, ... }: {
      imports = [ self.nixosModules.jaunder ];
      environment.systemPackages = [
        pkgs.sqlite
        testSupportBin
        diagnosticJaunderBin
        devtoolBin
        pkgs.jq
      ];
      services.jaunder.enable = true;
      services.jaunder.db = "sqlite:/var/lib/jaunder/data/jaunder.db";
      services.jaunder.bind = "127.0.0.1:3000";
      systemd.services.jaunder.wantedBy = lib.mkForce [ ];
      systemd.services.jaunder.preStart = lib.mkForce ''
        ${diagnosticJaunderBin}/bin/jaunder init --db "$JAUNDER_DB" --skip-if-exists
      '';
      systemd.services.jaunder.serviceConfig.ExecStart = lib.mkForce "${diagnosticJaunderBin}/bin/jaunder serve";
    };
    testScript = ''
      machine.start()
      machine.succeed("systemctl start jaunder.service")
      machine.succeed(
        "JAUNDER_WASM_COVERAGE_CSR=${self.packages.${system}.wasm-coverage-csr}"
        + " JAUNDER_WASM_COVERAGE_BROWSER=${browser}"
        + " devtool wasm-coverage initialize"
      )
      machine.wait_for_unit("jaunder.service", timeout=60)
      machine.wait_for_open_port(3000, timeout=30)
      machine.succeed("cp -r ${e2ePackage} /tmp/e2e && chmod -R u+w /tmp/e2e")
      if "${failure}" == "early":
        status, output = machine.execute(
          "bash -o pipefail -c '{ printf \"%s\\n\" \"injected early Playwright failure\"; exit 73; } 2>&1 | tee /var/lib/jaunder/wasm-coverage/diagnostics/playwright.log'",
          timeout=300,
        )
      else:
        status, output = machine.execute(
          "bash -o pipefail -c 'cd /tmp/e2e"
          + " && PLAYWRIGHT_BROWSERS_PATH=${pkgs.playwright-driver.browsers}"
          + " PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD=1"
          + " JAUNDER_DB=sqlite:/var/lib/jaunder/data/jaunder.db"
          + " FONTCONFIG_FILE=${visualFontConfig}"
          + " JAUNDER_WASM_COVERAGE_INJECT_FAILURE=${failure}"
          + " JAUNDER_WASM_COVERAGE_CSR=${self.packages.${system}.wasm-coverage-csr}"
          + " JAUNDER_WASM_COVERAGE_OUT=/var/lib/jaunder/wasm-coverage"
          + " ${pkgs.nodejs}/bin/node node_modules/.bin/playwright test"
          + " tests/wasm-coverage.spec.ts --config playwright.config.ts --project ${browser}"
          + " 2>&1 | tee /var/lib/jaunder/wasm-coverage/diagnostics/playwright.log'",
          timeout=300,
        )
      print(output)
      # The browser helper writes a v1 result before surfacing any capture
      # failure. If it never started, finalize the initialized sentinel with
      # the retained Playwright output and exact exit status.
      map_status, map_output = machine.execute(
        "JAUNDER_WASM_COVERAGE_CSR=${self.packages.${system}.wasm-coverage-csr}"
        + " JAUNDER_WASM_COVERAGE_INJECT_FAILURE=${failure}"
        + " devtool wasm-coverage map --site-src ${siteSrc}",
        timeout=120,
      )
      print(map_output)
      machine.succeed(
        "JAUNDER_WASM_COVERAGE_PLAYWRIGHT_EXIT="
        + str(status)
        + " devtool wasm-coverage finalize"
      )
      if "${failure}" == "early":
        machine.succeed(
          "jq -e '"
          + ".actual_browser == \"not-started\""
          + " and .csr_structural.outcome == \"passed\""
          + " and (.served_module.path | startswith(\"pkg/\"))"
          + " and .artifacts.module.path == (\"module/\" + .served_module.path)"
          + " and .diagnostic_export.outcome == \"failed\""
          + " and .diagnostic_export.blocker == \"Playwright exited with status 73 before coverage capture\""
          + " and .source_mapping.outcome == \"not-run\""
          + " and (.artifacts | keys | sort) == [\"diagnostics\", \"module\"]'"
          + " /var/lib/jaunder/wasm-coverage/status.json"
        )
      machine.succeed("tar czf /tmp/wasm-coverage-${browser}.tar.gz -C /var/lib/jaunder wasm-coverage")
      machine.copy_from_machine("/tmp/wasm-coverage-${browser}.tar.gz", "")
    '';
  };
# These manual timing producers are intentionally separate from the permanent
# coverage evidence producers above. `cacheBuster` is interpolated into the
# derivation name and retained result, so `--impure` invocation entropy changes
# the Nix realization rather than merely a runtime environment variable.
mkWasmCoverageMeasurementProducer =
  {
    browser,
    mode,
    cacheBuster,
  }:
  assert cacheBuster != "";
  pkgs.testers.nixosTest {
    name = "jaunder-wasm-coverage-measure-${browser}-${mode}-${cacheBuster}";
    nodes.machine = { lib, ... }: {
      imports = [ self.nixosModules.jaunder ];
      environment.systemPackages = [ pkgs.sqlite testSupportBin pkgs.python3 ];
      services.jaunder.enable = true;
      services.jaunder.db = "sqlite:/var/lib/jaunder/data/jaunder.db";
      services.jaunder.bind = "127.0.0.1:3000";
      systemd.services.jaunder.wantedBy = lib.mkForce [ ];
      systemd.services.jaunder.preStart = lib.mkForce ''
        ${if mode == "baseline" then diagnosticBaselineJaunderBin else diagnosticJaunderBin}/bin/jaunder init --db "$JAUNDER_DB" --skip-if-exists
      '';
      systemd.services.jaunder.serviceConfig.ExecStart = lib.mkForce "${if mode == "baseline" then diagnosticBaselineJaunderBin else diagnosticJaunderBin}/bin/jaunder serve";
    };
    testScript = ''
      machine.start()
      machine.succeed("systemctl start jaunder.service")
      machine.wait_for_unit("jaunder.service", timeout=60)
      machine.wait_for_open_port(3000, timeout=30)
      machine.succeed("cp -r ${e2ePackage} /tmp/e2e && chmod -R u+w /tmp/e2e && mkdir -p /var/lib/jaunder/wasm-coverage")
      status, output = machine.execute(
        "cd /tmp/e2e"
        + " && PLAYWRIGHT_BROWSERS_PATH=${pkgs.playwright-driver.browsers}"
        + " PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD=1"
        + " JAUNDER_DB=sqlite:/var/lib/jaunder/data/jaunder.db"
        + " FONTCONFIG_FILE=${visualFontConfig}"
        + " JAUNDER_WASM_COVERAGE_OUT=/var/lib/jaunder/wasm-coverage"
        + " JAUNDER_WASM_COVERAGE_CSR=${if mode == "baseline" then diagnosticBaselineCsrWasmBundle else diagnosticCsrWasmBundle}"
        + " JAUNDER_WASM_COVERAGE_CACHE_BUSTER=${cacheBuster}"
        + " JAUNDER_WASM_COVERAGE_MODE=${mode}"
        + " ${pkgs.nodejs}/bin/node node_modules/.bin/playwright test"
        + " tests/wasm-coverage-measure.spec.ts --config playwright.config.ts --project ${browser}",
        timeout=300,
      )
      assert status == 0, output
      machine.succeed("test -s /var/lib/jaunder/wasm-coverage/measurement.json")
      machine.succeed("tar czf /tmp/wasm-coverage-measure-${browser}-${mode}.tar.gz -C /var/lib/jaunder/wasm-coverage measurement.json")
      machine.copy_from_machine("/tmp/wasm-coverage-measure-${browser}-${mode}.tar.gz", "")
    '';
  };
  measurementCacheBuster = builtins.getEnv "JAUNDER_WASM_COVERAGE_CACHE_BUSTER";
in
{

  packages = pkgs.lib.optionalAttrs pkgs.stdenv.isLinux (
    {
# The e2e aggregate: a symlinkJoin of every browser/backend `e2e-*`
# check, exposed as `checks.e2e` and built by `cargo xtask validate`.
# Adding a new browser/backend combo automatically joins it here. Its exact
# `jaunder-e2e-checks` basename is a final verdict excluded by Cachix's
# hash-prefixed, basename-anchored `pushFilter`, so the aggregate cannot
# substitute a cached green result.
e2e-checks = pkgs.symlinkJoin {
  name = "jaunder-e2e-checks";
  paths = builtins.attrValues (
    pkgs.lib.filterAttrs (name: _: pkgs.lib.hasPrefix "e2e-" name) self.checks.${system}
  );
};
wasm-coverage-chromium = mkWasmCoverageProducer { browser = "chromium"; };
wasm-coverage-firefox = mkWasmCoverageProducer { browser = "firefox"; };
wasm-coverage-chromium-export-failure = mkWasmCoverageProducer {
  browser = "chromium";
  failure = "export";
};
wasm-coverage-firefox-mapping-failure = mkWasmCoverageProducer {
  browser = "firefox";
  failure = "mapping";
};
wasm-coverage-chromium-early-playwright-failure = mkWasmCoverageProducer {
  browser = "chromium";
  failure = "early";
};
}
// pkgs.lib.optionalAttrs (measurementCacheBuster != "") {
wasm-coverage-measure-chromium-baseline = mkWasmCoverageMeasurementProducer {
  browser = "chromium";
  mode = "baseline";
  cacheBuster = measurementCacheBuster;
};
wasm-coverage-measure-chromium-instrumented = mkWasmCoverageMeasurementProducer {
  browser = "chromium";
  mode = "instrumented";
  cacheBuster = measurementCacheBuster;
};
wasm-coverage-measure-firefox-baseline = mkWasmCoverageMeasurementProducer {
  browser = "firefox";
  mode = "baseline";
  cacheBuster = measurementCacheBuster;
};
wasm-coverage-measure-firefox-instrumented = mkWasmCoverageMeasurementProducer {
  browser = "firefox";
  mode = "instrumented";
  cacheBuster = measurementCacheBuster;
};
    }
    // e2eSingleWorkerPackages
  );

  checks = pkgs.lib.optionalAttrs pkgs.stdenv.isLinux (
e2eGateChecks
// {
  wasm-tests = craneLib.cargoTest (
    commonArgs
    // {
      src = wasmTestSrc;
    }
    // {
      cargoArtifacts = craneLib.buildDepsOnly (
        commonArgs
        // leanTestProfile
        // {
          src = wasmTestSrc;
        }
        // {
          CARGO_BUILD_TARGET = "wasm32-unknown-unknown";
          cargoExtraArgs = "-p client";
          doCheck = false;
        }
      );
      pname = "jaunder-wasm-tests";
      CARGO_BUILD_TARGET = "wasm32-unknown-unknown";
      # wasm-bindgen-test diagnostics do not depend on native DWARF.
      CARGO_PROFILE_TEST_DEBUG = "0";
      cargoTestExtraArgs = "-p client";
      nativeBuildInputs = commonArgs.nativeBuildInputs ++ [ wasm-bindgen-cli ];
      CHROMEDRIVER = "${pkgs.chromedriver}/bin/chromedriver";
      CHROMEDRIVER_ARGS = "--verbose";
      WASM_BINDGEN_TEST_WEBDRIVER_JSON = "${wasmTestWebdriverConfig}";
      preCheck = ''
        export XDG_CONFIG_HOME="$TMPDIR/chromium-config"
        mkdir -p "$XDG_CONFIG_HOME"
      '';
    }
  );

  # The browser/backend e2e gate `cargo xtask validate` builds.
  # `e2e-checks` aggregates every browser/backend `checks.e2e-*` combo
  # (now 4); they are independent derivations realized in parallel up
  # to the host `max-jobs` (CI's install-nix-action sets `max-jobs =
  # auto`; a plain dev box defaults to 1 and runs them serially). The exact
  # final aggregate basename is excluded by Cachix's hash-prefixed,
  # basename-anchored `pushFilter`, so it cannot substitute a cached green
  # aggregate.
  e2e = self.packages.${system}.e2e-checks;

  # The producer combines pure and server-backed ERT observations in
  # one VM, returning controlled outcomes as fixed artifacts for the
  # host-side authoritative consumer.
  elisp-coverage-producer = pkgs.testers.nixosTest {
    name = "jaunder-elisp-coverage-producer";
    nodes.machine = _: {
      virtualisation.memorySize = 4096;
      virtualisation.cores = 2;
      environment.systemPackages = [
        emacsForCi
        jaunderBin
        pkgs.curl
      ];
    };
    testScript = ''
      machine.start()
      machine.wait_for_unit("multi-user.target")
      machine.succeed("mkdir -p /tmp/elisp-coverage")
      machine.succeed(
          "JAUNDER_TEST_BINARY=${jaunderBin}/bin/jaunder "
          + "JAUNDER_ELISP_COVERAGE_DIR=/tmp/elisp-coverage "
          + "emacs --batch -Q -l ${emacsSrc}/scripts/run-coverage.el"
      )
      machine.succeed(
          "test -s /tmp/elisp-coverage/lcov.info"
          + " && test -s /tmp/elisp-coverage/summary.txt"
          + " && test -s /tmp/elisp-coverage/status.json"
      )
      machine.copy_from_machine("/tmp/elisp-coverage/lcov.info", "elisp-coverage")
      machine.copy_from_machine("/tmp/elisp-coverage/summary.txt", "elisp-coverage")
      machine.copy_from_machine("/tmp/elisp-coverage/status.json", "elisp-coverage")
    '';
  };

# `devtool` owns static-check definitions; separate source boundaries
# let Markdown-only changes avoid realizing the code-static group.
static-docs =
  let
    staticDocsSrc = pkgs.lib.cleanSourceWith {
      src = craneLib.path ../.;
      filter =
        path: type:
        let
          relative = pkgs.lib.removePrefix "${toString ../.}/" (toString path);
          ignored =
            relative == "docs/archive"
            || pkgs.lib.hasPrefix "docs/archive/" relative
            || relative == ".claude"
            || pkgs.lib.hasPrefix ".claude/" relative;
        in
        !ignored
        && (
          type == "directory"
          || pkgs.lib.hasSuffix ".md" path
          || pkgs.lib.hasSuffix "/.prettierrc.json" path
          || pkgs.lib.hasSuffix "/.prettierignore" path
        );
    };
  in
  pkgs.runCommand "static-docs"
    {
      nativeBuildInputs = [
        devtoolBin
        pkgs.prettier
      ];
    }
    ''
      cp --no-preserve=mode -r ${staticDocsSrc} src
      cd src
      devtool check --group docs
      touch $out
    '';
static-code =
  let
    staticCodeSrc = pkgs.lib.cleanSourceWith {
      src = craneLib.path ../.;
      filter =
        path: type:
        let
          isXtask = pkgs.lib.hasSuffix "/xtask" path || pkgs.lib.hasInfix "/xtask/" path;
        in
        !isXtask
        && (
          type == "directory"
          || (
            !(pkgs.lib.hasSuffix ".md" path)
            && (
              builtins.any (suffix: pkgs.lib.hasSuffix suffix path) [
                "/Cargo.toml"
                "/Cargo.lock"
                "/rust-toolchain.toml"
                "/deny.toml"
                "/clippy.toml"
                "/.rustfmt.toml"
                "/.prettierrc.json"
                "/.prettierignore"
                "/sgconfig.yml"
              ]
              || builtins.any (directory: pkgs.lib.hasInfix "/${directory}/" path) [
                ".cargo"
                "ast-grep"
                "common"
                "macros"
                "server"
                "storage"
                "web"
                "client"
                "csr"
                "host"
                "test-support"
                "public"
                "tools"
                "end2end"
                "elisp"
              ]
            )
          )
        );
    };
  in
  pkgs.runCommand "static-code"
    {
      nativeBuildInputs = [
        pkgs.stdenv.cc
      ]
      ++ hostArgs.nativeBuildInputs
      ++ [
        devtoolBin
        toolchain
        pkgs.cargo-deny
        pkgs.ast-grep
        leptosfmt
        pkgs.prettier
        pkgs.nodejs
        pkgs.typescript
        emacsForCi
      ];
      buildInputs = hostArgs.buildInputs;
      # ert needs a zone DB (#160); tsc needs BOTH node-dep envs
      # (`devtool provision-node-modules`'s resolver errors on each when
      # unset).
      TZDIR = "${pkgs.tzdata}/share/zoneinfo";
      E2E_TYPES_NODE_MODULES = "${e2ePackage}/node_modules";
      E2E_PLAYWRIGHT_TEST = "${pkgs.playwright-test}/lib/node_modules/@playwright/test";
      JAUNDER_DEVTOOL_PRODUCT_CARGO_HOME = "${appOfflineCargoHome}";
      JAUNDER_DEVTOOL_TOOLS_CARGO_HOME = "${toolsOfflineCargoHome}";
    }
    ''
      # Writable copy: `devtool check tsc` provisions end2end/node_modules
      # in-process (#229).
      cp --no-preserve=mode -r ${staticCodeSrc} src
      cd src
      devtool check --group code --sandbox-cargo
      touch $out
    '';
coverage = craneLib.mkCargoDerivation (
  hostArgs
  // {
    src = coverageSrc;
    inherit cargoArtifacts;
    pname = "jaunder-coverage";
    # Source-based coverage uses LLVM's embedded coverage map
    # (-Cinstrument-coverage), not DWARF, so dropping debuginfo
    # shrinks the instrumented test binaries dramatically with no
    # loss of line coverage. Without this the instrumented link
    # exhausts the build filesystem and rust-lld dies with SIGBUS
    # writing its mmap'd output on the CI runner.
    CARGO_PROFILE_DEV_DEBUG = "0";
    CARGO_PROFILE_TEST_DEBUG = "0";
    # Stage the real CSR bundle + public assets so `server`'s
    # `build.rs` embeds a POPULATED `site::Site` under instrumentation
    # (#237). Without this the coverage sandbox has no bundle, and the
    # `serve_site` handler's asset-serving branch could only be
    # `cov:ignore`d; with it, the handler is exercised end-to-end by
    # its integration tests and genuinely measured. Same env the
    # release `jaunderBin` uses; `build.rs` copies from these paths.
    JAUNDER_CSR_BUNDLE_DIR = "${csrWasmBundle}";
    JAUNDER_PUBLIC_DIR = "${../public}";
    nativeBuildInputs = hostArgs.nativeBuildInputs ++ [
      devtoolBin
      cargo-crap
      pkgs.cargo-llvm-cov
      pkgs.cargo-nextest
      # devtool runs the whole test suite under an ephemeral
      # PostgreSQL (via devtool pg) so
      # storage/src/postgres/* gets instrumented coverage. The
      # throwaway cluster needs initdb/pg_ctl/psql available inside
      # the build sandbox.
      pkgs.postgresql_18
    ];
    buildPhaseCargoCommand = ''
      export LD_LIBRARY_PATH="${pkgs.lib.makeLibraryPath [ pkgs.openssl pkgs.dav1d ]}:''${LD_LIBRARY_PATH:-}"
      mkdir -p emit-out
      # The producer emits checked stage evidence; the Nix gate and host xtask
      # consume its status and reports as separate authoritative boundaries.
      devtool coverage emit --out emit-out
    '';
    installPhaseCommand = ''
      mkdir -p $out
      # Preserve controlled-red producer status and diagnostics even when report
      # stages did not run. Reports are copied only when their producer stages
      # produced them; their host consumer rejects missing or empty evidence.
      if test -e emit-out/status.json; then
        cp emit-out/status.json $out/status.json
      fi
      if test -d emit-out/diagnostics; then
        cp -r emit-out/diagnostics $out/diagnostics
      fi
      # emit-out/coverage-report.lcov is intentionally NOT copied: it is an
      # intermediate consumed only by `cargo crap`, not a gate output the host reads.
      if test -e emit-out/coverage-report.txt; then
        cp emit-out/coverage-report.txt $out/coverage-report.txt
      fi
      if test -e emit-out/crap-report.json; then
        cp emit-out/crap-report.json $out/crap-report.json
      fi
    '';
  }
);
  # Probe-only identity: its sole varying input is the filtered coverage source.
  # Keep this separate from coverage.drvPath, which also includes producer inputs.
  coverage-source-probe = pkgs.runCommand "jaunder-coverage-source-probe" { src = coverageSrc; } ''
    touch $out
  '';
# Belt-and-suspenders: the sandbox gate validates completed producer evidence
# through the shared Rust contract, while the host separately consumes reports.
# Its exact final `jaunder-coverage-gate` basename is excluded by Cachix's
# hash-prefixed, basename-anchored `pushFilter`; support outputs remain eligible.
coverage-gate =
  pkgs.runCommand "jaunder-coverage-gate"
    {
      nativeBuildInputs = [ devtoolBin ];
    }
    ''
      devtool coverage validate-status \
        --status ${self.checks.${system}.coverage}/status.json
      touch $out
    '';

# Doctests: the one suite nextest structurally cannot run, so the
# `coverage` check above never sees them (#763). The producer runs
# `cargo test --workspace --doc` AND reconciles what ran against the
# fences the scanner finds in the source, in both directions — running
# alone would inherit every way a doctest population silently shrinks
# (a cfg gate, an unrecognized info string, a crate out of reach).
#
# `--workspace` is load-bearing, not incidental: package-scoping to
# `-p common -p macros` drops the three `#[cfg(feature = "sanitize")]`
# fences in `common/src/render.rs`, because nothing in that package set
# enables the feature. Under `--workspace`, unification enables it via
# `storage`. The invocation is pinned by a unit test in devtool.
#
# `--doc` runs OUTSIDE any llvm-cov instrumentation, so no profraw from
# these tests reaches the coverage profile: doctests deliberately do not
# feed the ADR-0050 coverage gate (`llvm-cov --doctests` is unstable).
doctests = craneLib.mkCargoDerivation (
  hostArgs
  // {
    cargoArtifacts = craneLib.buildDepsOnly (hostArgs // leanDevAndTestProfile);
    pname = "jaunder-doctests";
    # Doctest output comes from rustdoc/libtest diagnostics and the
    # fence reconciler, not DWARF. Keep the override local so manual
    # `cargo test --doc` remains fully debuggable.
    CARGO_PROFILE_DEV_DEBUG = "0";
    CARGO_PROFILE_TEST_DEBUG = "0";
    nativeBuildInputs = hostArgs.nativeBuildInputs ++ [ devtoolBin ];
    buildPhaseCargoCommand = ''
      export LD_LIBRARY_PATH="${pkgs.lib.makeLibraryPath [ pkgs.openssl pkgs.dav1d ]}:''${LD_LIBRARY_PATH:-}"
      mkdir -p emit-out
      # devtool always exits 0 after writing emit-out/status.json;
      # gating is the doctests-gate consumer + host xtask.
      devtool doctests emit --out emit-out
    '';
    installPhaseCommand = ''
      mkdir -p $out
      cp emit-out/status.json $out/status.json
      cp -r emit-out/diagnostics $out/diagnostics
    '';
  }
);
# The consumer that actually fails, mirroring `coverage-gate`. Named
# `jaunder-doctests-gate` for symmetry with its producer.
doctests-gate =
  pkgs.runCommand "jaunder-doctests-gate"
    {
      nativeBuildInputs = [ pkgs.jq ];
    }
    ''
      cat ${self.checks.${system}.doctests}/status.json
      cat=$(jq -r .category ${self.checks.${system}.doctests}/status.json)
      if [ "$cat" != "ok" ]; then
        echo "doctest gate failed: category=$cat" >&2
        jq -r '.infra_detail // (.violations[] | if .line == null then "\(.file) [\(.kind)] \(.detail)" else "\(.file):\(.line) [\(.kind)] \(.detail)" end)' \
          ${self.checks.${system}.doctests}/status.json >&2
        exit 1
      fi
      touch $out
    '';
    }
  );
}
