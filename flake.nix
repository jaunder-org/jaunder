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
    serena = {
      inputs.nixpkgs.follows = "nixpkgs";
      url = "github:oraios/serena";
    };
    crane.url = "github:ipetkov/crane";
  };

  outputs =
    {
      self,
      nixpkgs,
      fenix,
      flake-utils,
      serena,
      crane,
    }:
    let
      interactiveTestingVmSystem = "x86_64-linux";
      postgresTestingVmSystem = "x86_64-linux";
      mailCaptureEnv = {
        JAUNDER_MAIL_CAPTURE_FILE = "/var/lib/jaunder/mail.jsonl";
        JAUNDER_WEBSUB_CAPTURE_FILE = "/var/lib/jaunder/websub.jsonl";
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
          site = self.packages.${targetSystem}.site;
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
              preStart = ''
                mkdir -p target
                ln -sfn ${site} target/site
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

          systemd.services.jaunder.environment = mailCaptureEnv;

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
        toolchain = fenix.packages.${system}.fromToolchainFile {
          file = ./rust-toolchain.toml;
          sha256 = "sha256-gh/xTkxKHL4eiRXzWv8KP7vfjSk61Iq48x47BEDFgfk=";
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

        jaunderBin = craneLib.buildPackage (
          commonArgs
          // {
            inherit cargoArtifacts;
            cargoExtraArgs = "-p jaunder";
            # Tests are covered by the separate `nextest` check; disabling here
            # avoids a redundant `cargo test` compile + run during the package
            # build.
            doCheck = false;
          }
        );

        hydrateWasm = craneLib.buildPackage (
          commonArgs
          // {
            cargoArtifacts = craneLib.buildDepsOnly (
              commonArgs
              // {
                CARGO_BUILD_TARGET = "wasm32-unknown-unknown";
                cargoExtraArgs = "-p hydrate";
                doCheck = false;
              }
            );
            CARGO_BUILD_TARGET = "wasm32-unknown-unknown";
            cargoExtraArgs = "-p hydrate";
            doCheck = false;
            installPhaseCommand = ''
              mkdir -p $out/lib
              cp target/wasm32-unknown-unknown/release/hydrate.wasm $out/lib/
            '';
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

        wasm-bindgen-cli = pkgs.wasm-bindgen-cli.overrideAttrs (old: rec {
          version = "0.2.121";
          src = pkgs.fetchCrate {
            pname = "wasm-bindgen-cli";
            inherit version;
            hash = "sha256-ZOMgFNOcGkO66Jz/Z83eoIu+DIzo3Z/vq6Z5g6BDY/w=";
          };
          cargoDeps = pkgs.rustPlatform.fetchCargoVendor {
            inherit src;
            hash = "sha256-DPdCDPTAPBrbqLUqnCwQu1dePs9lGg85JCJOCIr9qjU=";
          };
        });

        wasmBundle =
          pkgs.runCommand "jaunder-wasm-bundle"
            {
              nativeBuildInputs = [
                wasm-bindgen-cli
                pkgs.gnused
              ];
            }
            ''
              mkdir -p $out
              wasm-bindgen \
                --target web \
                --out-dir $out \
                ${hydrateWasm}/lib/hydrate.wasm
              # Rename to match output-name = "jaunder" expected by the Leptos SSR HTML
              mv $out/hydrate.js $out/jaunder.js
              mv $out/hydrate_bg.wasm $out/jaunder_bg.wasm
              sed -i 's/hydrate_bg\.wasm/jaunder_bg.wasm/g' $out/jaunder.js
            '';

        site = pkgs.runCommand "jaunder-site" { } ''
          mkdir -p $out/pkg
          cp -r ${wasmBundle}/. $out/pkg/
          cp -r ${./public}/. $out/
        '';

        # Playwright config for the Nix VM environment.
        # Chromium needs --no-sandbox (runs as root) and GPU/shm disabled.
        # WebKit (WPE) is excluded: WPEWebProcess crashes with SIGABRT in
        # the NixOS VM, filling the disk with coredumps.
        nixPlaywrightConfig = pkgs.writeText "playwright.nix.config.js" ''
          const { defineConfig, devices } = require('@playwright/test');
          const traceParent = process.env.JAUNDER_E2E_TRACEPARENT;
          module.exports = defineConfig({
            testDir: './tests',
            timeout: 30 * 1000,
            expect: { timeout: 5000 },
            reporter: 'line',
            use: {
              actionTimeout: 0,
              ...(traceParent ? { extraHTTPHeaders: { traceparent: traceParent } } : {}),
            },
            // Run spec files sequentially to avoid SQLite write contention.
            // Each browser project is already run in isolation (separate seed_db()
            // calls), so one worker is sufficient and prevents locking errors.
            workers: 1,
            projects: [
              {
                name: 'chromium',
                use: {
                  ...devices['Desktop Chrome'],
                  launchOptions: {
                    args: [
                      '--no-sandbox',
                      '--disable-gpu',
                      '--disable-dev-shm-usage',
                    ],
                  },
                },
              },
              {
                name: 'firefox',
                use: {
                  ...devices['Desktop Firefox'],
                },
              },
            ],
          });
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
              path: /var/lib/jaunder/otel-traces.jsonl
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
          npmDepsHash = "sha256-k+N5Zf+jX2wT9Q2N1yaPYngjV0qTBFWNRdZMjqeE+t0=";
          dontNpmBuild = true;
          installPhase = ''
            mkdir -p $out
            cp -r . $out/
          '';
        };

        end2endSrc = pkgs.lib.cleanSourceWith {
          src = ./end2end;
          filter = path: _type: !(pkgs.lib.hasInfix "/node_modules" path);
        };

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

        mkE2eSqliteCheck =
          {
            checkName,
            warmupEnv ? "",
          }:
          pkgs.testers.nixosTest {
            name = checkName;

            nodes.machine =
              { pkgs, ... }:
              {
                imports = [ self.nixosModules.jaunder ];

                virtualisation.memorySize = 2048;
                environment.systemPackages = [
                  pkgs.sqlite
                  pkgs.opentelemetry-collector-contrib
                ];
                environment.etc."jaunder-otel-collector.yaml".source = e2eOtelCollectorConfig;

                systemd.services.otel-collector = {
                  description = "Jaunder e2e OTel Collector";
                  wantedBy = [ "multi-user.target" ];
                  after = [ "network.target" ];
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
                systemd.services.jaunder.environment = mailCaptureEnv // {
                  RUST_LOG = "info";
                  JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT = "http://127.0.0.1:4317";
                };
              };

            testScript = ''
              def seed_db():
                # Wipe the SQLite data dir wholesale and let jaunder's auto-init
                # recreate the schema. Avoids hardcoding a table list (which
                # would silently drift as the schema grows); mirrors the local
                # `scripts/e2e-local.sh` flow where each run gets a fresh
                # temp storage dir.
                machine.succeed("systemctl stop jaunder.service")
                machine.succeed("rm -rf /var/lib/jaunder/data")
                machine.succeed("systemctl start jaunder.service")
                machine.wait_for_unit("jaunder.service", timeout=60)
                machine.wait_for_open_port(3000, timeout=30)
                machine.succeed("sqlite3 /var/lib/jaunder/data/jaunder.db \"INSERT OR REPLACE INTO site_config (key, value) VALUES ('site.registration_policy', 'open')\"")
                machine.succeed("sqlite3 /var/lib/jaunder/data/jaunder.db \"INSERT OR REPLACE INTO site_config (key, value) VALUES ('feeds.websub_hub_url', 'https://hub.test.local/')\"")
                machine.succeed(
                  "cd /var/lib/jaunder"
                  + " && JAUNDER_BIN=${jaunderBin}/bin/jaunder"
                  + " JAUNDER_MAIL_CAPTURE_FILE=/var/lib/jaunder/mail.jsonl"
                  + " ${./scripts/seed-e2e-fixtures.sh}"
                )

              machine.start()
              machine.wait_for_unit("otel-collector.service", timeout=60)
              machine.wait_for_unit("jaunder.service", timeout=60)
              machine.wait_for_open_port(3000, timeout=30)

              machine.succeed("cp -r ${e2ePackage} /tmp/e2e && chmod -R u+w /tmp/e2e")
              machine.succeed("cp ${nixPlaywrightConfig} /tmp/e2e/playwright.nix.config.js")

              # Run Chromium and Firefox against separate fresh databases so that
              # state mutations in one browser's tests (e.g. password resets) do
              # not interfere with the other browser's tests.
              seed_db()
              machine.succeed(
                "cd /tmp/e2e"
                + " && PLAYWRIGHT_BROWSERS_PATH=${pkgs.playwright-driver.browsers}"
                + " PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD=1"
                + "${warmupEnv}"
                + " JAUNDER_MAIL_CAPTURE_FILE=/var/lib/jaunder/mail.jsonl"
                + " JAUNDER_WEBSUB_CAPTURE_FILE=/var/lib/jaunder/websub.jsonl"
                + " JAUNDER_E2E_TRACE_ID=11111111111111111111111111111111"
                + " JAUNDER_E2E_TRACEPARENT=00-11111111111111111111111111111111-1111111111111111-01"
                + " JAUNDER_E2E_OTLP_HTTP_ENDPOINT=http://127.0.0.1:4318/v1/traces"
                + " ${pkgs.nodejs}/bin/node node_modules/.bin/playwright test"
                + " --config playwright.nix.config.js --project chromium"
              )

              seed_db()
              machine.succeed(
                "cd /tmp/e2e"
                + " && PLAYWRIGHT_BROWSERS_PATH=${pkgs.playwright-driver.browsers}"
                + " PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD=1"
                + "${warmupEnv}"
                + " JAUNDER_MAIL_CAPTURE_FILE=/var/lib/jaunder/mail.jsonl"
                + " JAUNDER_WEBSUB_CAPTURE_FILE=/var/lib/jaunder/websub.jsonl"
                + " JAUNDER_E2E_TRACE_ID=22222222222222222222222222222222"
                + " JAUNDER_E2E_TRACEPARENT=00-22222222222222222222222222222222-2222222222222222-01"
                + " JAUNDER_E2E_OTLP_HTTP_ENDPOINT=http://127.0.0.1:4318/v1/traces"
                + " ${pkgs.nodejs}/bin/node node_modules/.bin/playwright test"
                + " --config playwright.nix.config.js --project firefox"
              )

              machine.succeed("systemctl stop otel-collector.service")
              machine.succeed("test -s /var/lib/jaunder/otel-traces.jsonl")
              machine.copy_from_vm("/var/lib/jaunder/otel-traces.jsonl", "otel-traces-sqlite.jsonl")
            '';
          };

        mkE2ePostgresCheck =
          {
            checkName,
            warmupEnv ? "",
          }:
          pkgs.testers.nixosTest {
            name = checkName;

            nodes.machine =
              { pkgs, lib, ... }:
              {
                imports = [ self.nixosModules.jaunder ];

                virtualisation.memorySize = 2048;
                environment.systemPackages = [
                  pkgs.postgresql_16
                  pkgs.opentelemetry-collector-contrib
                ];
                environment.etc."jaunder-otel-collector.yaml".source = e2eOtelCollectorConfig;

                systemd.services.otel-collector = {
                  description = "Jaunder e2e OTel Collector";
                  wantedBy = [ "multi-user.target" ];
                  after = [ "network.target" ];
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
                systemd.services.jaunder.environment = mailCaptureEnv // {
                  RUST_LOG = "info";
                  JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT = "http://127.0.0.1:4317";
                };
              };

            testScript = ''
              machine.start()
              machine.wait_for_unit("otel-collector.service", timeout=60)
              machine.wait_for_unit("postgresql.service", timeout=60)

              # Exercise create-pg-db
              machine.succeed(
                "${jaunderBin}/bin/jaunder create-pg-db"
                + " --bootstrap-db postgres://postgres@127.0.0.1/postgres"
                + " --app-db postgres://jaunder@127.0.0.1/jaunder"
                + " --app-role-password testpassword"
              )

              # Now start and wait for jaunder.service
              machine.succeed("systemctl start jaunder.service")
              machine.wait_for_unit("jaunder.service", timeout=60)
              machine.wait_for_open_port(3000, timeout=30)

              machine.succeed("cp -r ${e2ePackage} /tmp/e2e && chmod -R u+w /tmp/e2e")
              machine.succeed("cp ${nixPlaywrightConfig} /tmp/e2e/playwright.nix.config.js")

              def seed_db():
                # Dynamic TRUNCATE of every public-schema table avoids
                # hardcoding a list that would drift as the schema grows.
                # Postgres can't be stop-wiped the way SQLite can (it's
                # a separate service), so a wipe-via-TRUNCATE is the
                # cheapest equivalent.
                machine.succeed(
                  "sudo -u postgres psql -d jaunder -c \"DO \\$\\$ DECLARE r record;"
                  + " BEGIN FOR r IN SELECT tablename FROM pg_tables"
                  + " WHERE schemaname = 'public' AND tablename NOT LIKE '\\\\_sqlx%' LOOP"
                  + " EXECUTE 'TRUNCATE TABLE ' || quote_ident(r.tablename) || ' CASCADE';"
                  + " END LOOP; END \\$\\$;\""
                )
                machine.succeed("sudo -u postgres psql -d jaunder -c \"INSERT INTO site_config (key, value) VALUES ('site.registration_policy', 'open')\"")
                machine.succeed("sudo -u postgres psql -d jaunder -c \"INSERT INTO site_config (key, value) VALUES ('feeds.websub_hub_url', 'https://hub.test.local/')\"")
                machine.succeed(
                  "cd /var/lib/jaunder"
                  + " && JAUNDER_BIN=${jaunderBin}/bin/jaunder"
                  + " JAUNDER_DB=postgres://jaunder:testpassword@127.0.0.1/jaunder"
                  + " JAUNDER_MAIL_CAPTURE_FILE=/var/lib/jaunder/mail.jsonl"
                  + " ${./scripts/seed-e2e-fixtures.sh}"
                )

              # Run Chromium and Firefox against separate fresh databases so that
              # state mutations in one browser's tests (e.g. password resets) do
              # not interfere with the other browser's tests.
              seed_db()
              machine.succeed(
                "cd /tmp/e2e"
                + " && PLAYWRIGHT_BROWSERS_PATH=${pkgs.playwright-driver.browsers}"
                + " PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD=1"
                + "${warmupEnv}"
                + " JAUNDER_MAIL_CAPTURE_FILE=/var/lib/jaunder/mail.jsonl"
                + " JAUNDER_WEBSUB_CAPTURE_FILE=/var/lib/jaunder/websub.jsonl"
                + " JAUNDER_E2E_TRACE_ID=33333333333333333333333333333333"
                + " JAUNDER_E2E_TRACEPARENT=00-33333333333333333333333333333333-3333333333333333-01"
                + " JAUNDER_E2E_OTLP_HTTP_ENDPOINT=http://127.0.0.1:4318/v1/traces"
                + " ${pkgs.nodejs}/bin/node node_modules/.bin/playwright test"
                + " --config playwright.nix.config.js --project chromium"
              )

              seed_db()
              machine.succeed(
                "cd /tmp/e2e"
                + " && PLAYWRIGHT_BROWSERS_PATH=${pkgs.playwright-driver.browsers}"
                + " PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD=1"
                + "${warmupEnv}"
                + " JAUNDER_MAIL_CAPTURE_FILE=/var/lib/jaunder/mail.jsonl"
                + " JAUNDER_WEBSUB_CAPTURE_FILE=/var/lib/jaunder/websub.jsonl"
                + " JAUNDER_E2E_TRACE_ID=44444444444444444444444444444444"
                + " JAUNDER_E2E_TRACEPARENT=00-44444444444444444444444444444444-4444444444444444-01"
                + " JAUNDER_E2E_OTLP_HTTP_ENDPOINT=http://127.0.0.1:4318/v1/traces"
                + " ${pkgs.nodejs}/bin/node node_modules/.bin/playwright test"
                + " --config playwright.nix.config.js --project firefox"
              )

              machine.succeed("systemctl stop otel-collector.service")
              machine.succeed("test -s /var/lib/jaunder/otel-traces.jsonl")
              machine.copy_from_vm("/var/lib/jaunder/otel-traces.jsonl", "otel-traces-postgres.jsonl")
            '';
          };

      in
      {
        packages = pkgs.lib.optionalAttrs pkgs.stdenv.isLinux {
          jaunder = jaunderBin;
          site = site;

          e2e-sqlite-cold = mkE2eSqliteCheck {
            checkName = "jaunder-e2e-sqlite-cold";
          };

          e2e-postgres-cold = mkE2ePostgresCheck {
            checkName = "jaunder-e2e-postgres-cold";
          };

          # Regenerates the coverage + CRAP baselines from the reproducible,
          # no-network Nix sandbox (the CI environment) and exposes the two
          # manifests as build outputs to copy back into the repo. This is the
          # only correct way to re-baseline: a host `scripts/check-coverage
          # --update` bakes in higher numbers for network-sensitive files
          # (e.g. server/src/websub/http.rs, server/src/commands.rs) that the
          # sandboxed CI run cannot reproduce, which then fails the gate. Usage:
          #   nix build .#coverage-update
          #   cp result/.coverage-manifest.json result/.crap-manifest.json .
          # Mirrors checks.coverage (keep the build inputs in sync).
          coverage-update = craneLib.mkCargoDerivation (
            commonArgs
            // {
              src = pkgs.lib.cleanSourceWith {
                src = ./.;
                filter =
                  path: _type:
                  !(pkgs.lib.hasInfix "/xtask/" path)
                  && !(pkgs.lib.hasInfix "/docs/" path)
                  && !(pkgs.lib.hasInfix "/.github/" path);
              };
              inherit cargoArtifacts;
              pname = "jaunder-coverage-update";
              CARGO_PROFILE_DEV_DEBUG = "0";
              CARGO_PROFILE_TEST_DEBUG = "0";
              nativeBuildInputs = commonArgs.nativeBuildInputs ++ [
                cargo-crap
                pkgs.cargo-llvm-cov
                pkgs.cargo-nextest
                pkgs.jq
                pkgs.gawk
                pkgs.postgresql_16
              ];
              buildPhaseCargoCommand = ''
                export LD_LIBRARY_PATH="${pkgs.lib.makeLibraryPath [ pkgs.openssl ]}:''${LD_LIBRARY_PATH:-}"
                bash ./scripts/check-coverage --update
              '';
              installPhaseCommand = ''
                mkdir -p $out
                cp .coverage-manifest.json crap-manifest.json $out/
              '';
            }
          );

          # Sub-group meta-packages: CI jobs build these so that adding a
          # new e2e or postgres check only requires touching flake.nix.
          e2e-checks = pkgs.symlinkJoin {
            name = "jaunder-e2e-checks";
            paths = builtins.attrValues (
              pkgs.lib.filterAttrs (name: _: pkgs.lib.hasPrefix "e2e-" name) self.checks.${system}
            );
          };

          # Meta-package: all checks that require Nix (VM tests).
          # scripts/verify builds this instead of `nix flake check` to avoid
          # re-running format/clippy/nextest/deny that it already ran via Cargo.
          nix-only-checks = pkgs.symlinkJoin {
            name = "jaunder-nix-only-checks";
            paths = [
              self.packages.${system}.e2e-checks
            ];
          };
        };

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
          pkgs.lib.optionalAttrs pkgs.stdenv.isLinux {
            e2e-sqlite = mkE2eSqliteCheck {
              checkName = "jaunder-e2e-sqlite";
              warmupEnv = " JAUNDER_E2E_WARMUP=1";
            };

            e2e-postgres = mkE2ePostgresCheck {
              checkName = "jaunder-e2e-postgres";
              warmupEnv = " JAUNDER_E2E_WARMUP=1";
            };
          }
          // {
            clippy = craneLib.cargoClippy (
              commonArgs
              // {
                inherit cargoArtifacts;
                cargoClippyExtraArgs = "--all-targets -- -D warnings";
              }
            );
            rustfmt = craneLib.cargoFmt {
              inherit src;
              pname = "jaunder";
              version = "0.1.0";
            };
            leptosfmt-check =
              pkgs.runCommand "leptosfmt-check"
                {
                  nativeBuildInputs = [ pkgs.leptosfmt ];
                }
                ''
                  cd ${src}
                  leptosfmt -x .direnv -x .git -x target --check '**/*.rs'
                  touch $out
                '';
            nextest = craneLib.cargoNextest (
              commonArgs
              // {
                inherit cargoArtifacts;
                preCheck = ''
                  export LD_LIBRARY_PATH="${pkgs.lib.makeLibraryPath [ pkgs.openssl ]}:$LD_LIBRARY_PATH"
                '';
              }
            );
            deny = craneLib.cargoDeny {
              inherit src;
              pname = "jaunder";
              version = "0.1.0";
            };
            coverage = craneLib.mkCargoDerivation (
              commonArgs
              // {
                src = pkgs.lib.cleanSourceWith {
                  src = ./.;
                  filter =
                    path: _type:
                    !(pkgs.lib.hasInfix "/xtask/" path)
                    && !(pkgs.lib.hasInfix "/docs/" path)
                    && !(pkgs.lib.hasInfix "/.github/" path);
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
                nativeBuildInputs = commonArgs.nativeBuildInputs ++ [
                  cargo-crap
                  pkgs.cargo-llvm-cov
                  pkgs.cargo-nextest
                  pkgs.jq
                  pkgs.gawk
                  # check-coverage runs a host-PostgreSQL pass (via
                  # scripts/with-ephemeral-postgres) so storage/src/postgres/*
                  # gets instrumented coverage. The throwaway cluster needs
                  # initdb/pg_ctl/psql available inside the build sandbox.
                  pkgs.postgresql_16
                ];
                buildPhaseCargoCommand = ''
                  export LD_LIBRARY_PATH="${pkgs.lib.makeLibraryPath [ pkgs.openssl ]}:''${LD_LIBRARY_PATH:-}"
                  bash ./scripts/check-coverage --emit
                '';
                installPhaseCommand = ''
                  mkdir -p $out
                  # Non-dotted names: host xtask reads $out/coverage-report.txt
                  # and $out/crap-report.json (a plain `cp … $out/` would keep
                  # the leading dot and hide them).
                  cp .coverage-report.txt $out/coverage-report.txt
                  cp .crap-report.json $out/crap-report.json
                '';
              }
            );
            prettier-check =
              pkgs.runCommand "prettier-check"
                {
                  nativeBuildInputs = [ pkgs.prettier ];
                }
                ''
                  prettier --check ${end2endSrc}
                  touch $out
                '';
          };

        devShells.default = pkgs.mkShell {
          buildInputs = [
            toolchain
            pkgs.cachix
            cargo-crap
            pkgs.cargo-deny
            pkgs.cargo-generate
            pkgs.cargo-leptos
            pkgs.cargo-llvm-cov
            pkgs.cargo-mutants
            pkgs.cargo-nextest
            pkgs.dart-sass
            pkgs.jq
            pkgs.leptosfmt
            pkgs.nodejs
            pkgs.openssl
            pkgs.pkg-config
            pkgs.playwright-test
            pkgs.postgresql_16
            pkgs.prettier
            serena.packages.${pkgs.stdenv.hostPlatform.system}.serena
            pkgs.sqlx-cli
            pkgs.sqlite
            pkgs.typescript-language-server
            pkgs.vscode-langservers-extracted
            wasm-bindgen-cli
          ]
          ++ pkgs.lib.optionals pkgs.stdenv.isDarwin [
            pkgs.darwin.apple_sdk.frameworks.SystemConfiguration
          ];
          RUST_SRC_PATH = "${toolchain}/lib/rustlib/src/rust/library";
          PLAYWRIGHT_BROWSERS_PATH = "${pkgs.playwright-driver.browsers}";
          PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD = "1";
          shellHook = ''
            export LD_LIBRARY_PATH="${pkgs.lib.makeLibraryPath [ pkgs.openssl ]}:$LD_LIBRARY_PATH"

            # Symlink Nix-provided Playwright into node_modules to avoid instance conflict
            # and provide IDE support without redundant disk usage.
            mkdir -p end2end/node_modules/@playwright
            ln -sfn ${pkgs.playwright-test}/lib/node_modules/@playwright/test end2end/node_modules/@playwright/test
          '';
        };
      }
    );
}
