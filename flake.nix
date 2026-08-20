{
  description = "jaunder - a federated social media application";

  nixConfig = {
    extra-substituters = [ "https://jaunder-org.cachix.org" ];
    extra-trusted-public-keys = [
      "jaunder-org.cachix.org-1:usr4hb9a8+Ykafq+ZmX8ROwK8TXQXFwqGSDRLQysJeo="
    ];
  };

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    flake-utils.url = "github:numtide/flake-utils";
    crane.url = "github:ipetkov/crane";
  };

  outputs =
    {
      self,
      nixpkgs,
      fenix,
      flake-utils,
      crane,
    }:
    let
      interactiveTestingVmSystem = "x86_64-linux";
      postgresTestingVmSystem = "x86_64-linux";
      # Part of the canonical e2e-server env-var set that the VM systemd unit and
      # the host `cargo xtask e2e-local` driver both source (names shared, values
      # per-environment). The full documented list lives in
      # `xtask/src/steps/e2e_local.rs` (module docs). See issue #249.
      captureEnv = {
        # The single capture-dir contract (#227): the server writes mail.jsonl,
        # websub.jsonl, and diag.log into this dir, and the e2e otel-collector writes
        # otel-traces.jsonl (#332). Spliced into the jaunder.service and otel-collector
        # service envs below; the whole dir is tarred out per combo in e2eRunAndCapture.
        JAUNDER_CAPTURE_DIR = "/var/lib/jaunder/capture";
      };

      jaunderModule =
        {
          lib,
          pkgs,
          config,
          ...
        }:
        let
          cfg = config.services.jaunder;
        in
        let
          targetSystem = pkgs.stdenv.hostPlatform.system;
          jaunderBin = self.packages.${targetSystem}.jaunder;
        in
        {
          options.services.jaunder = {
            enable = lib.mkEnableOption "the Jaunder service";

            bind = lib.mkOption {
              type = lib.types.str;
              default = "127.0.0.1:3000";
            };

            db = lib.mkOption {
              type = lib.types.str;
              default = "sqlite:./data/jaunder.db";
              description = "Database URL passed to jaunder via JAUNDER_DB.";
            };

            prod = lib.mkOption {
              type = lib.types.bool;
              default = false;
            };
          };

          config = lib.mkIf cfg.enable {
            users.groups.jaunder = { };

            users.users.jaunder = {
              isNormalUser = true;
              group = "jaunder";
              home = "/var/lib/jaunder";
              createHome = true;
              packages = [ jaunderBin ];
              shell = pkgs.bashInteractive;
            };

            systemd.services.jaunder = {
              description = "Jaunder";
              wantedBy = [ "multi-user.target" ];
              after = [ "network.target" ];
              environment = {
                JAUNDER_BIND = cfg.bind;
                JAUNDER_DB = cfg.db;
              }
              // lib.optionalAttrs cfg.prod {
                JAUNDER_ENV = "prod";
              };
              # No `target/site` symlink: the binary embeds its CSR bundle +
              # public assets (#237), so it serves them with no external files.
              preStart = ''
                ${jaunderBin}/bin/jaunder init --db "$JAUNDER_DB" --skip-if-exists
              '';
              serviceConfig = {
                User = "jaunder";
                Group = "jaunder";
                StateDirectory = "jaunder";
                WorkingDirectory = "%S/jaunder";
                ExecStart = "${jaunderBin}/bin/jaunder serve";
                Restart = "on-failure";
                RestartSec = "2s";
              };
            };
          };
        };

      interactiveTestingVmModule =
        {
          pkgs,
          ...
        }:
        {
          imports = [ self.nixosModules.jaunder ];

          networking.hostName = "jaunder-interactive-testing";
          boot.postBootCommands = ''
            sleep 5
            ${pkgs.systemd}/bin/systemctl --no-pager status jaunder.service || true
            ${pkgs.systemd}/bin/journalctl -u jaunder.service -b --no-pager -n 100 || true
          '';

          virtualisation.vmVariant = {
            virtualisation.graphics = false;
            virtualisation.forwardPorts = [
              {
                from = "host";
                host.port = 2222;
                guest.port = 22;
              }
              {
                from = "host";
                host.port = 3000;
                guest.port = 3000;
              }
            ];
          };

          boot.loader.grub.devices = [ "nodev" ];
          fileSystems."/" = {
            device = "tmpfs";
            fsType = "tmpfs";
          };

          networking.firewall.allowedTCPPorts = [ 3000 ];

          services.jaunder.enable = true;
          services.jaunder.bind = "0.0.0.0:3000";

          systemd.services.jaunder.environment = captureEnv;

          services.getty.autologinUser = "jaunder";
          security.sudo.wheelNeedsPassword = false;

          users.users.jaunder.extraGroups = [ "wheel" ];
          users.users.jaunder.initialPassword = "jaunder";
          users.users.jaunder.packages = [
            pkgs.postgresql_16
            pkgs.sqlite
          ];

          system.stateVersion = "26.05";
        };

      interactiveTestingVmConfiguration = nixpkgs.lib.nixosSystem {
        system = interactiveTestingVmSystem;
        modules = [ interactiveTestingVmModule ];
      };

      postgresTestingVmModule =
        {
          lib,
          pkgs,
          ...
        }:
        {
          networking.hostName = "jaunder-postgres-testing";

          virtualisation.vmVariant = {
            virtualisation.graphics = false;
            virtualisation.forwardPorts = [
              {
                from = "host";
                host.port = 55432;
                guest.port = 5432;
              }
            ];
          };

          boot.loader.grub.devices = [ "nodev" ];
          fileSystems."/" = {
            device = "tmpfs";
            fsType = "tmpfs";
          };

          networking.firewall.allowedTCPPorts = [ 5432 ];

          services.postgresql = {
            enable = true;
            package = pkgs.postgresql_16;
            ensureDatabases = [ "jaunder" ];
            ensureUsers = [
              {
                name = "jaunder";
                ensureDBOwnership = true;
              }
            ];
            authentication = ''
              local all all trust
              host all all 0.0.0.0/0 trust
              host all all ::0/0 trust
            '';
            settings = {
              listen_addresses = lib.mkForce "*";
            };
            initialScript = pkgs.writeText "jaunder-postgres-init.sql" ''
              ALTER ROLE jaunder WITH LOGIN;
            '';
          };

          environment.systemPackages = [
            pkgs.postgresql_16
          ];

          system.stateVersion = "26.05";
        };

      postgresTestingVmConfiguration = nixpkgs.lib.nixosSystem {
        system = postgresTestingVmSystem;
        modules = [ postgresTestingVmModule ];
      };

    in
    {
      nixosModules.jaunder = jaunderModule;
      nixosConfigurations.interactive-testing-vm = interactiveTestingVmConfiguration;
      nixosConfigurations.postgres-testing-vm = postgresTestingVmConfiguration;
    }
    // flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
        # One explicit screenshot font universe for host baseline generation and
        # NixOS-VM comparison. The file embeds the DejaVu store path, so the font
        # derivation stays in both closures without ambient system-font lookup.
        visualFontConfig = pkgs.makeFontsConf {
          fontDirectories = [ pkgs.dejavu_fonts ];
        };
        toolchain = fenix.packages.${system}.fromToolchainFile {
          file = ./rust-toolchain.toml;
          sha256 = "sha256-A1abGIbOtcBSdrUMhDGrER3pRM1hQP4fp9gh3Y4PKc8=";
        };

        craneLib = (crane.mkLib pkgs).overrideToolchain toolchain;

        src = pkgs.lib.cleanSourceWith {
          src = craneLib.path ./.;
          filter =
            path: type:
            # xtask/ is the host-only dev driver (a separate workspace these
            # derivations never build). Excluding it keeps driver edits from
            # busting the app caches AND guarantees a derivation can never run a
            # stale xtask: it is not in the sandbox, so an accidental
            # `cargo xtask` fails loudly rather than running stale. xtask runs
            # only on the host (dev box / CI runner).
            (!pkgs.lib.hasInfix "/xtask/" path)
            && (
              (pkgs.lib.hasSuffix ".sql" path)
              || (pkgs.lib.hasSuffix ".css" path)
              # The CSR SPA shell the server embeds via include_str! (#239). Specific
              # (not a broad .html suffix) to keep stray HTML out of the crane src.
              || (pkgs.lib.hasSuffix "csr/index.html" path)
              || (builtins.match "scripts/.*" path != null)
              || (craneLib.filterCargoSources path type)
            );
        };

        commonArgs = {
          inherit src;
          pname = "jaunder";
          version = "0.1.0";
          strictDeps = true;
          RUST_MIN_STACK = "16777216";
          nativeBuildInputs = [ pkgs.pkg-config ];
          buildInputs = [
            pkgs.openssl
            pkgs.sqlite
          ]
          ++ pkgs.lib.optionals pkgs.stdenv.isDarwin [
            pkgs.darwin.apple_sdk.frameworks.SystemConfiguration
          ];
        };

        cargoArtifacts = craneLib.buildDepsOnly commonArgs;

        # Compile-only and test-only gates do not need full DWARF. Keep these
        # overrides local to gate derivations so production packages and normal
        # human debug builds keep their documented profiles.
        leanDevProfile = {
          CARGO_PROFILE_DEV_DEBUG = "0";
        };
        leanTestProfile = {
          CARGO_PROFILE_TEST_DEBUG = "0";
        };
        leanDevAndTestProfile = leanDevProfile // leanTestProfile;

        cargoArtifactsLeanDev = craneLib.buildDepsOnly (commonArgs // leanDevProfile);

        jaunderBin = craneLib.buildPackage (
          commonArgs
          // {
            inherit cargoArtifacts;
            cargoExtraArgs = "-p jaunder";
            # Embed the CSR bundle + public assets into the binary (#237): the
            # release artifact is self-contained (ADR-0003/0008), serving pkg/*
            # and public/* with no external files. `server/build.rs` stages these
            # into the embed; the env vars are its inputs (the crane `src` filter
            # admits neither the bundle nor public/, so they arrive via env). This
            # is the build-order edge that makes the binary depend on the bundle.
            JAUNDER_CSR_BUNDLE_DIR = "${csrWasmBundle}";
            JAUNDER_PUBLIC_DIR = "${./public}";
            # Tests are covered by the separate `coverage` check (which runs the
            # instrumented nextest suite) and, for doctests — which nextest
            # structurally cannot run — the separate `doctests` check. Disabling
            # here avoids a redundant `cargo test` compile + run during the
            # package build.
            doCheck = false;
          }
        );

        # The out-of-process e2e seed helper (ADR-0046). Built as its own small
        # crane package (no leptos/wasm/web deps; shares cargoArtifacts) and placed
        # ONLY on the e2e VM PATH — deliberately absent from the `jaunder` prod
        # binary and the `services.jaunder` NixOS module, so there is no seed
        # surface anywhere near the release artifact.
        testSupportBin = craneLib.buildPackage (
          commonArgs
          // {
            inherit cargoArtifacts;
            pname = "test-support";
            cargoExtraArgs = "-p test-support";
            doCheck = false;
          }
        );

        # The auxiliary tools workspace is separate from the product workspace
        # (ADR-0141). Keep its source and cargo artifacts separate from
        # `commonArgs`/`cargoArtifacts`: `tools/Cargo.lock` owns these deps, while
        # `xtask/` remains host-only and outside the flake source (ADR-0028).
        toolsSrc = pkgs.lib.cleanSourceWith {
          src = craneLib.path ./tools;
          filter = craneLib.filterCargoSources;
        };
        toolsArgs = {
          src = toolsSrc;
          pname = "jaunder-tools";
          version = "0.1.0";
          strictDeps = true;
        };
        toolsCargoArtifacts = craneLib.buildDepsOnly toolsArgs;

        # The in-sandbox dev tool (tools/ workspace: devtool + its coverage and
        # doctests path-deps). The offline coverage/doctests sandboxes run it
        # from PATH (nativeBuildInputs) instead of an in-sandbox `cargo run`,
        # whose deps would not be vendored.
        devtoolBin = craneLib.buildPackage (
          toolsArgs
          // {
            cargoArtifacts = toolsCargoArtifacts;
            pname = "devtool";
            cargoExtraArgs = "-p devtool";
            doCheck = false;
          }
        );

        cargo-crap = pkgs.callPackage (
          {
            lib,
            fetchCrate,
            fetchFromGitHub,
            rustPlatform,
          }:
          let
            crateSrc = fetchCrate {
              pname = "cargo-crap";
              version = "0.2.2";
              hash = "sha256-cZ30mdHHLXzpvMhkC6XoPMgfqAdsmdqhEfHq8T15Fmw=";
            };
          in
          rustPlatform.buildRustPackage (finalAttrs: {
            pname = "cargo-crap";
            version = "0.2.2";

            src = fetchFromGitHub {
              owner = "minikin";
              repo = "cargo-crap";
              rev = "v${finalAttrs.version}";
              hash = "sha256-yDoHqkMittJEFYxjpEb/C4+0sRg7ZnMpRO7a9aw5NvI=";
            };

            cargoLock.lockFile = "${crateSrc}/Cargo.lock";

            postPatch = ''
              ln -s ${crateSrc}/Cargo.lock Cargo.lock
            '';

            meta = {
              description = "Compute the CRAP (Change Risk Anti-Patterns) metric for Rust projects";
              mainProgram = "cargo-crap";
              homepage = "https://github.com/minikin/cargo-crap";
              changelog = "https://github.com/minikin/cargo-crap/blob/v${finalAttrs.version}/CHANGELOG.md";
              license = lib.licenses.mit;
              maintainers = [ lib.maintainers.mdorman ];
            };
          })
        ) { };

        # `buildRustPackage` requires a flat vendor directory plus Cargo.lock.
        # Crane fetches from static.crates.io reliably but groups packages by
        # registry hash, so adapt that output without re-downloading crates.
        vendorCargoDepsForBuildRustPackage =
          { name, src }:
          let
            vendor = craneLib.vendorCargoDeps { inherit src; };
            cratesIoDir = builtins.hashString "sha256" "registry+https://github.com/rust-lang/crates.io-index";
          in
          pkgs.runCommand "${name}-cargo-deps" { } ''
            mkdir -p $out
            cp -r ${vendor}/${cratesIoDir}/. $out/
            cp ${src}/Cargo.lock $out/Cargo.lock
          '';

        wasm-bindgen-cli = pkgs.wasm-bindgen-cli.overrideAttrs (old: rec {
          version = "0.2.121";
          src = pkgs.fetchCrate {
            pname = "wasm-bindgen-cli";
            inherit version;
            hash = "sha256-ZOMgFNOcGkO66Jz/Z83eoIu+DIzo3Z/vq6Z5g6BDY/w=";
          };
          cargoDeps = vendorCargoDepsForBuildRustPackage {
            name = "wasm-bindgen-cli";
            inherit src;
          };
        });

        wasmTestWebdriverConfig = pkgs.writeText "wasm-bindgen-test-webdriver.json" (
          builtins.toJSON {
            "goog:chromeOptions" = {
              binary = "${pkgs.chromium}/bin/chromium";
              args = [
                "--no-sandbox"
                "--disable-dev-shm-usage"
              ];
            };
          }
        );

        # leptosfmt pinned past its last release (#420): 0.1.33 mangles wrapping
        # generic component tags; the fix is merged upstream but unreleased.
        # REMOVE THIS OVERRIDE once a leptosfmt release later than 0.1.33
        # exists: drop this binding and take `pkgs.leptosfmt` again. The
        # override mechanics (`src` swap, the `cargoDeps` cascade, Crane's
        # static.crates.io vendoring adapter, and why `version` stays "0.1.33")
        # are in docs/adr/0118-leptosfmt-pinned-past-release.md.
        leptosfmt = pkgs.leptosfmt.overrideAttrs (_old: rec {
          src = pkgs.fetchFromGitHub {
            owner = "bram209";
            repo = "leptosfmt";
            rev = "8b4194ba33eee417ababdd15498940014fd6d237";
            # PR #167 bumps a `prettyplease` submodule; replacing `src`
            # wholesale drops nixpkgs' own `fetchSubmodules`, so it is restated.
            fetchSubmodules = true;
            hash = "sha256-F06Ag99rCn3qZywdxyP7ULOgyhbSzWNe+drBDZJWVxo=";
          };
          # Overriding `src` alone is not enough: nixpkgs passes `cargoHash`,
          # which `buildRustPackage` consumes *before* `overrideAttrs` applies,
          # so the 0.1.33 vendor tree would survive a bare `src` swap.
          cargoDeps = vendorCargoDepsForBuildRustPackage {
            name = "leptosfmt";
            inherit src;
          };
        });

        # The CSR client's wasm bundle (`pkg/*`) + public assets, assembled as a
        # tree. The server does not serve this from disk — it embeds the bundle
        # + public assets (#237) and the SPA shell (#239) into the binary. `site`
        # exists because `cargo xtask audit-wasm` builds `.#site` and inspects
        # `$out/pkg/jaunder.{wasm,js}` for bundle-size analysis (ADR-0028).
        site = pkgs.runCommand "jaunder-site" { } ''
          mkdir -p $out/pkg
          cp -r ${csrWasmBundle}/. $out/pkg/
          cp -r ${./public}/. $out/
        '';

        # --- leptos-CSR client (#177/#180) --------------------------------------
        # The client-side-render wasm binary — the only client (#180).
        # `csrWasmBundle` runs wasm-bindgen over it; `site`
        # (above) bundles it with the public assets + the CSR SPA shell.
        csrWasm = craneLib.buildPackage (
          commonArgs
          // {
            cargoArtifacts = craneLib.buildDepsOnly (
              commonArgs
              // {
                CARGO_BUILD_TARGET = "wasm32-unknown-unknown";
                cargoExtraArgs = "-p csr";
                doCheck = false;
              }
            );
            CARGO_BUILD_TARGET = "wasm32-unknown-unknown";
            cargoExtraArgs = "-p csr";
            doCheck = false;
            installPhaseCommand = ''
              mkdir -p $out/lib
              cp target/wasm32-unknown-unknown/release/csr.wasm $out/lib/
            '';
          }
        );

        csrWasmBundle =
          pkgs.runCommand "jaunder-csr-wasm-bundle"
            {
              nativeBuildInputs = [
                devtoolBin
                pkgs.binaryen
                wasm-bindgen-cli
              ];
            }
            ''
              # Post-process the crane-built csr.wasm into the served bundle
              # (pkg/jaunder.{js,wasm}) via the shared `devtool csr-bundle` — the
              # SAME implementation the host build (`cargo xtask build-csr`) runs, so
              # host and Nix cannot drift (#236). devtool shells out to
              # `wasm-bindgen` (on PATH here) and does the rename + js wasm-ref fix.
              devtool csr-bundle --wasm ${csrWasm}/lib/csr.wasm --out $out
            '';

        e2eOtelCollectorConfig = pkgs.writeText "jaunder-otel-collector.yaml" ''
          receivers:
            otlp:
              protocols:
                grpc:
                  endpoint: 127.0.0.1:4317
                http:
                  endpoint: 127.0.0.1:4318
          processors:
            batch: {}
          exporters:
            file:
              # The doubled-apostrophe prefix below is the Nix indented-string escape,
              # so the env-var reference reaches otelcol verbatim and its env provider
              # expands it at runtime, rather than Nix antiquoting it at eval time.
              path: ''${env:JAUNDER_CAPTURE_DIR}/otel-traces.jsonl
          service:
            pipelines:
              traces:
                receivers: [otlp]
                processors: [batch]
                exporters: [file]
        '';

        e2ePackage = pkgs.buildNpmPackage {
          name = "jaunder-e2e";
          src = ./end2end;
          npmDepsHash = "sha256-z4BCkyRqFaBc2YpPjUNpAPer1SVKLvv0XLxVhrzJI90=";
          dontNpmBuild = true;
          installPhase = ''
            mkdir -p $out
            cp -r . $out/
          '';
        };

        emacsSrc = pkgs.lib.cleanSourceWith {
          src = ./elisp;
        };

        # One emacs for both the host verify gate (the xtask StepSpecs) and the
        # hermetic nix checks, so they cannot diverge. withPackages (vs bare
        # pkgs.emacs) is the extension point for units C/D to add elisp packages
        # via nix. `plz` is the AtomPub client's HTTP transport (ADR-0037) — it
        # drives the `curl` binary, so anything running plz also needs `curl` on
        # PATH (the e2e VM and the ci dev shell, below).
        emacsForCi = pkgs.emacs.pkgs.withPackages (epkgs: [ epkgs.plz ]);

        interactiveTestingVmRunner = pkgs.writeShellApplication {
          name = "interactive-testing-vm";
          text = ''
            echo "HTTP: http://localhost:3000"
            exec ${interactiveTestingVmConfiguration.config.system.build.vm}/bin/run-jaunder-interactive-testing-vm "$@"
          '';
        };

        postgresTestingVmRunner = pkgs.writeShellApplication {
          name = "postgres-testing-vm";
          text = ''
            echo "PostgreSQL: postgres://jaunder@127.0.0.1:55432/jaunder"
            exec ${postgresTestingVmConfiguration.config.system.build.vm}/bin/run-jaunder-postgres-testing-vm "$@"
          '';
        };

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
          # copy_from_vm's 2nd arg is a target *directory*; "" lands the file flat at
          # $out/jaunder-journal-${backend}.log (the per-backend name comes from the source).
          machine.copy_from_vm("/tmp/jaunder-journal-${backend}.log", "")
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
            # (e.g. an early crash) never aborts the remaining copies. copy_from_vm's
            # 2nd arg is a target *dir*; "" lands the file flat under the per-backend
            # name carried by the source.
            def _grab(path):
                if machine.execute("test -e " + path)[0] == 0:
                    machine.copy_from_vm(path, "")

            machine.execute("test -s /tmp/e2e/test-results/results.json && cp /tmp/e2e/test-results/results.json /tmp/playwright-report-${backend}.json")
            _grab("/tmp/playwright-report-${backend}.json")

            machine.execute("tar czf /tmp/playwright-artifacts-${backend}.tar.gz -C /tmp/e2e test-results 2>/dev/null || true")
            _grab("/tmp/playwright-artifacts-${backend}.tar.gz")

            machine.execute("journalctl --no-pager -o short-precise > /tmp/system-journal-${backend}.log")
            _grab("/tmp/system-journal-${backend}.log")

            # Capture-dir contract (#227, #332): tar the whole capture dir out per combo as
            # capture-${backend}.tar.gz — a file copy mirroring the playwright-artifacts
            # tarball (the proven copy_from_vm shape). Holds diag.log, the collector's
            # otel-traces.jsonl (#332 — the collector is stopped above, so its file export is
            # flushed), plus any written mail.jsonl/websub.jsonl. The in-VM zero-panic gate
            # reads diag.log directly, so it does not depend on this lift.
            machine.execute("test -d /var/lib/jaunder/capture && tar czf /tmp/capture-${backend}.tar.gz -C /var/lib/jaunder capture 2>/dev/null || true")
            _grab("/tmp/capture-${backend}.tar.gz")

            ${e2ePanicGate backend}

            # Fail the check now — after all artifacts are safely copied out.
            assert pw_status == 0, "e2e Playwright failed (exit %d) for ${backend}/${browser}; see playwright-report-${backend}.json + playwright-artifacts-${backend}.tar.gz + build.log" % pw_status
          '';

        mkE2eSqliteCheck =
          {
            checkName,
            browser,
            traceId,
            traceParent,
            extraEnv ? "",
            vmMemory ? 2048,
            vmCores ? null,
          }:
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
                imports = [ self.nixosModules.jaunder ];

                virtualisation.memorySize = vmMemory;
                # Default (null) leaves the nixosTest core count alone; the #155
                # worker probes set >1 so concurrent workers get real parallelism
                # (a 1-vCPU VM would timeshare them, under-stressing SQLite
                # write contention — the very thing the probe measures).
                virtualisation.cores = lib.mkIf (vmCores != null) vmCores;
                environment.systemPackages = [
                  pkgs.sqlite
                  pkgs.opentelemetry-collector-contrib
                  testSupportBin
                  devtoolBin
                  # `jaunder site-config set` seed steps resolve bare `jaunder` here.
                  jaunderBin
                ];
                environment.etc."jaunder-otel-collector.yaml".source = e2eOtelCollectorConfig;

                systemd.tmpfiles.rules = [ "d /var/lib/jaunder/capture 0755 jaunder jaunder -" ];
                systemd.services.otel-collector = {
                  description = "Jaunder e2e OTel Collector";
                  wantedBy = [ "multi-user.target" ];
                  after = [ "network.target" ];
                  # ${env:JAUNDER_CAPTURE_DIR} in the exporter config expands from this.
                  environment = captureEnv;
                  serviceConfig = {
                    ExecStart = "${pkgs.opentelemetry-collector-contrib}/bin/otelcol-contrib --config /etc/jaunder-otel-collector.yaml";
                    Restart = "on-failure";
                    RestartSec = "2s";
                  };
                };

                services.jaunder.enable = true;
                services.jaunder.bind = "127.0.0.1:3000";
                systemd.services.jaunder.after = [ "otel-collector.service" ];
                systemd.services.jaunder.requires = [ "otel-collector.service" ];
                systemd.services.jaunder.environment = captureEnv // {
                  RUST_LOG = "info";
                  JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT = "http://127.0.0.1:4317";
                };
              };

            testScript = ''
              def seed_db():
                # Seed the fresh VM's already-migrated DB. This VM is single-use and
                # jaunder.service's boot preStart (`jaunder init`) has already created
                # and migrated an empty DB (incl. migration 0018 reference data);
                # nothing writes user data before this point, so no wipe is needed
                # (#271). Seeding runs against the running boot service.
                machine.succeed(
                  "JAUNDER_CAPTURE_DIR=/var/lib/jaunder/capture devtool seed-e2e"
                  + " --db sqlite:/var/lib/jaunder/data/jaunder.db"
                  + " --test-support-bin test-support"
                  + " --jaunder-bin jaunder"
                )

              machine.start()
              machine.wait_for_unit("otel-collector.service", timeout=60)
              machine.wait_for_unit("jaunder.service", timeout=60)
              machine.wait_for_open_port(3000, timeout=30)

              machine.succeed("cp -r ${e2ePackage} /tmp/e2e && chmod -R u+w /tmp/e2e")

              # Seed a fresh DB and run the one browser this derivation targets.
              # Browsers run as separate derivations (one VM each) so their state
              # mutations cannot interfere; that also lets CI fan them out.
              seed_db()
              ${e2eRunAndCapture {
                backend = "sqlite";
                jaunderDb = "sqlite:/var/lib/jaunder/data/jaunder.db";
                inherit
                  browser
                  traceId
                  traceParent
                  extraEnv
                  ;
              }}
            '';
          };

        mkE2ePostgresCheck =
          {
            checkName,
            browser,
            traceId,
            traceParent,
            extraEnv ? "",
            vmMemory ? 2048,
            vmCores ? null,
          }:
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
                imports = [ self.nixosModules.jaunder ];

                virtualisation.memorySize = vmMemory;
                # Default (null) leaves the nixosTest core count alone; the
                # gate sets 2, matching its worker count (workers>1 needs the
                # cores; 1 vCPU timeshares and starves the client render).
                virtualisation.cores = lib.mkIf (vmCores != null) vmCores;
                environment.systemPackages = [
                  pkgs.postgresql_16
                  pkgs.opentelemetry-collector-contrib
                  testSupportBin
                  devtoolBin
                  # `jaunder site-config set` seed steps resolve bare `jaunder` here.
                  jaunderBin
                ];
                environment.etc."jaunder-otel-collector.yaml".source = e2eOtelCollectorConfig;

                systemd.tmpfiles.rules = [ "d /var/lib/jaunder/capture 0755 jaunder jaunder -" ];
                systemd.services.otel-collector = {
                  description = "Jaunder e2e OTel Collector";
                  wantedBy = [ "multi-user.target" ];
                  after = [ "network.target" ];
                  # ${env:JAUNDER_CAPTURE_DIR} in the exporter config expands from this.
                  environment = captureEnv;
                  serviceConfig = {
                    ExecStart = "${pkgs.opentelemetry-collector-contrib}/bin/otelcol-contrib --config /etc/jaunder-otel-collector.yaml";
                    Restart = "on-failure";
                    RestartSec = "2s";
                  };
                };

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

                services.jaunder.enable = true;
                services.jaunder.db = "postgres://jaunder:testpassword@127.0.0.1/jaunder";
                # We delay jaunder.service until we have run create-pg-db in the testScript.
                systemd.services.jaunder.wantedBy = lib.mkForce [ ];
                systemd.services.jaunder.after = [ "otel-collector.service" ];
                systemd.services.jaunder.requires = [ "otel-collector.service" ];
                systemd.services.jaunder.environment = captureEnv // {
                  RUST_LOG = "info";
                  JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT = "http://127.0.0.1:4317";
                };
              };

            testScript = ''
              machine.start()
              machine.wait_for_unit("otel-collector.service", timeout=60)
              machine.wait_for_unit("postgresql.service", timeout=60)

              machine.succeed(
                "${jaunderBin}/bin/jaunder create-pg-db"
                + " --bootstrap-db postgres://postgres@127.0.0.1/postgres"
                + " --app-db postgres://jaunder@127.0.0.1/jaunder"
                + " --app-role-password testpassword"
              )

              machine.succeed("systemctl start jaunder.service")
              machine.wait_for_unit("jaunder.service", timeout=60)
              machine.wait_for_open_port(3000, timeout=30)

              machine.succeed("cp -r ${e2ePackage} /tmp/e2e && chmod -R u+w /tmp/e2e")

              def seed_db():
                # Seed the fresh VM's already-migrated DB. This VM is single-use;
                # create-pg-db + the delayed jaunder.service boot preStart
                # (`jaunder init`) have already created and migrated an empty DB
                # (incl. migration 0018 reference data), and nothing writes user data
                # before this point, so no TRUNCATE is needed (#271).
                machine.succeed(
                  "JAUNDER_CAPTURE_DIR=/var/lib/jaunder/capture devtool seed-e2e"
                  + " --db postgres://jaunder:testpassword@127.0.0.1/jaunder"
                  + " --test-support-bin test-support"
                  + " --jaunder-bin jaunder"
                )

              # Seed a fresh DB and run the one browser this derivation targets.
              # Browsers run as separate derivations (one VM each) so their state
              # mutations cannot interfere; that also lets CI fan them out.
              seed_db()
              ${e2eRunAndCapture {
                backend = "postgres";
                jaunderDb = "postgres://jaunder:testpassword@127.0.0.1/jaunder";
                inherit
                  browser
                  traceId
                  traceParent
                  extraEnv
                  ;
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
            mk = if backend == "sqlite" then mkE2eSqliteCheck else mkE2ePostgresCheck;
            traceId = pkgs.lib.concatStrings (pkgs.lib.genList (_: traceDigit) 32);
            traceParent = "00-${traceId}-${pkgs.lib.concatStrings (pkgs.lib.genList (_: traceDigit) 16)}-01";
          in
          mk {
            checkName = "jaunder-e2e-${backend}-${browser}${nameSuffix}";
            # The salt rides the combo's generic extra-env string, which is
            # interpolated into the VM testScript above — so it reaches the
            # derivation hash. The variable itself is inert: nothing reads
            # JAUNDER_E2E_SALT. Changing the hash is its whole job. Spliced here
            # rather than per-family so every combo salts alike.
            extraEnv = extraEnv + pkgs.lib.optionalString (e2eSalt != "") " JAUNDER_E2E_SALT=${e2eSalt}";
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
        # OOM that heavier VMs hit (#61). At workers=2 the per-VM footprint is
        # small enough that a 16-core dev box (and CI's per-combo runners) run the
        # combos comfortably; see docs/observability.md #155 AC3/AC4.
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
            jaunder = jaunderBin;
            site = site;
            # The pre-wasm-bindgen, unstripped wasm. Exposed so
            # `cargo xtask audit-wasm --breakdown` has an artifact that still
            # carries a name section: `wasm-opt` strips names from the shipped
            # bundle, so the shipped file cannot be attributed to crates (#836).
            inherit csrWasm;
            devtool = devtoolBin;
            # The out-of-process e2e seed helper (ADR-0046). Exposed so it is
            # directly buildable/verifiable; it is placed only on the e2e VM PATH,
            # never in the prod artifact or the NixOS module.
            test-support = testSupportBin;

            # The e2e aggregate: a symlinkJoin of every `e2e-*` check, exposed as
            # `checks.e2e` and built by `cargo xtask validate`. Adding a new e2e
            # combo automatically joins it here. Its `jaunder-e2e*` name keeps it
            # out of the cachix push, so building it always realizes the
            # underlying VM checks rather than substituting a cached aggregate.
            e2e-checks = pkgs.symlinkJoin {
              name = "jaunder-e2e-checks";
              paths = builtins.attrValues (
                pkgs.lib.filterAttrs (name: _: pkgs.lib.hasPrefix "e2e-" name) self.checks.${system}
              );
            };
          }
          // e2eSingleWorkerPackages
        );

        apps =
          pkgs.lib.optionalAttrs
            (pkgs.stdenv.isLinux && pkgs.stdenv.hostPlatform.system == interactiveTestingVmSystem)
            {
              interactive-testing-vm = {
                type = "app";
                program = "${interactiveTestingVmRunner}/bin/interactive-testing-vm";
              };
              postgres-testing-vm = {
                type = "app";
                program = "${postgresTestingVmRunner}/bin/postgres-testing-vm";
              };
            };

        checks =
          pkgs.lib.optionalAttrs pkgs.stdenv.isLinux (
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

              # The single e2e gate `cargo xtask validate` builds. `e2e-checks`
              # aggregates every `checks.e2e-*` combo (now 4); they are independent
              # derivations realized in parallel up to the host `max-jobs` (CI's
              # install-nix-action sets `max-jobs = auto`; a plain dev box defaults
              # to 1 and runs them serially). The aggregate's name stays under
              # `jaunder-e2e*`, so the cachix pushFilter still excludes it — the VM
              # runs are never substituted from a cached aggregate.
              e2e = self.packages.${system}.e2e-checks;

              # Live elisp integration suite (ADR-0035): a minimal NixOS VM with
              # Emacs + the jaunder binary. The harness self-boots the server
              # (no systemd service, no Playwright), so the VM only supplies the
              # toolchain. The `e2e-` attr prefix folds it into the `e2e-checks`
              # aggregate (realized in parallel with the combos by local
              # `validate`); the `jaunder-e2e*` derivation name keeps it out of the
              # cachix push, so the VM test always re-runs (never a cached green).
              e2e-elisp-integration = pkgs.testers.nixosTest {
                name = "jaunder-e2e-elisp-integration";
                nodes.machine = _: {
                  # #628: headroom so the server boots fast enough for the one
                  # remaining readiness gate under CI load.
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
                  machine.succeed(
                      "JAUNDER_TEST_BINARY=${jaunderBin}/bin/jaunder "
                      + "emacs --batch -Q -l ${emacsSrc}/scripts/run-integration-tests.el"
                  )
                '';
              };
            }
          )
          // {
            clippy = craneLib.cargoClippy (
              commonArgs
              // {
                cargoArtifacts = cargoArtifactsLeanDev;
                # Crane defaults Clippy to release mode, but `--all-targets`
                # activates the test-only `cheap-kdf` feature whose optimized-build
                # guard must fail. Lint test targets in the development profile;
                # production package builds remain release-mode. Clippy diagnostics
                # do not need DWARF, so this derivation uses the lean dev profile.
                CARGO_PROFILE = "dev";
                CARGO_PROFILE_DEV_DEBUG = "0";
                cargoClippyExtraArgs = "--all-targets -- -D warnings";
              }
            );
            # wasm-clippy — `web::pages` compiles wasm-only (#300), so the host `clippy`
            # above never sees it; likewise the wasm-only `client` and `csr` entry crates
            # (#519). Lint them on the wasm target (mirrors the host xtask `wasm-clippy`
            # step).
            wasm-clippy = craneLib.cargoClippy (
              commonArgs
              // {
                cargoArtifacts = craneLib.buildDepsOnly (
                  commonArgs
                  // leanDevProfile
                  // {
                    CARGO_BUILD_TARGET = "wasm32-unknown-unknown";
                    cargoExtraArgs = "-p web -p client -p csr --features csr";
                    doCheck = false;
                  }
                );
                CARGO_BUILD_TARGET = "wasm32-unknown-unknown";
                CARGO_PROFILE_DEV_DEBUG = "0";
                cargoClippyExtraArgs = "-p web -p client -p csr --features csr -- -D warnings";
              }
            );
            # The non-compiling static checks (#188), unified behind one `devtool
            # check --all` — the same command the host verify ladder runs. Not a crane
            # derivation: none of these compiles, so no vendored deps are needed; a plain
            # runCommand over a broad source tree suffices (and stays cheap). The
            # compiling checks `clippy`/`deny` keep their crane derivations above/below.
            static-checks =
              let
                staticCheckSrc = pkgs.lib.cleanSourceWith {
                  src = craneLib.path ./.;
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
                    devtoolBin
                    toolchain
                    leptosfmt
                    pkgs.prettier
                    pkgs.nodejs
                    pkgs.typescript
                    emacsForCi
                  ];
                  # ert needs a zone DB (#160); tsc needs BOTH node-dep envs
                  # (`devtool provision-node-modules`'s resolver errors on each when
                  # unset).
                  TZDIR = "${pkgs.tzdata}/share/zoneinfo";
                  E2E_TYPES_NODE_MODULES = "${e2ePackage}/node_modules";
                  E2E_PLAYWRIGHT_TEST = "${pkgs.playwright-test}/lib/node_modules/@playwright/test";
                }
                ''
                  # Writable copy: `devtool check tsc` provisions end2end/node_modules
                  # in-process (#229).
                  cp --no-preserve=mode -r ${staticCheckSrc} src
                  cd src
                  devtool check --all
                  touch $out
                '';
            deny = craneLib.cargoDeny {
              inherit src;
              pname = "jaunder";
              version = "0.1.0";
            };
            coverage = craneLib.mkCargoDerivation (
              commonArgs
              // {
                src = pkgs.lib.cleanSourceWith {
                  src = craneLib.path ./.;
                  filter =
                    path: type:
                    # Coverage-specific exclusions: none of these are
                    # instrumented, and admitting them would let unrelated edits
                    # bust the coverage cache. xtask/ is the host-only driver;
                    # tools/, docs/, .github/, elisp/, and top-level *.md are
                    # non-source.
                    !(pkgs.lib.hasInfix "/xtask/" path)
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
                JAUNDER_PUBLIC_DIR = "${./public}";
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
                    jq -r '.infra_detail // (.violations[] | "\(.file):\(.line) [\(.kind)] \(.detail)")' \
                      ${self.checks.${system}.doctests}/status.json >&2
                    exit 1
                  fi
                  touch $out
                '';
          };

        devShells =
          let
            # Everything `cargo xtask validate` needs on the host (toolchain + the
            # static-check tools) plus what the Nix checks pull anyway — so the CI
            # shell shares those store paths rather than adding cost.
            ciInputs = [
              toolchain
              pkgs.cachix
              cargo-crap
              pkgs.cargo-deny
              pkgs.cargo-llvm-cov
              pkgs.cargo-nextest
              pkgs.curl
              # `devtool run -- <cmd>` for humans/agents, and the `shellHook`'s
              # `devtool provision-node-modules` (#229) — so it must be on PATH in the
              # CI shell too, not just the interactive one. Already built for the
              # coverage and static-checks derivations, so this adds no new build.
              devtoolBin
              emacsForCi
              pkgs.jq
              leptosfmt
              pkgs.nodejs
              pkgs.openssl
              pkgs.pkg-config
              pkgs.playwright-test
              pkgs.postgresql_16
              pkgs.prettier
              pkgs.sqlite
              pkgs.typescript
              # Host xtask steps opt Rust-compiling cargo invocations into
              # `RUSTC_WRAPPER=sccache`; xtask maintains the multi-checkout
              # `SCCACHE_BASEDIRS` registry at runtime.
              pkgs.sccache
              # `wasm-opt`, run by `devtool csr-bundle` after `wasm-bindgen` (#836).
              # In `ciInputs` rather than `devOnly` because `cargo xtask build-csr`
              # invokes it on the host, so the CI shell needs it too.
              pkgs.binaryen
              wasm-bindgen-cli
            ]
            ++ pkgs.lib.optionals pkgs.stdenv.isDarwin [
              pkgs.darwin.apple_sdk.frameworks.SystemConfiguration
            ];
            # Interactive-only tools that `cargo xtask validate` never invokes and no
            # Nix check pulls (the language servers are the bulk). Kept out of
            # `devShells.ci` so CI does not download/build them.
            devOnly = [
              pkgs.typescript-language-server
              pkgs.vscode-langservers-extracted
              pkgs.cargo-generate
              pkgs.cargo-mutants
              pkgs.sqlx-cli
              # `cargo xtask pr watch`/`pr land` shell out to `gh` (#729). Host-only
              # manual commands — no Nix check or CI job runs them — so this stays
              # out of `ciInputs`, like the other interactive tooling here.
              pkgs.gh
            ];
            shellEnv = {
              RUST_SRC_PATH = "${toolchain}/lib/rustlib/src/rust/library";
              PLAYWRIGHT_BROWSERS_PATH = "${pkgs.playwright-driver.browsers}";
              PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD = "1";
              FONTCONFIG_FILE = "${visualFontConfig}";
              # The host `ert` step (run via `nix develop .#ci -c cargo xtask …`)
              # computes timezone->UTC from IANA zone names, which need a zone
              # database for `encode-time` to resolve. A clean CI runner has none
              # in this shell, so provide it deterministically rather than relying
              # on the host system's own TZDIR (which masked this locally). Mirrors
              # the ert-check derivation's TZDIR (#160).
              TZDIR = "${pkgs.tzdata}/share/zoneinfo";
              # Store paths for `devtool provision-node-modules`. Exported as env
              # vars (rather than baked into the shellHook) so they survive `cd`
              # into a worktree — that is what lets `devtool check tsc` re-run the
              # provisioning there, where the shellHook never fired.
              E2E_TYPES_NODE_MODULES = "${e2ePackage}/node_modules";
              E2E_PLAYWRIGHT_TEST = "${pkgs.playwright-test}/lib/node_modules/@playwright/test";
              shellHook = ''
                export LD_LIBRARY_PATH="${pkgs.lib.makeLibraryPath [ pkgs.openssl ]}:$LD_LIBRARY_PATH"

                # Provision end2end/node_modules (the tsc type-dep closure) so the
                # devShell `tsc` and IDEs can type-check end2end/ offline in this
                # checkout. The same subcommand runs in-process from `devtool check
                # tsc`, so worktrees self-heal there; see
                # tools/devtool/src/provision.rs for the full rationale.
                devtool provision-node-modules
              '';
            };
          in
          {
            # Lean shell used by CI (`nix develop .#ci -c cargo xtask validate`).
            ci = pkgs.mkShell (shellEnv // { buildInputs = ciInputs; });
            # The scheduled mutation-testing job, and nothing else. `cargo-mutants`
            # stays out of `ciInputs` on purpose: every PR job enters `.#ci`, and
            # none of them run mutants. This shell is `ciInputs` plus that one
            # tool, so the weekly job gets it without the pull-request path paying
            # for it. See .github/workflows/mutants.yml.
            mutants = pkgs.mkShell (shellEnv // { buildInputs = ciInputs ++ [ pkgs.cargo-mutants ]; });
            # Full interactive shell for local development.
            default = pkgs.mkShell (shellEnv // { buildInputs = ciInputs ++ devOnly; });
          };
      }
    );
}
