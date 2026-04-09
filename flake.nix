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
              }
              // lib.optionalAttrs cfg.prod {
                JAUNDER_ENV = "prod";
              };
              preStart = ''
                mkdir -p target
                ln -sfn ${site} target/site
                ${jaunderBin}/bin/jaunder init --skip-if-exists
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

          systemd.services.jaunder.environment = mailCaptureEnv // {
            JAUNDER_BIND = "0.0.0.0:3000";
          };

          services.getty.autologinUser = "jaunder";
          security.sudo.wheelNeedsPassword = false;

          users.users.jaunder.extraGroups = [ "wheel" ];
          users.users.jaunder.initialPassword = "jaunder";
          users.users.jaunder.packages = [ pkgs.sqlite ];

          system.stateVersion = "26.05";
        };

      interactiveTestingVmConfiguration = nixpkgs.lib.nixosSystem {
        system = interactiveTestingVmSystem;
        modules = [ interactiveTestingVmModule ];
      };

    in
    {
      nixosModules.jaunder = jaunderModule;
      nixosConfigurations.interactive-testing-vm = interactiveTestingVmConfiguration;
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
          filter = path: type: (pkgs.lib.hasSuffix ".sql" path) || (craneLib.filterCargoSources path type);
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

        # Playwright config for the Nix VM environment: adds --no-sandbox
        # (required when running Chromium as root) and disables GPU/shm.
        nixPlaywrightConfig = pkgs.writeText "playwright.nix.config.js" ''
          const { defineConfig, devices } = require('@playwright/test');
          module.exports = defineConfig({
            testDir: './tests',
            timeout: 30 * 1000,
            expect: { timeout: 5000 },
            reporter: 'line',
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
            ],
          });
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

      in
      {
        packages = pkgs.lib.optionalAttrs pkgs.stdenv.isLinux {
          jaunder = jaunderBin;
          site = site;
        };

        apps =
          pkgs.lib.optionalAttrs
            (pkgs.stdenv.isLinux && pkgs.stdenv.hostPlatform.system == interactiveTestingVmSystem)
            {
              interactive-testing-vm = {
                type = "app";
                program = "${interactiveTestingVmRunner}/bin/interactive-testing-vm";
              };
            };

        checks =
          pkgs.lib.optionalAttrs pkgs.stdenv.isLinux {
            e2e = pkgs.testers.nixosTest {
              name = "jaunder-e2e";

              nodes.machine =
                { pkgs, ... }:
                {
                  imports = [ self.nixosModules.jaunder ];

                  virtualisation.memorySize = 2048;
                  environment.systemPackages = [ pkgs.sqlite ];

                  services.jaunder.enable = true;
                  services.jaunder.bind = "127.0.0.1:3000";
                  systemd.services.jaunder.environment = mailCaptureEnv // {
                    RUST_LOG = "info";
                  };
                };

              testScript = ''
                machine.start()
                machine.wait_for_unit("jaunder.service", timeout=60)
                machine.wait_for_open_port(3000, timeout=30)

                machine.succeed("sqlite3 /var/lib/jaunder/data/jaunder.db \"INSERT OR REPLACE INTO site_config (key, value) VALUES ('site.registration_policy', 'open')\"")
                machine.succeed("cd /var/lib/jaunder && ${jaunderBin}/bin/jaunder user-create --username testlogin --password testpassword123")

                machine.succeed("cp -r ${e2ePackage} /tmp/e2e && chmod -R u+w /tmp/e2e")
                machine.succeed("cp ${nixPlaywrightConfig} /tmp/e2e/playwright.nix.config.js")
                machine.succeed(
                  "cd /tmp/e2e"
                  + " && PLAYWRIGHT_BROWSERS_PATH=${pkgs.playwright-driver.browsers}"
                  + " PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD=1"
                  + " JAUNDER_MAIL_CAPTURE_FILE=/var/lib/jaunder/mail.jsonl"
                  + " ${pkgs.nodejs}/bin/node node_modules/.bin/playwright test"
                  + " --config playwright.nix.config.js"
                )
              '';
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
            pkgs.prettier
            pkgs.sqlx-cli
            pkgs.sqlite
            pkgs.typescript-language-server
            pkgs.wasm-bindgen-cli
          ]
          ++ pkgs.lib.optionals pkgs.stdenv.isDarwin [
            pkgs.darwin.apple_sdk.frameworks.SystemConfiguration
          ];
          RUST_SRC_PATH = "${toolchain}/lib/rustlib/src/rust/library";
          shellHook = ''
            export LD_LIBRARY_PATH="${pkgs.lib.makeLibraryPath [ pkgs.openssl ]}:$LD_LIBRARY_PATH"
          '';
        };
      }
    );
}
