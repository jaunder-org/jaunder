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
          sha256 = "sha256-zC8E38iDVJ1oPIzCqTk/Ujo9+9kx9dXq7wAwPMpkpg0=";
        };

        craneLib = (crane.mkLib pkgs).overrideToolchain toolchain;

        src = pkgs.lib.cleanSourceWith {
          src = craneLib.path ./.;
          filter =
            path: type:
            (pkgs.lib.hasSuffix ".sql" path)
            || (pkgs.lib.hasSuffix ".css" path)
            || (craneLib.filterCargoSources path type);
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
            # Tests are covered by the separate `nextest` check, which runs
            # each test in its own process.  Disabling here avoids a duplicate
            # `cargo test` run that shares a process across async tests and can
            # cause Leptos reactive state to leak between parallel tests.
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

        wasm-bindgen-cli = pkgs.wasm-bindgen-cli.overrideAttrs (old: rec {
          version = "0.2.115";
          src = pkgs.fetchCrate {
            pname = "wasm-bindgen-cli";
            inherit version;
            hash = "sha256-wRynyZKYEMoIhX64n4DkGG2iepU6rE5qdBjT1LkUgtE=";
          };
          cargoDeps = pkgs.rustPlatform.fetchCargoVendor {
            inherit src;
            hash = "sha256-+7hgX56dOo/GErpf/unRprv72Kkars5dOFew+NfZZMY=";
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

        postgresIntegrationTests = craneLib.buildPackage (
          commonArgs
          // {
            inherit cargoArtifacts;
            cargoExtraArgs = "-p jaunder --test commands --test storage --test web_account --test web_auth --test web_email --test web_password_reset";
            doCheck = false;
            installPhaseCommand = ''
              mkdir -p $out/lib $out/tests
              ln -s ${pkgs.openssl.out}/lib/libssl.so.3 $out/lib/libssl.so.3
              ln -s ${pkgs.openssl.out}/lib/libcrypto.so.3 $out/lib/libcrypto.so.3
              cp "$(find target/release/deps -maxdepth 1 -type f -executable -name 'commands-*' | head -n 1)" $out/tests/commands
              cp "$(find target/release/deps -maxdepth 1 -type f -executable -name 'storage-*' | head -n 1)" $out/tests/storage
              cp "$(find target/release/deps -maxdepth 1 -type f -executable -name 'web_account-*' | head -n 1)" $out/tests/web_account
              cp "$(find target/release/deps -maxdepth 1 -type f -executable -name 'web_auth-*' | head -n 1)" $out/tests/web_auth
              cp "$(find target/release/deps -maxdepth 1 -type f -executable -name 'web_email-*' | head -n 1)" $out/tests/web_email
              cp "$(find target/release/deps -maxdepth 1 -type f -executable -name 'web_password_reset-*' | head -n 1)" $out/tests/web_password_reset
            '';
          }
        );

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

        # PostgreSQL-backed Rust integration tests need a live database service,
        # so they run inside a NixOS VM instead of under the plain `nextest`
        # check. We keep one VM check per test binary: that is coarse enough to
        # avoid a derivation-per-test maintenance burden, but still fine-grained
        # enough that failures and long poles are easy to localize.
        postgresTestBinaryCheck =
          {
            checkName,
            testBinary,
            includeIgnored ? false,
            extraEnv ? "",
            filter ? "",
          }:
          pkgs.testers.nixosTest {
            name = checkName;

            nodes.machine =
              { pkgs, lib, ... }:
              {
                virtualisation.memorySize = 4096;
                virtualisation.diskSize = 4096;

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
                  '';
                  settings = {
                    listen_addresses = lib.mkForce "*";
                  };
                };

                environment.systemPackages = [
                  pkgs.postgresql_16
                ];
              };

            testScript = ''
              machine.start()
              machine.wait_for_unit("postgresql.service", timeout=60)
              machine.wait_until_succeeds(
                "sudo -u postgres psql -tAc \"SELECT 1 FROM pg_roles WHERE rolname = 'jaunder'\" | grep -q 1"
              )
              machine.wait_until_succeeds(
                "sudo -u postgres psql -tAc \"SELECT 1 FROM pg_database WHERE datname = 'jaunder'\" | grep -q 1"
              )
              machine.succeed(
                "${extraEnv}JAUNDER_PG_TEST_URL=postgres://jaunder@127.0.0.1/jaunder"
                + " ${postgresIntegrationTests}/tests/${testBinary}"
                + " ${if includeIgnored then "--include-ignored " else ""}--test-threads=1"
                + " ${filter}"
              )
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
                machine.succeed("sqlite3 /var/lib/jaunder/data/jaunder.db \"DELETE FROM users; DELETE FROM sessions; DELETE FROM invites; DELETE FROM email_verifications; DELETE FROM password_resets; DELETE FROM posts; DELETE FROM tags; DELETE FROM site_config;\"")
                machine.succeed("sqlite3 /var/lib/jaunder/data/jaunder.db \"INSERT OR REPLACE INTO site_config (key, value) VALUES ('site.registration_policy', 'open')\"")
                machine.succeed("cd /var/lib/jaunder && ${jaunderBin}/bin/jaunder user-create --username testlogin --password testpassword123")
                machine.succeed("cd /var/lib/jaunder && ${jaunderBin}/bin/jaunder user-create --username testnoemail --password testpassword123")
                machine.succeed("rm -f /var/lib/jaunder/mail.jsonl")

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
                machine.succeed("sudo -u postgres psql -d jaunder -c \"TRUNCATE users, sessions, invites, email_verifications, password_resets, posts, tags, site_config CASCADE\"")
                machine.succeed("sudo -u postgres psql -d jaunder -c \"INSERT INTO site_config (key, value) VALUES ('site.registration_policy', 'open')\"")
                machine.succeed(
                  "cd /var/lib/jaunder"
                  + " && ${jaunderBin}/bin/jaunder user-create"
                  + " --db postgres://jaunder:testpassword@127.0.0.1/jaunder"
                  + " --username testlogin"
                  + " --password testpassword123"
                )
                machine.succeed(
                  "cd /var/lib/jaunder"
                  + " && ${jaunderBin}/bin/jaunder user-create"
                  + " --db postgres://jaunder:testpassword@127.0.0.1/jaunder"
                  + " --username testnoemail"
                  + " --password testpassword123"
                )
                machine.succeed("rm -f /var/lib/jaunder/mail.jsonl")

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

            # `commands` includes PostgreSQL-only ignored bootstrap tests, so
            # this VM check runs the full binary with `--include-ignored`.
            postgres-commands = postgresTestBinaryCheck {
              checkName = "jaunder-postgres-commands";
              testBinary = "commands";
              includeIgnored = true;
              extraEnv = "JAUNDER_PG_BOOTSTRAP_TEST_URL=postgres://postgres@127.0.0.1/postgres ";
            };

            # `storage` also carries ignored PostgreSQL-only parity/migration
            # tests, so it likewise includes ignored cases in the VM run.
            postgres-storage = postgresTestBinaryCheck {
              checkName = "jaunder-postgres-storage";
              testBinary = "storage";
              includeIgnored = true;
            };

            postgres-web-account = postgresTestBinaryCheck {
              checkName = "jaunder-postgres-web-account";
              testBinary = "web_account";
            };

            postgres-web-auth = postgresTestBinaryCheck {
              checkName = "jaunder-postgres-web-auth";
              testBinary = "web_auth";
            };

            postgres-web-email = postgresTestBinaryCheck {
              checkName = "jaunder-postgres-web-email";
              testBinary = "web_email";
            };

            postgres-web-password-reset = postgresTestBinaryCheck {
              checkName = "jaunder-postgres-web-password-reset";
              testBinary = "web_password_reset";
            };
          }
          // {
            clippy = craneLib.cargoClippy (
              commonArgs
              // {
                inherit cargoArtifacts;
                cargoClippyExtraArgs = "-- -D warnings";
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
            pkgs.cargo-deny
            pkgs.cargo-generate
            pkgs.cargo-leptos
            pkgs.cargo-llvm-cov
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
            pkgs.wasm-bindgen-cli
          ]
          ++ pkgs.lib.optionals pkgs.stdenv.isDarwin [
            pkgs.darwin.apple_sdk.frameworks.SystemConfiguration
          ];
          RUST_SRC_PATH = "${toolchain}/lib/rustlib/src/rust/library";
          PLAYWRIGHT_BROWSERS_PATH = "${pkgs.playwright-driver.browsers}";
          PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD = "1";
          shellHook = ''
            export LD_LIBRARY_PATH="${pkgs.lib.makeLibraryPath [ pkgs.openssl ]}:$LD_LIBRARY_PATH"
          '';
        };
      }
    );
}
