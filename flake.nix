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
    serena =  {
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
    flake-utils.lib.eachDefaultSystem (
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
            (pkgs.lib.hasSuffix ".sql" path) || (craneLib.filterCargoSources path type);
        };

        commonArgs = {
          inherit src;
          pname = "jaunder";
          version = "0.1.0";
          strictDeps = true;
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

        serverBin = craneLib.buildPackage (
          commonArgs
          // {
            inherit cargoArtifacts;
            cargoExtraArgs = "-p server";
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

      in
      {
        checks = pkgs.lib.optionalAttrs pkgs.stdenv.isLinux {
          e2e = pkgs.testers.nixosTest {
            name = "jaunder-e2e";

            nodes.machine =
              { pkgs, ... }:
              {
                virtualisation.memorySize = 2048;
                environment.systemPackages = [ pkgs.sqlite ];

                systemd.services.jaunder = {
                  wantedBy = [ "multi-user.target" ];
                  environment = {
                    LEPTOS_SITE_ROOT = "${site}";
                    LEPTOS_SITE_ADDR = "127.0.0.1:3000";
                    LEPTOS_OUTPUT_NAME = "jaunder";
                    LEPTOS_PKG_DIR = "pkg";
                    LEPTOS_ENV = "PROD";
                    RUST_LOG = "info";
                  };
                  preStart = ''
                    mkdir -p /var/lib/jaunder
                    cat > /var/lib/jaunder/Leptos.toml <<'EOF'
                    [leptos]
                    output-name = "jaunder"
                    site-root = "${site}"
                    site-addr = "127.0.0.1:3000"
                    env = "PROD"
                    EOF
                    ${serverBin}/bin/server init --skip-if-exists
                  '';
                  serviceConfig = {
                    ExecStart = "${serverBin}/bin/server serve";
                    WorkingDirectory = "/var/lib/jaunder";
                    StateDirectory = "jaunder";
                    Restart = "on-failure";
                    RestartSec = "2s";
                  };
                };
              };

            testScript = ''
              machine.start()
              machine.wait_for_unit("jaunder.service", timeout=60)
              machine.wait_for_open_port(3000, timeout=30)

              machine.succeed("sqlite3 /var/lib/jaunder/data/jaunder.db \"INSERT OR REPLACE INTO site_config (key, value) VALUES ('site.registration_policy', 'open')\"")
              machine.succeed("cd /var/lib/jaunder && ${serverBin}/bin/server user-create --username testlogin --password testpassword123")

              machine.succeed("cp -r ${e2ePackage} /tmp/e2e && chmod -R u+w /tmp/e2e")
              machine.succeed("cp ${nixPlaywrightConfig} /tmp/e2e/playwright.nix.config.js")
              machine.succeed(
                "cd /tmp/e2e"
                + " && PLAYWRIGHT_BROWSERS_PATH=${pkgs.playwright-driver.browsers}"
                + " PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD=1"
                + " ${pkgs.nodejs}/bin/node node_modules/.bin/playwright test"
                + " --config playwright.nix.config.js"
              )
            '';
          };
        } // {
          clippy = craneLib.cargoClippy (commonArgs // {
            inherit cargoArtifacts;
            cargoClippyExtraArgs = "-- -D warnings";
          });
          rustfmt = craneLib.cargoFmt { inherit src; pname = "jaunder"; version = "0.1.0"; };
          leptosfmt-check = pkgs.runCommand "leptosfmt-check" {
            nativeBuildInputs = [ pkgs.leptosfmt ];
          } ''
            cd ${src}
            leptosfmt -x .direnv -x .git -x target --check '**/*.rs'
            touch $out
          '';
          nextest = craneLib.cargoNextest (commonArgs // {
            inherit cargoArtifacts;
          });
          deny = craneLib.cargoDeny { inherit src; pname = "jaunder"; version = "0.1.0"; };
          prettier-check = pkgs.runCommand "prettier-check" {
            nativeBuildInputs = [ pkgs.prettier ];
          } ''
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
        };
      }
    );
}
