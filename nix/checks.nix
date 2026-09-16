{ self, system, pkgs, nixpkgs, nixosInternals, packageInternals }:
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
    siteCargoMembers
    wasmTestCargoMembers
    appOfflineCargoHome
    toolsOfflineCargoHome
    workspaceMembers
    cargoTargetSource
    cargoMemberSource
    cargoPackageClosure
    cargoArtifacts
    leanTestProfile
    leanDevAndTestProfile
    jaunderBin
    testSupportBin
    devtoolBin
    docsDevtoolBin
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
  # Final coverage/e2e verdicts must execute for the current ref. These
  # derivation flags are checked from actual Nix metadata by cache-safety probe;
  # the closure proof remains the independent defense against result leakage.
  nonSubstitutable =
    derivation:
    if derivation ? overrideAttrs then
      derivation.overrideAttrs (_: {
        allowSubstitutes = false;
        preferLocalBuild = true;
      })
    else
      derivation.overrideTestDerivation (_: {
        allowSubstitutes = false;
        preferLocalBuild = true;
      });
  # The root workspace remains the coverage population. Its Cargo manifests
  # define the recursively discovered local path package build closure.
  coverageMembers = cargoPackageClosure workspaceMembers;
  cargoSourcePath = relative: "${toString ../.}/${relative}";
  siteTargetSource =
    relative: cargoTargetSource siteCargoMembers (cargoSourcePath relative) "regular";
  wasmTestTargetSource =
    relative: cargoTargetSource wasmTestCargoMembers (cargoSourcePath relative) "regular";
  sourceMembershipAssertions =
    assert builtins.elem "tools/performance" siteCargoMembers;
    assert builtins.elem "tools/performance" wasmTestCargoMembers;
    assert (siteTargetSource "tools/performance/Cargo.toml");
    assert (siteTargetSource "tools/performance/src/lib.rs");
    assert (wasmTestTargetSource "tools/performance/Cargo.toml");
    assert (wasmTestTargetSource "tools/performance/src/lib.rs");
    assert !(siteTargetSource "tools/devtool/Cargo.toml");
    assert !(siteTargetSource "tools/doctests/Cargo.toml");
    assert !(siteTargetSource "tools/diagnostic-coverage-runtime/Cargo.toml");
    assert !(siteTargetSource "xtask/Cargo.toml");
    assert !(wasmTestTargetSource "tools/devtool/Cargo.toml");
    assert !(wasmTestTargetSource "tools/doctests/Cargo.toml");
    assert !(wasmTestTargetSource "tools/diagnostic-coverage-runtime/Cargo.toml");
    assert !(wasmTestTargetSource "xtask/Cargo.toml");
    true;

  # Coverage source remains bounded to Cargo-recognized package inputs plus the
  # explicit nextest profile, SQLx migration trees, compile-time rust-embed
  # assets and CSR shell, and the immutable backup compatibility corpus consumed
  # at runtime through CARGO_MANIFEST_DIR.
  coverageAuxiliarySource =
    relative:
    relative == ".config/nextest.toml"
    || relative == "csr/index.html"
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
    assert (coverageAuxiliarySource "csr/index.html");
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
e2eSeedTraceTimeout = 30;

