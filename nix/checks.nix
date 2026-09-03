{ self, system, pkgs, nixosInternals, packageInternals }:
let
  inherit (nixosInternals) captureEnv e2eOtelCollectorEnv;
  inherit (packageInternals)
    visualFontConfig
    toolchain
    craneLib
    commonArgs
    appOfflineCargoHome
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
    emacsSrc
    emacsForCi
    ;

# #93 / ADR-0032: shared zero-panic gate appended to each e2e testScript.
# A server Rust panic is isolated (tests still pass), so without this it
# gets cached green and stays invisible. Dump the service journal, copy it
# to $out before asserting, then run the shared Rust verifier from
# `test-support`. It scans raw bytes from the union of the scoped diagnostic
# stream (#144/#227) and the journal fallback, de-duplicates by panic
# location with the scoped record winning, and owns the default-empty
# source-controlled allowlist. The CLI receives the capture directory rather
# than restating the diagnostic filename defined by `host::capture`.
e2ePanicGate = backend: ''
  machine.succeed("journalctl -u jaunder.service --no-pager -o cat > /tmp/jaunder-journal-${backend}.log")
  # copy_from_machine's 2nd arg is a target *directory*; "" lands the file
  # flat at $out/jaunder-journal-${backend}.log (the per-backend name comes
  # from the source).
  machine.copy_from_machine("/tmp/jaunder-journal-${backend}.log", "")
  machine.succeed(
      "test-support verify-no-panics"
      + " --capture-dir /var/lib/jaunder/capture"
      + " --server-log /tmp/jaunder-journal-${backend}.log"
  )
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
# ~40 s, so 180 s is ~4x headroom.
e2ePlaywrightTimeout = 1020;
e2eGlobalTimeout = 1200;

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
    pw_status, pw_out = machine.execute(
      "cd /tmp/e2e"
      + " && PLAYWRIGHT_BROWSERS_PATH=${pkgs.playwright-driver.browsers}"
      + " PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD=1"
      + " FONTCONFIG_FILE=${visualFontConfig}"
      + "${extraEnv}"
      + " JAUNDER_CAPTURE_DIR=/var/lib/jaunder/capture"
      + " JAUNDER_DB=${jaunderDb}"
      + " JAUNDER_E2E_TRACE_ID=${traceId}"
      + " JAUNDER_E2E_TRACEPARENT=${traceParent}"
      + " JAUNDER_E2E_OTLP_HTTP_ENDPOINT=http://127.0.0.1:4318/v1/traces"
      + " ${pkgs.nodejs}/bin/node node_modules/.bin/playwright test"
      + " --config playwright.config.ts"
      + " --project ${browser} --project ${browser}-admin",
      timeout=${toString e2ePlaywrightTimeout},
    )
    # Stream the Playwright line-reporter output into the build log (-L), so
    # the failing test + assertion are recoverable from build.log alone,
    # even on failure and without --keep-failed.
    print(pw_out)

    # Stop otel so its trace flushes; ignore status (best-effort capture).
    machine.execute("systemctl stop otel-collector.service")

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

    ${e2ePanicGate backend}

    # Fail the check now — after all artifacts are safely copied out.
    assert pw_status == 0, "e2e Playwright failed (exit %d) for ${backend}/${browser}; see playwright-report-${backend}.json + duration-budget-manifest-${backend}.json + playwright-artifacts-${backend}.tar.gz + build.log" % pw_status
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
          package = pkgs.postgresql_16;
          jaunderDb = "postgres://jaunder:testpassword@127.0.0.1/jaunder";
          nodeConfig = lib: {
            services.postgresql = {
              enable = true;
              package = pkgs.postgresql_16;
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
    # fails near 20 min instead of burning the full hour. See issue #130.
    # This is the OUTER budget: `e2ePlaywrightTimeout` above expires first
    # and is the one sized against the test run itself (~10.6 min for the
    # slowest single-browser combo, so ~1.6x headroom).
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
      ${e2eOtelTestHelpers}${beforeMachineStart}machine.start()
      machine.wait_for_unit("otel-collector.service", timeout=60)
      # `active` precedes the OTLP receiver binds; seeding immediately can
      # export into that gap and leave no trace population to verify.
      wait_for_otel_receivers()${setupBeforeJaunder}machine.succeed("systemctl start jaunder.service")
      machine.wait_for_unit("jaunder.service", timeout=60)
      machine.wait_for_open_port(3000, timeout=30)

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
in
{
  packages = pkgs.lib.optionalAttrs pkgs.stdenv.isLinux (
    {
# The e2e aggregate: a symlinkJoin of every browser/backend `e2e-*`
# check, exposed as `checks.e2e` and built by `cargo xtask validate`.
# Adding a new browser/backend combo automatically joins it here. Its
# `jaunder-e2e*` name keeps it out of the cachix push, so building it
# always realizes the underlying VM checks rather than substituting a
# cached aggregate.
e2e-checks = pkgs.symlinkJoin {
  name = "jaunder-e2e-checks";
  paths = builtins.attrValues (
    pkgs.lib.filterAttrs (name: _: pkgs.lib.hasPrefix "e2e-" name) self.checks.${system}
  );
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
      cargoArtifacts = craneLib.buildDepsOnly (
        commonArgs
        // leanTestProfile
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

# The static checks (#188/#276), unified behind one `devtool check
# --all --sandbox-cargo`. The host verify ladder runs the same
# devtool definitions through host-local lanes, while this derivation
# proves those definitions hermetically with offline Cargo homes.
static-checks =
  let
    staticCheckSrc = pkgs.lib.cleanSourceWith {
      src = craneLib.path ../.;
      # Rust + end2end/ + elisp/ + tools/ + all *.md + the prettier config.
      # Exclusion-only; keep the working tree clean when building locally.
      filter =
        path: _type:
        !(pkgs.lib.hasInfix "/xtask/" path)
        && !(pkgs.lib.hasInfix "/node_modules" path)
        && !(pkgs.lib.hasInfix "/target/" path)
        && !(pkgs.lib.hasInfix "/.direnv/" path);
    };
  in
  pkgs.runCommand "static-checks"
    {
      nativeBuildInputs = [
        pkgs.stdenv.cc
      ]
      ++ commonArgs.nativeBuildInputs
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
      buildInputs = commonArgs.buildInputs;
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
      cp --no-preserve=mode -r ${staticCheckSrc} src
      cd src
      devtool check --all --sandbox-cargo
      touch $out
    '';
coverage = craneLib.mkCargoDerivation (
  commonArgs
  // {
    src = pkgs.lib.cleanSourceWith {
      src = craneLib.path ../.;
      filter =
        path: type:
        # Coverage-specific exclusions: none of these are
        # instrumented, and admitting them would let unrelated edits
        # bust the coverage cache. xtask/ is the host-only driver;
        # tools/, docs/, .github/, elisp/, and top-level *.md are
        # non-source.
        # Nix assembly is not coverage source; exclude only its top-level root.
        !(type == "directory" && path == "${toString (craneLib.path ../.)}/nix")
        && !(pkgs.lib.hasInfix "/xtask/" path)
        && !(pkgs.lib.hasInfix "/tools/" path)
        && !(pkgs.lib.hasInfix "/docs/" path)
        && !(pkgs.lib.hasInfix "/.github/" path)
        && !(pkgs.lib.hasInfix "/elisp/" path)
        && !(pkgs.lib.hasSuffix ".md" path)
        && (
          # Cargo-source ADMISSION clause (mirrors commonArgs.src
          # :272-289): without it, ANY untracked non-gitignored file
          # (a stray .txt, an editor temp) would enter the derivation
          # and change its hash — impure (#37). Only buildable inputs
          # are admitted.
          (pkgs.lib.hasSuffix ".sql" path)
          || (pkgs.lib.hasSuffix ".css" path)
          || (builtins.match "scripts/.*" path != null)
          # web/src/app/render.rs `include_str!`s csr/index.html
          # inside a #[test], so the instrumented coverage BUILD needs
          # it at compile time. filterCargoSources drops .html, so
          # re-admit it explicitly or the build fails to compile.
          || (pkgs.lib.hasSuffix "csr/index.html" path)
          || (craneLib.filterCargoSources path type)
        );
    };
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
    nativeBuildInputs = commonArgs.nativeBuildInputs ++ [
      devtoolBin
      cargo-crap
      pkgs.cargo-llvm-cov
      pkgs.cargo-nextest
      # devtool runs the whole test suite under an ephemeral
      # PostgreSQL (via devtool pg) so
      # storage/src/postgres/* gets instrumented coverage. The
      # throwaway cluster needs initdb/pg_ctl/psql available inside
      # the build sandbox.
      pkgs.postgresql_16
    ];
    buildPhaseCargoCommand = ''
      export LD_LIBRARY_PATH="${pkgs.lib.makeLibraryPath [ pkgs.openssl ]}:''${LD_LIBRARY_PATH:-}"
      mkdir -p emit-out
      # devtool always exits 0 after writing emit-out/status.json;
      # gating is the coverage-gate consumer derivation + host xtask.
      devtool coverage emit --out emit-out
    '';
    installPhaseCommand = ''
      mkdir -p $out
      # emit-out/coverage-report.lcov is intentionally NOT copied: it
      # is an intermediate consumed only by `cargo crap`, not a gate
      # output the host reads.
      cp emit-out/coverage-report.txt $out/coverage-report.txt
      cp emit-out/crap-report.json $out/crap-report.json
      cp emit-out/status.json $out/status.json
      cp -r emit-out/diagnostics $out/diagnostics
    '';
  }
);
# Belt-and-suspenders: an independent Nix-level red for in-sandbox
# failures (test/infra) even if a caller bypasses host xtask. The
# coverage-regression verdict is host-only (needs committed baselines
# + git) and lives in xtask, not here. Named `jaunder-coverage-gate`
# so the cachix pushFilter (jaunder-coverage|jaunder-e2e) excludes it.
coverage-gate =
  pkgs.runCommand "jaunder-coverage-gate"
    {
      nativeBuildInputs = [ pkgs.jq ];
    }
    ''
      cat ${self.checks.${system}.coverage}/status.json
      cat=$(jq -r .category ${self.checks.${system}.coverage}/status.json)
      if [ "$cat" != "tests-ok" ]; then
        echo "coverage gate failed: category=$cat" >&2
        jq -r '.infra_detail // (.failed_tests | join("\n"))' \
          ${self.checks.${system}.coverage}/status.json >&2
        exit 1
      fi
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
  commonArgs
  // {
    cargoArtifacts = craneLib.buildDepsOnly (commonArgs // leanDevAndTestProfile);
    pname = "jaunder-doctests";
    # Doctest output comes from rustdoc/libtest diagnostics and the
    # fence reconciler, not DWARF. Keep the override local so manual
    # `cargo test --doc` remains fully debuggable.
    CARGO_PROFILE_DEV_DEBUG = "0";
    CARGO_PROFILE_TEST_DEBUG = "0";
    nativeBuildInputs = commonArgs.nativeBuildInputs ++ [ devtoolBin ];
    buildPhaseCargoCommand = ''
      export LD_LIBRARY_PATH="${pkgs.lib.makeLibraryPath [ pkgs.openssl ]}:''${LD_LIBRARY_PATH:-}"
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
