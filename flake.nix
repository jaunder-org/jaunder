{
  description = "jaunder - a federated social media application";

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
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
        toolchain = fenix.packages.${system}.fromToolchainFile {
          file = ./rust-toolchain.toml;
          sha256 = "sha256-zC8E38iDVJ1oPIzCqTk/Ujo9+9kx9dXq7wAwPMpkpg0=";
        };

        craneLib = (crane.mkLib pkgs).overrideToolchain toolchain;

        src = craneLib.cleanCargoSource ./.;

        commonArgs = {
          inherit src;
          pname = "jaunder";
          version = "0.1.0";
          strictDeps = true;
          nativeBuildInputs = [ pkgs.pkg-config ];
          buildInputs = [
            pkgs.openssl
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

        frontendWasm = craneLib.buildPackage (
          commonArgs
          // {
            cargoArtifacts = craneLib.buildDepsOnly (
              commonArgs
              // {
                CARGO_BUILD_TARGET = "wasm32-unknown-unknown";
                cargoExtraArgs = "-p frontend";
                doCheck = false;
              }
            );
            CARGO_BUILD_TARGET = "wasm32-unknown-unknown";
            cargoExtraArgs = "-p frontend";
            doCheck = false;
            installPhaseCommand = ''
              mkdir -p $out/lib
              cp target/wasm32-unknown-unknown/release/frontend.wasm $out/lib/
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
                ${frontendWasm}/lib/frontend.wasm
              # Rename to match output-name = "jaunder" expected by the Leptos SSR HTML
              mv $out/frontend.js $out/jaunder.js
              mv $out/frontend_bg.wasm $out/jaunder_bg.wasm
              sed -i 's/frontend_bg\.wasm/jaunder_bg.wasm/g' $out/jaunder.js
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

      in
      {
        checks = pkgs.lib.optionalAttrs pkgs.stdenv.isLinux {
          e2e = pkgs.testers.nixosTest {
            name = "jaunder-e2e";

            nodes.machine =
              { pkgs, ... }:
              {
                virtualisation.memorySize = 2048;

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
                  '';
                  serviceConfig = {
                    ExecStart = "${serverBin}/bin/server";
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
        };

        devShells.default = pkgs.mkShell {
          buildInputs = [
            toolchain
            pkgs.cargo-generate
            pkgs.cargo-leptos
            pkgs.dart-sass
            pkgs.leptosfmt
            pkgs.nodejs
            pkgs.openssl
            pkgs.pkg-config
            pkgs.prettier
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