# Performance producers seed larger profiles before measuring. Their browser
# step gets 75 minutes; the VM budget leaves 25 minutes for boot, seed, and
# artifact recovery so the inner timeout remains diagnostic-preserving.
performanceBrowserTimeout = 4500;
performanceGlobalTimeout = 6000;

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
e2eOtelTestHelpers = backend: ''
  def wait_for_otel_receivers():
    machine.wait_for_open_port(4317, timeout=30)
    machine.wait_for_open_port(4318, timeout=30)

  def retain_e2e_capture():
    # The ordinary and pre-Playwright paths retain one whole-directory artifact,
    # so a failure cannot silently lose a capture stream that ordinary retention preserves.
    machine.execute("test -d /var/lib/jaunder/capture && tar czf /tmp/capture-${backend}.tar.gz -C /var/lib/jaunder capture 2>/dev/null || true")
    if machine.execute("test -e /tmp/capture-${backend}.tar.gz")[0] == 0:
      machine.copy_from_machine("/tmp/capture-${backend}.tar.gz", "")

  def assert_seed_storage_spans():
    stop_status, stop_out = machine.execute(
      "systemctl stop otel-collector.service",
      timeout=${toString e2eSeedTraceTimeout},
    )
    if stop_status != 0:
      retain_e2e_capture()
      if stop_status == 124:
        raise AssertionError("seed-collector-stop-timeout: collector stop exceeded ${toString e2eSeedTraceTimeout}s")
      raise AssertionError("seed-collector-stop-failed: " + stop_out)

    verify_status, verify_out = machine.execute(
      "test-support verify-seed-trace --capture-dir /var/lib/jaunder/capture 2>&1",
      timeout=${toString e2eSeedTraceTimeout},
    )
    if verify_status != 0:
      retain_e2e_capture()
      if verify_status == 124:
        raise AssertionError("seed-trace-verifier-timeout: verification exceeded ${toString e2eSeedTraceTimeout}s")
      raise AssertionError(verify_out)

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

  e2e_phases: list[dict[str, object]] = [
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

    # Capture-dir contract (#227, #332): the shared helper retains the complete
    # capture directory for ordinary and seed-failure paths alike. It holds diag.log,
    # the collector's flushed otel-traces.jsonl, plus any written mail/websub stream.
    retain_e2e_capture()
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
    producer ? null,
    extraNodeConfig ? (_: { }),
    vmGlobalTimeout ? e2eGlobalTimeout,
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
  nonSubstitutable (pkgs.testers.nixosTest {
    name = checkName;

    # Caller-selected outer budget for boot, seed, execution, and artifact
    # recovery. The ordinary gate defaults to 28 minutes; performance producers
    # extend it for canonical dataset seeding. Each inner execution timeout must
    # expire first so diagnostics remain recoverable.
    globalTimeout =
      assert e2eSeedTraceTimeout < e2ePlaywrightTimeout;
      assert e2eSeedTraceTimeout < vmGlobalTimeout;
      vmGlobalTimeout;

    nodes.machine =
      { pkgs, lib, ... }:
      {
        imports = [
          self.nixosModules.jaunder
          (backendPolicy.nodeConfig lib)
          (extraNodeConfig { inherit pkgs lib; })
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

    testScript =
      if producer == null then
        ''
          ${e2ePhaseTimingHelpers backend browser}${e2eOtelTestHelpers backend}${beforeMachineStart}vm_startup_started_at = time.monotonic()
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
        ''
      else
        producer { inherit backendPolicy; };
  });

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
# A manual producer selects exactly one storage or browser measurement. It
# reuses the e2e VM's backend lifecycle while keeping the gate's default branch
# byte-for-byte stable.
mkPerformanceProducer =
  {
    freshnessNonce,
    profile,
    backend,
    producerKind,
    browser ? null,
    posts ? null,
    authors ? null,
    revisions ? null,
  }:
  assert freshnessNonce != "";
  assert builtins.elem profile [ "small" "medium" "large" ];
  assert builtins.elem backend [ "sqlite" "postgres" ];
  assert builtins.elem producerKind [ "storage" "browser" ];
  assert (producerKind == "browser") == (browser != null);
  assert browser == null || builtins.elem browser [ "chromium" "firefox" ];
  assert posts == null || posts > 0;
  assert authors == null || authors > 0;
  assert revisions == null || revisions > 0;
  let
    selectedBrowser = if browser == null then "chromium" else browser;
    traceDigest = builtins.hashString "sha256" "performance-${freshnessNonce}-${backend}-${selectedBrowser}";
    performanceTraceId = "1${builtins.substring 1 31 traceDigest}";
    performanceParentId = "1${builtins.substring 33 15 traceDigest}";
    performanceDiskSize = {
      small = 2048;
      medium = 8192;
      large = 32768;
    }.${profile};
    performanceTraceParent = "00-${performanceTraceId}-${performanceParentId}-01";
  in
  mkE2eCheck {
    inherit backend;
    checkName = "jaunder-performance-${producerKind}-${backend}-${profile}-${freshnessNonce}";
    browser = selectedBrowser;
    traceId = performanceTraceId;
    traceParent = performanceTraceParent;
    vmGlobalTimeout =
      assert performanceBrowserTimeout < performanceGlobalTimeout;
      performanceGlobalTimeout;
    extraNodeConfig = { lib, ... }: {
      # Performance fixtures need runtime data capacity beyond the closure-sized test disk.
      virtualisation.diskSize = performanceDiskSize;
      systemd.services.jaunder.environment.JAUNDER_STORAGE_PATH = "/var/lib/jaunder/media";
    };
    producer = { backendPolicy }: ''
      import json
      import shlex
      import time

      ${e2eOtelTestHelpers backend}
      machine.start()
      provisioning_started = time.monotonic_ns()
      machine.wait_for_unit("otel-collector.service", timeout=60)
      wait_for_otel_receivers()
      ${backendPolicy.setupBeforeJaunder}
      machine.succeed("install -d -o jaunder -g jaunder /var/lib/jaunder/data /var/lib/jaunder/media && mkdir -p /var/lib/jaunder/performance/fragments /var/lib/jaunder/performance/diagnostics")
      machine.succeed("systemctl start jaunder.service")
      machine.wait_for_unit("jaunder.service", timeout=60)
      machine.wait_for_open_port(3000, timeout=30)
      provisioning_us = (time.monotonic_ns() - provisioning_started) // 1000

      seeding_started = time.monotonic_ns()
      machine.succeed(
        "JAUNDER_DB=${backendPolicy.jaunderDb}"
        + " test-support perf-seed --profile ${profile}"
        + " --output /var/lib/jaunder/performance"
        + " --storage-path /var/lib/jaunder/media"
        + "${pkgs.lib.optionalString (posts != null) " --posts ${toString posts}"}"
        + "${pkgs.lib.optionalString (authors != null) " --authors ${toString authors}"}"
        + "${pkgs.lib.optionalString (revisions != null) " --revisions ${toString revisions}"}"
      )
      setup = {
        "provisioning_us": provisioning_us,
        "seeding_us": (time.monotonic_ns() - seeding_started) // 1000,
      }
      machine.succeed(
        "printf %s "
        + shlex.quote(json.dumps(setup, separators=(",", ":")))
        + " > /var/lib/jaunder/performance/setup.json"
      )

      architecture = machine.succeed("uname -m").strip()
      cpu_model = machine.succeed("awk -F ': ' '/^model name/ { print $2; exit }' /proc/cpuinfo").strip()
      identity = {
        "nix_system": "${system}",
        "runner_image": "nixos-vm",
        "runner_architecture": architecture,
        "cpu_model": cpu_model,
        "database_version": "${backendPolicy.package.version}",
        "browser_version": "not-applicable",
        "stable_derivation_identities": [
          { "name": "client", "identity": "${csrWasmBundle.drvPath}" },
          { "name": "producer", "identity": "${devtoolBin.drvPath}" },
          { "name": "server", "identity": "${jaunderBin.drvPath}" },
        ],
      }

      ${pkgs.lib.optionalString (producerKind == "storage") ''
        machine.succeed(
          "printf %s "
          + shlex.quote(json.dumps(identity, separators=(",", ":")))
          + " > /var/lib/jaunder/performance/identity.json"
        )
        storage_status, storage_output = machine.execute(
          "devtool performance storage --db ${backendPolicy.jaunderDb} --backend ${backend}"
          + " --manifest /var/lib/jaunder/performance/dataset-manifest-v1.json"
          + " --output /var/lib/jaunder/performance/fragments"
          + " --provisioning-us " + str(setup["provisioning_us"])
          + " --seeding-us " + str(setup["seeding_us"])
          + " --nix-system ${system} --runner-image nixos-vm"
          + " --runner-architecture " + shlex.quote(architecture)
          + " --cpu-model " + shlex.quote(cpu_model)
          + " --database-version ${backendPolicy.package.version}"
          + " --stable-derivation-identity client=${csrWasmBundle.drvPath}"
          + " --stable-derivation-identity producer=${devtoolBin.drvPath}"
          + " --stable-derivation-identity server=${jaunderBin.drvPath} 2>&1"
        )
        machine.succeed(
          "printf %s "
          + shlex.quote(json.dumps({
            "schema_version": 1,
            "ok": storage_status == 0,
            "detail": storage_output,
          }, separators=(",", ":")))
          + " > /var/lib/jaunder/performance/producer-status-v1.json"
        )
        machine.copy_from_machine("/var/lib/jaunder/performance", "")
      ''}

      ${pkgs.lib.optionalString (producerKind == "browser") ''
        machine.succeed("cp -r ${e2ePackage} /tmp/e2e && chmod -R u+w /tmp/e2e")
        browser_path = machine.succeed(
          "cd /tmp/e2e && PLAYWRIGHT_BROWSERS_PATH=${pkgs.playwright-driver.browsers}"
          + " PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD=1"
          + " ${pkgs.nodejs}/bin/node -e 'const p = require(\"playwright\"); console.log(p.${selectedBrowser}.executablePath())'"
        ).strip()
        identity["browser_version"] = machine.succeed(browser_path + " --version").strip()
        machine.succeed(
          "printf %s "
          + shlex.quote(json.dumps(identity, separators=(",", ":")))
          + " > /var/lib/jaunder/performance/identity.json"
        )
        browser_status, browser_output = machine.execute(
          "cd /tmp/e2e"
          + " && PLAYWRIGHT_BROWSERS_PATH=${pkgs.playwright-driver.browsers}"
          + " PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD=1"
          + " JAUNDER_DB=${backendPolicy.jaunderDb}"
          + " JAUNDER_PERF_MANIFEST_PATH=/var/lib/jaunder/performance/dataset-manifest-v1.json"
          + " JAUNDER_PERF_FRAGMENT_DIR=/var/lib/jaunder/performance/fragments"
          + " JAUNDER_PERF_BACKEND=${backend}"
          + " JAUNDER_PERF_BROWSER=${selectedBrowser}"
          + " JAUNDER_PERF_SETUP_JSON=$(cat /var/lib/jaunder/performance/setup.json)"
          + " JAUNDER_E2E_TRACE_ID=${performanceTraceId}"
          + " JAUNDER_E2E_TRACEPARENT=${performanceTraceParent}"
          + " JAUNDER_E2E_OTLP_HTTP_ENDPOINT=http://127.0.0.1:4318/v1/traces"
          + " JAUNDER_PERF_IDENTITY_JSON=$(cat /var/lib/jaunder/performance/identity.json)"
          + " JAUNDER_PERF_BUILD_MODE=release"
          + " ${pkgs.nodejs}/bin/node node_modules/.bin/playwright test"
          + " tests/browser-performance.measure.spec.ts --config playwright.config.ts --project ${selectedBrowser} --no-deps 2>&1",
          timeout=${toString performanceBrowserTimeout},
        )
        machine.execute("systemctl stop otel-collector.service")
        otel_status, otel_output = machine.execute(
          "test -s /var/lib/jaunder/capture/otel-traces.jsonl"
          + " && install -D /var/lib/jaunder/capture/otel-traces.jsonl"
          + " /var/lib/jaunder/performance/fragments/diagnostics/otel-traces.jsonl 2>&1"
        )
        machine.succeed(
          "mkdir -p /var/lib/jaunder/performance/diagnostics"
          + " && cp -r /tmp/e2e/test-results"
          + " /var/lib/jaunder/performance/diagnostics/playwright-test-results 2>/dev/null || true"
        )
        if browser_status != 0:
          producer_ok = False
          producer_detail = browser_output
        elif otel_status != 0:
          producer_ok = False
          producer_detail = (
            "correlated OTLP trace unavailable (exit %d): %s"
            % (otel_status, otel_output)
          )
        else:
          browser_validation_status, browser_validation_output = machine.execute(
            "devtool performance validate-browser"
            + " --manifest /var/lib/jaunder/performance/dataset-manifest-v1.json"
            + " --input /var/lib/jaunder/performance/fragments/browser-${backend}-${selectedBrowser}-v1.json"
            + " --output /var/lib/jaunder/performance/fragments 2>&1"
          )
          producer_ok = browser_validation_status == 0
          producer_detail = browser_validation_output
        machine.succeed(
          "printf %s "
          + shlex.quote(json.dumps({
            "schema_version": 1,
            "ok": producer_ok,
            "detail": producer_detail,
          }, separators=(",", ":")))
          + " > /var/lib/jaunder/performance/producer-status-v1.json"
        )
        machine.copy_from_machine("/var/lib/jaunder/performance", "")
      ''}
    '';
  };

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

  mkStackConfiguration = stack:
    nixpkgs.lib.nixosSystem {
      inherit system;
      modules = [
        self.nixosModules.jaunder-stack
        ({ ... }: {
          boot.isContainer = true;
          system.stateVersion = "26.05";
          services.jaunder.stack = stack;
        })
      ];
    };
  stackEvaluationSucceeds = stack:
    (builtins.tryEval (mkStackConfiguration stack).config.system.build.toplevel.drvPath).success;
  stackEvaluationFails = stack: !(stackEvaluationSucceeds stack);
  validStackBasicAuth = {
    username = "operator";
    passwordHash = "$2b$12$abcdefghijklmnopqrstuuV4qg5bR1uRgYBzO8pu0h1rlaL8fQ2gQ";
  };
  sixtyThreeCharacterDnsLabel = builtins.concatStringsSep "" (builtins.genList (_: "a") 63);
  invalidDnsHostNames = [
    "https://jaunder.example.test"
    "jaunder.example.test:443"
    "*.jaunder.example.test"
    "jaunder example.test"
    "jaunder\nexample.test"
    "jaunder..example.test"
    ".jaunder.example.test"
    "jaunder.example.test."
    "-jaunder.example.test"
    "jaunder-.example.test"
    "${builtins.concatStringsSep "" (builtins.genList (_: "a") 64)}.example.test"
    "${sixtyThreeCharacterDnsLabel}.${sixtyThreeCharacterDnsLabel}.${sixtyThreeCharacterDnsLabel}.${sixtyThreeCharacterDnsLabel}"
  ];
  invalidApplicationHostStack = mkStackConfiguration {
    enable = true;
    hostName = "https://jaunder.example.test";
  };
  invalidObservabilityHostStack = mkStackConfiguration {
    enable = true;
    hostName = "jaunder.example.test";
    observability = {
      hostName = "https://observe.example.test";
      basicAuth = validStackBasicAuth;
    };
  };
  caseNormalizedHostStack = mkStackConfiguration {
    enable = true;
    hostName = "Jaunder.Example.Test";
    observability = {
      hostName = "Observe.Example.Test";
      basicAuth = validStackBasicAuth;
    };
  };
  sqliteStack = mkStackConfiguration {
    enable = true;
    hostName = "jaunder.example.test";
  };
  postgresStack = mkStackConfiguration {
    enable = true;
    hostName = "jaunder.example.test";
    database = "postgresql";
  };
  postgresFixtureModule =
    { pkgs, ... }:
    {
      # This separate fixture module owns the host's PostgreSQL package and
      # global policy. The stack must merge with it at ordinary priority.
      services.postgresql = {
        package = pkgs.postgresql_16;
        settings.log_min_duration_statement = 4242;
        authentication = "local all all peer";
        ensureDatabases = [ "unrelated" ];
        ensureUsers = [
          {
            name = "unrelated";
            ensureDBOwnership = true;
          }
        ];
      };
    };
  postgresFixtureStack = nixpkgs.lib.nixosSystem {
    inherit system;
    modules = [
      self.nixosModules.jaunder-stack
      postgresFixtureModule
      ({ ... }: {
        system.stateVersion = "26.05";
        services.jaunder.stack = {
          enable = true;
          hostName = "jaunder.example.test";
          database = "postgresql";
        };
      })
    ];
  };
  bcryptStack = mkStackConfiguration {
    enable = true;
    hostName = "jaunder.example.test";
    observability = {
      hostName = "observe.example.test";
      basicAuth = {
        username = "operator";
        passwordHash = "$2b$12$abcdefghijklmnopqrstuuV4qg5bR1uRgYBzO8pu0h1rlaL8fQ2gQ";
      };
    };
  };
  argon2idStack = mkStackConfiguration {
    enable = true;
    hostName = "jaunder.example.test";
    observability = {
      hostName = "observe.example.test";
      basicAuth = {
        username = "operator";
        passwordHash = "$argon2id$v=19$m=65536,t=3,p=4$c29tZXNhbHQ$YWJjZGVmZ2hpams";
      };
    };
  };
  nativeRetentionOverrideStack = nixpkgs.lib.nixosSystem {
    inherit system;
    modules = [
      self.nixosModules.jaunder-stack
      ({ ... }: {
        system.stateVersion = "26.05";
        services.jaunder.stack = {
          enable = true;
          hostName = "jaunder.example.test";
        };
        services.victoriametrics.retentionPeriod = "14d";
        services.victorialogs.extraOptions = [ "-retentionPeriod=14d" ];
        services.victoriatraces.retentionPeriod = "30d";
      })
    ];
  };
  minimalModule = nixpkgs.lib.nixosSystem {
    inherit system;
    modules = [
      self.nixosModules.jaunder
      ({ ... }: { system.stateVersion = "26.05"; })
    ];
  };
  jaunderStackModuleCheck =
    assert sqliteStack.config.services.jaunder.bind == "127.0.0.1:3000";
    assert sqliteStack.config.services.jaunder.prod;
    assert sqliteStack.config.services.jaunder.db == "sqlite:/var/lib/jaunder/data/jaunder.db";
    assert postgresStack.config.services.jaunder.db == "postgresql://jaunder@localhost/jaunder?host=/run/postgresql";
    assert postgresStack.config.services.postgresql.enable;
    assert postgresStack.config.services.postgresql.ensureDatabases == [ "jaunder" ];
    assert postgresFixtureStack.config.services.postgresql.package == pkgs.postgresql_16;
    assert !postgresFixtureStack.config.services.postgresql.enableTCPIP;
    assert postgresFixtureStack.config.services.postgresql.settings.listen_addresses == "localhost";
    assert postgresFixtureStack.config.services.postgresql.settings.log_min_duration_statement == 4242;
    assert postgresFixtureStack.config.networking.firewall.allowedTCPPorts == [ 80 443 ];
    assert sqliteStack.config.networking.firewall.allowedTCPPorts == [ 80 443 ];
    assert sqliteStack.config.services.victoriametrics.listenAddress == "127.0.0.1:8428";
    assert sqliteStack.config.services.victorialogs.listenAddress == "127.0.0.1:9428";
    assert sqliteStack.config.services.victoriatraces.listenAddress == "127.0.0.1:10428";
    assert sqliteStack.config.services.victoriametrics.extraOptions == [ "-http.pathPrefix=/metrics" ];
    assert sqliteStack.config.services.victorialogs.extraOptions == [ "-http.pathPrefix=/logs" ];
    # Metrics and logs omit retention CLI arguments, retaining their native one-month and seven-day defaults.
    assert sqliteStack.config.services.victoriametrics.retentionPeriod == null;
    assert sqliteStack.config.services.victoriatraces.retentionPeriod == "7d";
    assert sqliteStack.config.services.victoriatraces.extraOptions == [ "-http.pathPrefix=/traces" ];
    assert nativeRetentionOverrideStack.config.services.victoriametrics.retentionPeriod == "14d";
    assert builtins.any (option: option == "-retentionPeriod=14d") nativeRetentionOverrideStack.config.services.victorialogs.extraOptions;
    assert nativeRetentionOverrideStack.config.services.victoriatraces.retentionPeriod == "30d";
    assert sqliteStack.config.services.opentelemetry-collector.package == pkgs.opentelemetry-collector-contrib;
    assert sqliteStack.config.systemd.services.opentelemetry-collector.serviceConfig.DynamicUser;
    assert sqliteStack.config.systemd.services.opentelemetry-collector.serviceConfig.SupplementaryGroups == [ "systemd-journal" ];
    assert sqliteStack.config.services.opentelemetry-collector.settings.receivers.journald.units == [ "jaunder.service" ];
    assert sqliteStack.config.services.opentelemetry-collector.settings.exporters.prometheusremotewrite.endpoint == "http://127.0.0.1:8428/metrics/api/v1/write";
    assert sqliteStack.config.services.opentelemetry-collector.settings.exporters."otlphttp/victoriatraces".traces_endpoint == "http://127.0.0.1:10428/traces/insert/opentelemetry/v1/traces";
    assert sqliteStack.config.services.opentelemetry-collector.settings.exporters."otlphttp/victorialogs".logs_endpoint == "http://127.0.0.1:9428/logs/insert/opentelemetry/v1/logs";
    assert builtins.hasAttr "jaunder.example.test" bcryptStack.config.services.caddy.virtualHosts;
    assert builtins.hasAttr "observe.example.test" bcryptStack.config.services.caddy.virtualHosts;
    assert pkgs.lib.hasInfix "reverse_proxy 127.0.0.1:3000" bcryptStack.config.services.caddy.virtualHosts."jaunder.example.test".extraConfig;
    assert pkgs.lib.hasInfix "operator $2b$12$abcdefghijklmnopqrstuuV4qg5bR1uRgYBzO8pu0h1rlaL8fQ2gQ" bcryptStack.config.services.caddy.virtualHosts."observe.example.test".extraConfig;
    assert builtins.any (v: pkgs.lib.hasInfix "basic_auth bcrypt" v.extraConfig) (builtins.attrValues bcryptStack.config.services.caddy.virtualHosts);
    assert builtins.any (v: pkgs.lib.hasInfix "basic_auth argon2id" v.extraConfig) (builtins.attrValues argon2idStack.config.services.caddy.virtualHosts);
    assert builtins.hasAttr "jaunder.example.test" caseNormalizedHostStack.config.services.caddy.virtualHosts;
    assert builtins.hasAttr "observe.example.test" caseNormalizedHostStack.config.services.caddy.virtualHosts;
    assert !(builtins.hasAttr "Jaunder.Example.Test" caseNormalizedHostStack.config.services.caddy.virtualHosts);
    assert !(builtins.hasAttr "https://jaunder.example.test" invalidApplicationHostStack.config.services.caddy.virtualHosts);
    assert !(builtins.hasAttr "https://observe.example.test" invalidObservabilityHostStack.config.services.caddy.virtualHosts);
    assert stackEvaluationSucceeds { enable = true; hostName = "ordinary-host.example.test"; };
    assert stackEvaluationSucceeds { enable = true; hostName = "jaunder.example.test"; };
    assert stackEvaluationSucceeds { enable = true; hostName = "jaunder.example.test"; database = "postgresql"; };
    assert stackEvaluationSucceeds { enable = true; hostName = "jaunder.example.test"; observability = { hostName = "observe.example.test"; basicAuth = { username = "operator"; passwordHash = "$2b$12$abcdefghijklmnopqrstuuV4qg5bR1uRgYBzO8pu0h1rlaL8fQ2gQ"; }; }; };
    assert stackEvaluationSucceeds { enable = true; hostName = "jaunder.example.test"; observability = { hostName = "observe.example.test"; basicAuth = { username = "operator"; passwordHash = "$2a$12$abcdefghijklmnopqrstuuV4qg5bR1uRgYBzO8pu0h1rlaL8fQ2gQ"; }; }; };
    assert stackEvaluationSucceeds { enable = true; hostName = "jaunder.example.test"; observability = { hostName = "observe.example.test"; basicAuth = { username = "operator"; passwordHash = "$argon2id$v=19$m=65536,t=3,p=4$c29tZXNhbHQ$YWJjZGVmZ2hpams"; }; }; };
    assert stackEvaluationFails { enable = true; };
    assert stackEvaluationFails { enable = true; hostName = " "; };
    assert builtins.all (hostName: stackEvaluationFails { enable = true; inherit hostName; }) invalidDnsHostNames;
    assert builtins.all (hostName: stackEvaluationFails {
      enable = true;
      hostName = "jaunder.example.test";
      observability = { inherit hostName; basicAuth = validStackBasicAuth; };
    }) invalidDnsHostNames;
    assert stackEvaluationFails { enable = true; hostName = "jaunder.example.test"; observability.hostName = " "; };
    assert stackEvaluationFails { enable = true; hostName = "jaunder.example.test"; observability.hostName = "observe.example.test"; };
    assert stackEvaluationFails { enable = true; hostName = "jaunder.example.test"; observability = { hostName = "observe.example.test"; basicAuth = { username = " "; passwordHash = "$2b$12$abcdefghijklmnopqrstuuV4qg5bR1uRgYBzO8pu0h1rlaL8fQ2gQ"; }; }; };
    assert stackEvaluationFails { enable = true; hostName = "jaunder.example.test"; observability = { hostName = "observe.example.test"; basicAuth = { username = "operator name"; passwordHash = "$2b$12$abcdefghijklmnopqrstuuV4qg5bR1uRgYBzO8pu0h1rlaL8fQ2gQ"; }; }; };
    assert stackEvaluationFails { enable = true; hostName = "jaunder.example.test"; observability = { hostName = "observe.example.test"; basicAuth = { username = "operator}"; passwordHash = "$2b$12$abcdefghijklmnopqrstuuV4qg5bR1uRgYBzO8pu0h1rlaL8fQ2gQ"; }; }; };
    assert stackEvaluationFails { enable = true; hostName = "jaunder.example.test"; observability = { hostName = "observe.example.test"; basicAuth = { username = "operator\nreverse_proxy"; passwordHash = "$2b$12$abcdefghijklmnopqrstuuV4qg5bR1uRgYBzO8pu0h1rlaL8fQ2gQ"; }; }; };
    assert stackEvaluationFails { enable = true; hostName = "jaunder.example.test"; observability = { hostName = "jaunder.example.test"; basicAuth = { username = "operator"; passwordHash = "$2b$12$abcdefghijklmnopqrstuuV4qg5bR1uRgYBzO8pu0h1rlaL8fQ2gQ"; }; }; };
    assert stackEvaluationFails { enable = true; hostName = "jaunder.example.test"; observability = { hostName = "JAUNDER.EXAMPLE.TEST"; basicAuth = validStackBasicAuth; }; };
    assert stackEvaluationFails { enable = true; hostName = "jaunder.example.test"; observability = { hostName = "observe.example.test"; basicAuth = { username = "operator"; passwordHash = " "; }; }; };
    assert stackEvaluationFails { enable = true; hostName = "jaunder.example.test"; observability = { hostName = "observe.example.test"; basicAuth = { username = "operator"; passwordHash = "plaintext"; }; }; };
    assert stackEvaluationFails { enable = true; hostName = "jaunder.example.test"; observability = { hostName = "observe.example.test"; basicAuth = { username = "operator"; passwordHash = "$2b$12$too-short"; }; }; };
    assert stackEvaluationFails { enable = true; hostName = "jaunder.example.test"; observability = { hostName = "observe.example.test"; basicAuth = { username = "operator"; passwordHash = "$argon2id$v=19$m=65536,t=3,p=4$c29tZXNhbHQ"; }; }; };
    assert stackEvaluationFails { enable = true; hostName = "jaunder.example.test"; observability = { hostName = "observe.example.test"; basicAuth = { username = "operator"; passwordHash = "$argon2id$v=19$m=65536,t=3,p=4$c2FsdA$YWJjZGVmZ2hpams"; }; }; };
    assert stackEvaluationFails { enable = true; hostName = "jaunder.example.test"; observability = { hostName = "observe.example.test"; basicAuth = { username = "operator"; passwordHash = "$argon2id$v=19$m=65536,t=3,p=4$c29tZXNhbHQ$YWJj"; }; }; };
    assert stackEvaluationFails { enable = true; hostName = "jaunder.example.test"; observability = { hostName = "observe.example.test"; basicAuth = { username = "operator"; passwordHash = "$argon2id$v=19$m=65536,t=3,p=4$c29tZXNhbHR$YWJjZGVmZ2hpams"; }; }; };
    assert stackEvaluationFails { enable = true; hostName = "jaunder.example.test"; observability = { hostName = "observe.example.test"; basicAuth = { username = "operator"; passwordHash = "$argon2id$v=19$m=65536,t=3,p=4$c29tZXNhbHQ$YWJjZGVmZ2hpamt"; }; }; };
    assert stackEvaluationFails { enable = true; hostName = "jaunder.example.test"; observability = { hostName = "observe.example.test"; basicAuth = { username = "operator"; passwordHash = "$scrypt$ln=16,r=8,p=1$c2FsdA$aGFzaA"; }; }; };
    assert stackEvaluationFails { enable = true; hostName = "jaunder.example.test"; database = "mysql"; };
    assert !(builtins.hasAttr "stack" minimalModule.options.services.jaunder);
    pkgs.runCommand "jaunder-stack-module" { } ''
      touch $out
    '';
  mkJaunderStackVmCheck =
    {
      checkName,
      passwordHash,
      captureSignals ? false,
      persistSignals ? false,
      database ? "sqlite",
    }:
    pkgs.testers.nixosTest {
      name = checkName;
      globalTimeout = 600;
      nodes.machine =
        { lib, pkgs, ... }:
        {
          imports = [ self.nixosModules.jaunder-stack ] ++ lib.optional (database == "postgresql") postgresFixtureModule;
          virtualisation.memorySize = 2048;
          boot.loader.grub.devices = [ "nodev" ];
          environment.systemPackages = [
            pkgs.curl
            pkgs.gnugrep
            pkgs.gawk
            pkgs.iproute2
            pkgs.jq
            pkgs.procps
          ] ++ lib.optionals (database == "postgresql") [
            pkgs.iptables
            pkgs.postgresql_16
          ];
          services.jaunder.stack = {
            enable = true;
            inherit database;
            hostName = "jaunder.stack.test";
            observability = {
              hostName = "observe.stack.test";
              basicAuth = {
                username = "operator";
                inherit passwordHash;
              };
            };
          };
          # Public ACME is an operator contract. The VM has no public DNS, so
          # only this test replaces it with Caddy's deterministic local CA.
          services.caddy.virtualHosts."jaunder.stack.test".extraConfig = lib.mkAfter ''
            tls internal
          '';
          services.caddy.virtualHosts."observe.stack.test".extraConfig = lib.mkAfter ''
            tls internal
          '';
          # The production default is intentionally quiet; the test drives an
          # INFO request event so the journald parser has a named field to prove.
          systemd.services.jaunder.environment.RUST_LOG = "info";
          system.stateVersion = "26.05";
        };
      testScript = ''
        import json
        ${pkgs.lib.optionalString captureSignals ''
        import shlex
        import urllib.parse
        ''}
        curl_options = "--connect-timeout 5 --max-time 20"

        def caddy_status(path, credentials=""):
          return machine.succeed(
            "curl " + curl_options + " -ksS -o /dev/null -w '%{http_code}'"
            + " --resolve observe.stack.test:443:127.0.0.1"
            + credentials
            + " https://observe.stack.test" + path
          )

        def curl_json(command):
          status, output = machine.execute(command)
          assert status == 0, "request failed: %s\n%s" % (command, output)
          try:
            return json.loads(output)
          except json.JSONDecodeError as error:
            raise AssertionError("invalid JSON from %s: %s\n%s" % (command, error, output)) from error

        def assert_ingress():
          for path in ["/metrics/", "/logs/", "/traces/"]:
            assert caddy_status(path) == "401", "unauthenticated ingress unexpectedly allowed %s" % path
          for path in [
            "/metrics/",
            "/metrics/api/v1/query?query=jaunder_db_pool_max",
            "/logs/",
            "/logs/select/logsql/query?query=uri:*",
            "/traces/",
            "/traces/select/jaeger/api/services",
          ]:
            assert caddy_status(path, " -u operator:stack-password") == "200", path

        def assert_local_uis():
          for port, prefix in [(8428, "metrics"), (9428, "logs"), (10428, "traces")]:
            machine.succeed("curl " + curl_options + " -fsS http://127.0.0.1:%d/%s/ > /dev/null" % (port, prefix))

        def wait_for_stack_ready():
          for unit in [
            "caddy.service",
            "jaunder.service",
            "opentelemetry-collector.service",
            "victoriametrics.service",
            "victorialogs.service",
            "victoriatraces.service",
          ]:
            machine.wait_for_unit(unit, timeout=90)
          for port in [80, 443, 3000, 4317, 4318, 8428, 9428, 10428]:
            machine.wait_for_open_port(port, timeout=60)

        def assert_listener_contract():
          listeners = machine.succeed("ss -ltnpH").splitlines()

          def port(local_address):
            return int(local_address.rsplit(":", 1)[1])

          def loopback(local_address):
            return local_address.startswith("127.") or local_address.startswith("[::1]")

          loopback_ports = [3000, 4317, 4318, 8428, 9428, 10428]
          for expected_port in loopback_ports:
            rows = [row for row in listeners if port(row.split()[3]) == expected_port]
            assert rows, "no listener found for required loopback port %d" % expected_port
            exposed = [row for row in rows if not loopback(row.split()[3])]
            assert not exposed, "non-loopback listener on port %d:\n%s" % (expected_port, "\n".join(exposed))

          non_loopback = [row for row in listeners if not loopback(row.split()[3])]
          unexpected = [
            row for row in non_loopback
            if port(row.split()[3]) not in [80, 443] or "caddy" not in row
          ]
          assert not unexpected, "unexpected non-loopback TCP listeners:\n%s" % "\n".join(unexpected)
          for caddy_port in [80, 443]:
            rows = [row for row in non_loopback if port(row.split()[3]) == caddy_port]
            assert rows, "no non-loopback Caddy listener found on port %d" % caddy_port

        def log_records(command):
          status, output = machine.execute(command)
          assert status == 0, "log query failed: %s\n%s" % (command, output)
          try:
            return [json.loads(line) for line in output.splitlines() if line]
          except json.JSONDecodeError as error:
            raise AssertionError("invalid VictoriaLogs record: %s\n%s" % (error, output)) from error

        machine.start(allow_reboot=True)
        wait_for_stack_ready()

        ${pkgs.lib.optionalString (database == "postgresql") ''
          # PostgreSQL cold-boots with the stack. Its native readiness target
          # must complete before Jaunder's peer-authenticated initialization.
          machine.wait_for_unit("postgresql.service", timeout=90)
          machine.succeed("systemctl is-active postgresql.target")
          machine.succeed(
            "systemctl show --property After --value jaunder.service"
            + " | tr ' ' '\\n' | grep -Fx postgresql.target"
          )
          machine.succeed(
            "systemctl show --property Requires --value jaunder.service"
            + " | tr ' ' '\\n' | grep -Fx postgresql.target"
          )
          machine.succeed(
            "pid=$(systemctl show --property MainPID --value jaunder.service)"
            + "; tr '\\0' '\\n' < /proc/$pid/environ"
            + " | grep -Fx 'JAUNDER_DB=postgresql://jaunder@localhost/jaunder?host=/run/postgresql'"
          )
          machine.succeed(
            "test -S /run/postgresql/.s.PGSQL.5432"
            + " && ss -ltnH | awk '$4 ~ /:5432$/ && $4 !~ /^127\\./ && $4 !~ /^::1:/ && $4 !~ /^\\[::1\\]/ { exit 1 }'"
          )
          machine.succeed(
            "runuser -u postgres -- psql -d postgres -Atqc 'SHOW server_version_num'"
            + " | grep -Ex '16[0-9]{4}'"
          )
          machine.succeed(
            "test \"$(runuser -u postgres -- psql -d postgres -Atqc 'SHOW listen_addresses')\" = \"localhost\""
            + " && runuser -u postgres -- psql -d postgres -Atqc 'SHOW log_min_duration_statement'"
            + " | grep -Fx '4242ms'"
          )
          machine.succeed(
            "runuser -u jaunder -- psql -h /run/postgresql -d jaunder -Atqc "
            + shlex.quote("SELECT current_user = 'jaunder' AND current_database() = 'jaunder'")
            + " | grep -Fx t"
          )
          machine.succeed(
            "runuser -u postgres -- psql -d postgres -Atqc "
            + shlex.quote("SELECT datdba::regrole = 'jaunder'::regrole FROM pg_database WHERE datname = 'jaunder'")
            + " | grep -Fx t"
          )
          role_password_status, role_password = machine.execute(
            "runuser -u postgres -- psql -d postgres -Atqc "
            + shlex.quote("SELECT rolcanlogin, rolpassword IS NULL FROM pg_authid WHERE rolname = 'jaunder'")
          )
          assert role_password_status == 0 and role_password.strip() == "t|t", (
            "jaunder role is not a passwordless login role: %s" % role_password
          )
          table_ownership_status, table_ownership = machine.execute(
            "runuser -u jaunder -- psql -h /run/postgresql -d jaunder -Atqc "
            + shlex.quote("SELECT count(*), coalesce(bool_and(tableowner = 'jaunder'), false) FROM pg_tables WHERE schemaname = 'public'")
          )
          assert table_ownership_status == 0 and table_ownership.strip() != "0|f" and table_ownership.strip().endswith("|t"), (
            "application tables are not owned by jaunder: %s" % table_ownership
          )
          machine.succeed(
            "runuser -u postgres -- psql -d postgres -Atqc "
            + shlex.quote("SELECT datdba::regrole = 'unrelated'::regrole FROM pg_database WHERE datname = 'unrelated'")
            + " | grep -Fx t"
          )
          machine.succeed(
            "runuser -u postgres -- psql -d postgres -Atqc "
            + shlex.quote("SELECT rolcanlogin FROM pg_roles WHERE rolname = 'unrelated'")
            + " | grep -Fx t"
          )
          machine.succeed(
            "hba=$(runuser -u postgres -- psql -d postgres -Atqc 'SHOW hba_file')"
            + "; grep -Eq '^local[[:space:]]+all[[:space:]]+all[[:space:]]+peer$' \"$hba\""
            + "; ! grep -Eq '^[[:space:]]*host[[:space:]]' \"$hba\""
            + "; pid=$(systemctl show --property MainPID --value jaunder.service)"
            + "; ! tr '\\0' '\\n' < /proc/$pid/environ | grep -Eq '^JAUNDER_DB_PASSWORD(=|_)'"
            + "; ! tr '\\0' '\\n' < /proc/$pid/environ | grep -Eqi 'password='"
          )
          machine.succeed(
            "test \"$(iptables -S nixos-fw | awk '$1 == \"-A\" && $2 == \"nixos-fw\" && $3 == \"-p\" && $4 == \"tcp\" && $5 == \"-m\" && $6 == \"tcp\" && $7 == \"--dport\" { print $8 }' | sort -n | paste -sd, -)\" = \"80,443\""
          )
        ''}

        # The application request crosses the public Caddy seam, rather than
        # reaching Jaunder's loopback listener directly.
        machine.succeed(
          "curl " + curl_options + " -ksSf --resolve jaunder.stack.test:443:127.0.0.1"
          + " https://jaunder.stack.test/ > /dev/null"
        )
        assert_local_uis()
        assert_ingress()

        assert_listener_contract()
        collector_pid = machine.succeed(
          "systemctl show --value --property MainPID opentelemetry-collector.service"
        ).strip()
        machine.succeed("test \"%s\" -gt 0" % collector_pid)
        machine.succeed("test \"$(ps -o uid= -p %s | tr -d ' ')\" != 0" % collector_pid)
        machine.succeed(
          "journal_gid=$(getent group systemd-journal | cut -d: -f3)"
          + "; grep -Eq \"^Groups:.*(^|[[:space:]])$journal_gid([[:space:]]|$)\""
          + " /proc/%s/status" % collector_pid
        )
        machine.succeed(
          "systemctl show --property SupplementaryGroups --value opentelemetry-collector.service"
          + " | grep -Fx systemd-journal"
        )

        ${pkgs.lib.optionalString captureSignals ''
          trace_id = "0123456789abcdef0123456789abcdef"
          request_id = "jaunder-stack-telemetry-request"
          authorization = "Bearer jaunder-stack-fake-authorization-credential"
          cookie = "session=jaunder-stack-fake-cookie-credential"
          telemetry_uri = "/atompub/nonexistent/posts"
          status, output = machine.execute(
            "curl " + curl_options + " -ksS -o /dev/null -w '%{http_code}'"
            + " --resolve jaunder.stack.test:443:127.0.0.1"
            + " -H " + shlex.quote("traceparent: 00-" + trace_id + "-0123456789abcdef-01")
            + " -H " + shlex.quote("x-request-id: " + request_id)
            + " -H " + shlex.quote("authorization: " + authorization)
            + " -H " + shlex.quote("cookie: " + cookie)
            + " https://jaunder.stack.test" + telemetry_uri
          )
          assert status == 0 and output == "401", "telemetry request did not return 401:\n%s" % output

          metric_command = (
            "curl " + curl_options + " -fsSG"
            + " --data-urlencode " + shlex.quote('query=jaunder_atompub_requests_total{op="collection_get",result="client_error"}')
            + " http://127.0.0.1:8428/metrics/api/v1/query"
          )
          metric_sample = None
          for _ in range(120):
            candidate = curl_json(metric_command)
            results = candidate.get("data", {}).get("result", [])
            if candidate.get("status") == "success" and len(results) == 1:
              metric_sample = results[0]
              break
            machine.sleep(1)
          assert metric_sample is not None, "driven AtomPub metric never appeared before reboot"
          metric_identity = metric_sample["metric"]
          metric_value = metric_sample["value"]
          assert all(metric_identity.get(label) == value for label, value in {
            "__name__": "jaunder_atompub_requests_total",
            "op": "collection_get",
            "result": "client_error",
          }.items()), "unexpected driven metric identity: %s" % metric_identity
          assert len(metric_value) == 2, "metric sample lacks timestamp/value: %s" % metric_sample

          log_command = (
            "curl " + curl_options + " -fsSG --data-urlencode 'query=* | limit 10000'"
            + " http://127.0.0.1:9428/logs/select/logsql/query"
          )
          target_log = None
          for _ in range(120):
            records = log_records(log_command)
            target_log = next((
              record for record in records
              if record.get("jaunder.target") == "tower_http::trace::on_response"
              and record.get("jaunder.request.uri") == telemetry_uri
            ), None)
            if target_log is not None:
              break
            machine.sleep(1)
          assert target_log is not None, "driven structured response log never appeared before reboot:\n%s" % records[-10:]
          assert "_time" in target_log, "structured response log lacks a timestamp: %s" % target_log
          target_log_identity = json.dumps(target_log, sort_keys=True, separators=(",", ":"))
          assert request_id not in target_log_identity, "request header marker reached VictoriaLogs: %s" % target_log
          for credential in [authorization, cookie]:
            assert credential not in target_log_identity, "request credential reached VictoriaLogs: %s" % target_log
          assert "jaunder.request.headers" not in target_log, "request headers were promoted to VictoriaLogs: %s" % target_log

          trace_command = (
            "curl " + curl_options + " -fsS http://127.0.0.1:10428/traces/select/jaeger/api/traces/"
            + urllib.parse.quote(trace_id, safe="")
          )
          trace_payload = None
          for _ in range(120):
            candidate = curl_json(trace_command)
            if any(trace.get("traceID") == trace_id for trace in candidate.get("data", [])):
              trace_payload = candidate
              break
            machine.sleep(1)
          assert trace_payload is not None, "driven trace ID %s never appeared before reboot" % trace_id
          trace_payload_identity = json.dumps(trace_payload, sort_keys=True, separators=(",", ":"))
          for credential in [authorization, cookie]:
            assert credential not in trace_payload_identity, "request credential reached VictoriaTraces: %s" % trace_payload

          ${pkgs.lib.optionalString persistSignals ''
          machine.reboot()
          wait_for_stack_ready()
          machine.succeed(
            "curl " + curl_options + " -ksSf --resolve jaunder.stack.test:443:127.0.0.1"
            + " https://jaunder.stack.test/ > /dev/null"
          )
          assert_local_uis()
          assert_ingress()
          assert_listener_contract()

          metric_at_capture_time = curl_json(
            "curl " + curl_options + " -fsSG"
            + " --data-urlencode " + shlex.quote('query=jaunder_atompub_requests_total{op="collection_get",result="client_error"}')
            + " --data-urlencode " + shlex.quote("time=" + str(metric_value[0]))
            + " http://127.0.0.1:8428/metrics/api/v1/query"
          )
          persisted_metrics = metric_at_capture_time.get("data", {}).get("result", [])
          assert any(
            result.get("metric") == metric_identity and result.get("value") == metric_value
            for result in persisted_metrics
          ), "driven pre-reboot metric sample is absent at its captured timestamp"

          persisted_logs = log_records(log_command)
          assert any(
            json.dumps(record, sort_keys=True, separators=(",", ":")) == target_log_identity
            for record in persisted_logs
          ), "exact driven pre-reboot response log is absent"

          trace_payload = curl_json(trace_command)
          assert any(trace.get("traceID") == trace_id for trace in trace_payload.get("data", [])), (
            "driven pre-reboot trace ID %s is absent" % trace_id
          )
          ''}
        ''}
      '';
    };
  # These are the existing cache-filtered support derivations in the e2e
  # graph. Exposing them does not create work: e2e checks already depend on
  # both derivations, whose output names match the current broad filter.
  e2eTestDriverPackages =
    let
      drivers = checks:
        pkgs.lib.mapAttrs' (name: check: {
          name = "${name}-driver";
          value = check.driver;
        }) checks;
    in
    drivers e2eGateChecks // drivers e2eSingleWorkerPackages;
  e2eSupportPackages = {
    # These derivations already sit beneath each NixOS test result and have
    # names caught by the broad Cachix exclusion. They contain test machinery,
    # not test evidence.
    e2e-support = e2ePackage;
    e2e-npm-deps = e2ePackage.npmDeps;
  }
  // e2eTestDriverPackages;
  # This concern-owned attrset is the sole definition of Rust coverage outputs.
  # The source probe is preparation only; the producer and its consumer are
  # verdict-bearing and therefore final.
  coverageFinalCacheChecks = rec {
    coverage = nonSubstitutable (craneLib.mkCargoDerivation (
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
    ));
    # Belt-and-suspenders: this sandbox consumer validates completed producer
    # evidence through the shared Rust contract, while host xtask consumes its
    # reports separately. Its `jaunder-coverage-gate` name stays broad-filtered.
    coverage-gate = nonSubstitutable (pkgs.runCommand "jaunder-coverage-gate" { nativeBuildInputs = [ devtoolBin ]; } ''
      devtool coverage validate-status --status ${coverage}/status.json
      touch $out
    '');
  };
  coverageSupportCacheChecks = {
    # Probe-only identity: its sole varying input is the filtered coverage
    # source. Keep this separate from coverage.drvPath, which also includes the
    # coverage producer's tooling and runtime inputs.
    coverage-source-probe = pkgs.runCommand "jaunder-coverage-source-probe" { src = coverageSrc; } ''
      touch $out
    '';
  };
  coverageCacheChecks = coverageFinalCacheChecks // coverageSupportCacheChecks;
  coveragePartitionAssertion =
    let
      final = builtins.attrNames coverageFinalCacheChecks;
      support = builtins.attrNames coverageSupportCacheChecks;
      complete = builtins.attrNames coverageCacheChecks;
    in
    assert pkgs.lib.intersectLists final support == [ ];
    assert builtins.sort builtins.lessThan complete == builtins.sort builtins.lessThan (final ++ support);
    true;
  # These classifications are derived directly from the concern-owned attrsets.
  # The JSON inventory below is the probe's authority; cache-policy.json must
  # reconcile to it and cannot classify an attr by name convention.
  cacheSafetyFinalAttrs =
    assert coveragePartitionAssertion;
    (map (name: "checks.${system}.${name}") (builtins.attrNames coverageFinalCacheChecks))
    ++ (map (name: "checks.${system}.${name}") (builtins.attrNames e2eGateChecks))
    ++ [ "checks.${system}.e2e" "packages.${system}.e2e-checks" ]
    ++ (map (name: "packages.${system}.${name}") (builtins.attrNames e2eSingleWorkerPackages));
  cacheSafetyDriverAttrs =
    map (name: "packages.${system}.${name}") (builtins.attrNames e2eTestDriverPackages);
  cacheSafetyPackageSupportAttrs = map (name: "packages.${system}.${name}") [ "e2e-support" "e2e-npm-deps" ];
  cacheSafetySupportAttrs =
    (map (name: "checks.${system}.${name}") (builtins.attrNames coverageSupportCacheChecks))
    ++ cacheSafetyPackageSupportAttrs
    ++ cacheSafetyDriverAttrs;
  cacheSafetySourceFamilies = {
    coverage = {
      categories = [
        { name = "cargo-workspace"; relevant = "server/src/lib.rs"; excluded = "xtask/src/main.rs"; }
        { name = "nextest-profile"; relevant = ".config/nextest.toml"; excluded = "docs/README.md"; }
        { name = "csr-shell"; relevant = "csr/index.html"; excluded = "flake.nix"; }
        { name = "embedded-assets"; relevant = "server/assets/jaunder.css"; excluded = "end2end/tests/visual.css"; }
        { name = "migrations"; relevant = "storage/migrations/sqlite/0001_create_site_config.sql"; excluded = "xtask/src/main.rs"; }
        { name = "backup-corpus"; relevant = "server/tests/misc/backup_corpus/index.json"; excluded = "docs/README.md"; }
      ];
    };
    e2e-package = {
      # buildNpmPackage's `src = ../end2end` is the universal boundary; these
      # finite paths are regression arms, not a substitute for that input.
      categories = [
        { name = "npm-manifest"; relevant = "end2end/package.json"; excluded = "docs/README.md"; }
        { name = "npm-lock"; relevant = "end2end/package-lock.json"; excluded = "xtask/src/main.rs"; }
        { name = "playwright-config"; relevant = "end2end/playwright.config.ts"; excluded = "docs/README.md"; }
        { name = "playwright-tests"; relevant = "end2end/tests/fixtures.ts"; excluded = "xtask/src/main.rs"; }
        { name = "otel-config"; relevant = "end2end/otel-collector.yaml"; excluded = "docs/README.md"; }
      ];
    };
    e2e-driver = {
      # Driver expressions explicitly interpolate jaunderBin, e2ePackage, and
      # this NixOS test definition; all three are inputs to every driver.
      categories = [
        { name = "application"; relevant = "server/src/lib.rs"; excluded = "docs/README.md"; }
        { name = "e2e-package"; relevant = "end2end/tests/fixtures.ts"; excluded = "xtask/src/main.rs"; }
        { name = "nix-test-definition"; relevant = "nix/checks.nix"; excluded = "docs/README.md"; }
      ];
    };
  };
  cacheSafetySupportFamilies =
    (map (attr: { inherit attr; family = "coverage"; }) (map (name: "checks.${system}.${name}") (builtins.attrNames coverageSupportCacheChecks)))
    ++ (map (attr: { inherit attr; family = "e2e-package"; }) cacheSafetyPackageSupportAttrs)
    ++ (map (attr: { inherit attr; family = "e2e-driver"; }) cacheSafetyDriverAttrs);
in
{

  internals = {
    inherit mkPerformanceProducer;
  };
  packages = pkgs.lib.optionalAttrs pkgs.stdenv.isLinux (
    e2eSupportPackages
    // {
      # The probe realizes only this declarative inventory before it evaluates
      # closures. The inventory is not a cache-policy output.
      cache-safety-inventory = pkgs.writeText "jaunder-cache-safety-inventory.json" (
        builtins.toJSON {
          schemaVersion = 1;
          finalAttrs = cacheSafetyFinalAttrs;
          supportAttrs = cacheSafetySupportAttrs;
          supportFamilies = cacheSafetySupportFamilies;
          sourceFamilies = cacheSafetySourceFamilies;
        }
      );
# The e2e aggregate: a symlinkJoin of every browser/backend `e2e-*`
# check, exposed as `checks.e2e` and built by `cargo xtask validate`.
# Adding a new browser/backend combo automatically joins it here. Its
# `jaunder-e2e*` name keeps it out of the cachix push, so building it
# always realizes the underlying VM checks rather than substituting a
# cached aggregate.
e2e-checks = nonSubstitutable (pkgs.symlinkJoin {
  name = "jaunder-e2e-checks";
  paths = builtins.attrValues (
    pkgs.lib.filterAttrs (name: _: pkgs.lib.hasPrefix "e2e-" name) self.checks.${system}
  );
});
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
// coverageCacheChecks
// {
  wasm-tests = assert sourceMembershipAssertions; craneLib.cargoTest (
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
  # auto`; a plain dev box defaults to 1 and runs them serially). The
  # aggregate's name stays under `jaunder-e2e*`, so the cachix
  # pushFilter still excludes it — the VM runs are never substituted
  # from a cached aggregate.
  e2e = self.packages.${system}.e2e-checks;

  jaunder-stack-module = jaunderStackModuleCheck;
  jaunder-stack-sqlite-bcrypt = mkJaunderStackVmCheck {
    checkName = "jaunder-stack-sqlite-bcrypt";
    passwordHash = "$2a$14$3XbcVHEiOPQs7JeFsE4L6.viyrrG.5pCGkdC5yzH5WK4pGCIm4u4S";
    captureSignals = true;
    persistSignals = true;
  };
  jaunder-stack-sqlite-argon2id = mkJaunderStackVmCheck {
    checkName = "jaunder-stack-sqlite-argon2id";
    passwordHash = "$argon2id$v=19$m=47104,t=1,p=1$lF4nDRbJX4Fmyz51MRZ4+Q$YArAYMGOutNEtB7Pv8Fa9CNZ75tfV+5W3kUvP8m+7gQ";
  };
  jaunder-stack-postgresql = mkJaunderStackVmCheck {
    checkName = "jaunder-stack-postgresql";
    passwordHash = "$2a$14$3XbcVHEiOPQs7JeFsE4L6.viyrrG.5pCGkdC5yzH5WK4pGCIm4u4S";
    database = "postgresql";
    captureSignals = true;
  };

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

# The docs-only build preserves `devtool`'s single command catalog without
# importing the performance producer's product-storage source closure.
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
      nativeBuildInputs = [ docsDevtoolBin pkgs.prettier ];
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
