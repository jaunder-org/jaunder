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

    # TEMPORARY (jaunder #193 / ADR-0043): forks of atom_syndication and rss that
    # depend on quick-xml >= 0.41 (clears RUSTSEC-2026-0194/0195). Pinned to the exact
    # revs referenced by the root Cargo.toml [patch.crates-io]; fed to crane's cargo
    # vendor step via overrideVendorGitCheckout so the git [patch] resolves hermetically
    # (no build-time network). Remove once upstream releases on quick-xml >= 0.41.
    atom-fork = {
      url = "github:jaunder-org/atom/2462e3798295047ba35078b9634bcd129e887ffe";
      flake = false;
    };
    rss-fork = {
      url = "github:jaunder-org/rss/60b2a81445160af85ab94a23774c74e6616decfe";
      flake = false;
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      fenix,
      flake-utils,
      crane,
      atom-fork,
      rss-fork,
    }:
    let
      interactiveTestingVmSystem = "x86_64-linux";
      postgresTestingVmSystem = "x86_64-linux";
      mailCaptureEnv = {
        JAUNDER_MAIL_CAPTURE_FILE = "/var/lib/jaunder/mail.jsonl";
        JAUNDER_WEBSUB_CAPTURE_FILE = "/var/lib/jaunder/websub.jsonl";
        # Scoped server-diagnostics capture (#144): the server appends WARN+ events
        # and panic records here as JSONL. Spliced into the jaunder.service env below,
        # so the *server* (not the Playwright process) writes it; copied out per combo
        # in e2eRunAndCapture. (Name is now a slight misnomer — #227 consolidates.)
        JAUNDER_DIAG_LOG_FILE = "/var/lib/jaunder/jaunder-diag.log";
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

        # TEMPORARY (jaunder #193 / ADR-0043): vendor the git [patch.crates-io] forks of
        # atom_syndication / rss from pinned flake inputs rather than fetching them at
        # build time, so the hermetic (no-network) sandbox can resolve the patched
        # Cargo.lock. crane groups vendored crates by git source; for each fork source we
        # substitute the flake-input checkout (each fork's repo root *is* the crate).
        # crane's linkLockedDeps symlinks each git-sourced crate as `<name>-<version>`
        # pointing at `<checkout>/<name>-<version>`, so the returned checkout must place
        # the crate under that subdir. Each fork is a single-crate repo with its crate at
        # the root, so wrap the flake input in the expected `<name>-<version>/` layout.
        cargoVendorDir = craneLib.vendorCargoDeps {
          inherit src;
          overrideVendorGitCheckout =
            ps: drv:
            let
              forkFor =
                name:
                if name == "atom_syndication" then
                  atom-fork
                else if name == "rss" then
                  rss-fork
                else
                  null;
              p = builtins.head ps;
              fork = forkFor p.name;
            in
            if fork != null then
              pkgs.runCommandLocal "fork-vendor-${p.name}-${p.version}" { } ''
                dst="$out/${p.name}-${p.version}"
                mkdir -p "$dst"
                cp -a ${fork}/. "$dst/"
                chmod -R u+w "$dst"
                # Vendored git crates still need a checksum manifest; git sources carry
                # no registry checksum (package = null), and an empty files map skips
                # per-file verification (crane does the same for its own git checkouts).
                echo '{"files":{},"package":null}' > "$dst/.cargo-checksum.json"
              ''
            else
              drv;
        };

        commonArgs = {
          inherit src cargoVendorDir;
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

        # The in-sandbox dev tool (tools/ workspace: devtool + its coverage
        # path-dep), built as its OWN crane package with deps vendored from
        # tools/Cargo.lock. The offline coverage sandbox runs it from PATH
        # (nativeBuildInputs) instead of an in-sandbox `cargo run`, whose deps
        # would not be vendored. Building the self-contained tools/ workspace
        # (not the app root) keeps crane's metadata off the app's deps.
        devtoolSrc = pkgs.lib.cleanSourceWith {
          src = craneLib.path ./tools;
          filter = craneLib.filterCargoSources;
        };
        devtoolBin = craneLib.buildPackage {
          src = devtoolSrc;
          pname = "devtool";
          version = "0.1.0";
          cargoExtraArgs = "-p devtool";
          strictDeps = true;
          doCheck = false;
        };

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

        # The site the server serves: the CSR client's wasm bundle + public assets
        # + the CSR SPA shell (`csr/index.html`). CSR is the only client (#180); the
        # projector serves this same `index.html` as its SPA fallback.
        site = pkgs.runCommand "jaunder-site" { } ''
          mkdir -p $out/pkg
          cp -r ${csrWasmBundle}/. $out/pkg/
          cp -r ${./public}/. $out/
          cp ${./csr/index.html} $out/index.html
        '';

        # --- leptos-CSR client (#177/#180) --------------------------------------
        # The client-side-render wasm binary — the only client (#180 removed the
        # reactive SSR render). `csrWasmBundle` runs wasm-bindgen over it; `site`
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
                wasm-bindgen-cli
                pkgs.gnused
              ];
            }
            ''
              mkdir -p $out
              wasm-bindgen \
                --target web \
                --out-dir $out \
                ${csrWasm}/lib/csr.wasm
              # Rename to the "jaunder" output-name the CSR shell's <script> imports.
              mv $out/csr.js $out/jaunder.js
              mv $out/csr_bg.wasm $out/jaunder.wasm
              sed -i 's/csr_bg\.wasm/jaunder.wasm/g' $out/jaunder.js
            '';

        # Playwright config for the Nix VM environment.
        # Chromium needs --no-sandbox (runs as root) and GPU/shm disabled.
        # WebKit (WPE) is excluded: WPEWebProcess crashes with SIGABRT in
        # the NixOS VM, filling the disk with coredumps.
        nixPlaywrightConfig = pkgs.writeText "playwright.nix.config.js" ''
          const { defineConfig, devices } = require('@playwright/test');
          const traceParent = process.env.JAUNDER_E2E_TRACEPARENT;
          // Worker count is env-driven (#155). Default is 2: it cuts the e2e
          // wall-clock well below the old serial baseline (Firefox was the ~12min
          // long pole) while staying robust — 2 browser instances per combo is
          // far less bursty than 4, so it tolerates a shared/loaded host and lets
          // all four combos run concurrently in small (2-core/3GB) VMs without
          // oversubscribing a 16-core box. workers=4 is also viable (and ~1min
          // faster on the isolated CI combo) but needs cores>=workers, so its
          // 4-core VMs can't pack 4-wide — the local aggregate is slower and
          // needs concurrency throttling. workers=2 is the better balance
          // (measured #155). See docs/observability.md #155 AC3/AC4. Override
          // with JAUNDER_E2E_WORKERS.
          const workers = parseInt(process.env.JAUNDER_E2E_WORKERS || '2', 10);
          // Firefox in a headless VM defaults to Fission site-isolation plus a
          // pool of content processes; each Playwright worker is a separate
          // instance, so the RSS multiplies. These prefs collapse each instance
          // to a single content process and trim the in-memory caches. The e2e
          // suite exercises app behavior, not Firefox's process-isolation, so
          // this is transparent to the tests — it just cuts RSS enough to run
          // the VMs at 3 GB (#155, #61).
          const firefoxLaunchOptions = {
            firefoxUserPrefs: {
              'fission.autostart': false,
              'dom.ipc.processCount': 1,
              'dom.ipc.processCount.webIsolated': 1,
              'browser.sessionhistory.max_total_viewers': 0,
              'browser.cache.memory.capacity': 51200,
            },
          };
          module.exports = defineConfig({
            testDir: './tests',
            timeout: 30 * 1000,
            expect: { timeout: 5000 },
            reporter: [
              ['line'],
              ['json', { outputFile: '/tmp/e2e/playwright-report.json' }],
            ],
            use: {
              actionTimeout: 0,
              // Capture forensics only for failed tests, so a green run (the
              // common case) writes nothing extra and pays negligible overhead.
              // Recovered from the validate-diagnostics artifact on a red e2e
              // (#123/#49). No video — the trace already carries DOM snapshots.
              trace: 'retain-on-failure',
              screenshot: 'only-on-failure',
              ...(traceParent ? { extraHTTPHeaders: { traceparent: traceParent } } : {}),
            },
            // Artifact root for traces/screenshots; copied out by the testScript.
            outputDir: '/tmp/e2e/test-results',
            // SQLite write contention was the historical reason for workers:1,
            // but the pool runs WAL + 5s busy_timeout + BEGIN IMMEDIATE
            // (ADR-0039) and the #155 probes measured ZERO SQLITE_BUSY at 4
            // concurrent workers — the real limit was CPU oversubscription,
            // handled by worker-aware per-test budgets (fixtures.ts).
            workers: workers,
            fullyParallel: workers > 1,
            // admin-site mutates the site.title/base_url global singletons, so
            // under fullyParallel it must not overlap specs that read them
            // (ADR-0039). Each browser is split: the main project excludes
            // admin-site and runs in parallel; a serial `-admin` project runs
            // admin-site alone AFTER the main project (project `dependencies` +
            // fullyParallel:false). At workers=1 this is inert (all serial anyway).
            projects: [
              {
                name: 'chromium',
                testIgnore: /admin-site\.spec\.ts/,
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
                name: 'chromium-admin',
                testMatch: /admin-site\.spec\.ts/,
                fullyParallel: false,
                dependencies: ['chromium'],
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
                testIgnore: /admin-site\.spec\.ts/,
                use: {
                  ...devices['Desktop Firefox'],
                  launchOptions: firefoxLaunchOptions,
                },
              },
              {
                name: 'firefox-admin',
                testMatch: /admin-site\.spec\.ts/,
                fullyParallel: false,
                dependencies: ['firefox'],
                use: {
                  ...devices['Desktop Firefox'],
                  launchOptions: firefoxLaunchOptions,
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
        # to $out (before the assert, so a failing run is still diagnosable), then
        # fail the check on any `panicked at` line. Default-deny via `allowed_panics`.
        #
        # #144: panic detection now sources from the UNION of the scoped diag file
        # (`/var/lib/jaunder/jaunder-diag.log` — the low-noise primary the app's panic
        # hook writes) and the journal (the fallback, and the only source for a panic
        # that fires before the hook is installed). Reports are de-duped by panic
        # location, the scoped record winning; a location seen only in the journal is
        # still reported. Raw-substring scan (not JSON parsing) so a rare torn line in
        # the scoped file can't crash the gate.
        e2ePanicGate = backend: ''
          machine.succeed("journalctl -u jaunder.service --no-pager -o cat > /tmp/jaunder-journal-${backend}.log")
          # copy_from_vm's 2nd arg is a target *directory*; "" lands the file flat at
          # $out/jaunder-journal-${backend}.log (the per-backend name comes from the source).
          machine.copy_from_vm("/tmp/jaunder-journal-${backend}.log", "")
          journal = machine.succeed("cat /tmp/jaunder-journal-${backend}.log")
          # `cat` tolerates the scoped file being absent (empty output) — a run that
          # never wrote it still gates on the journal alone.
          diag = machine.execute("cat /var/lib/jaunder/jaunder-diag.log 2>/dev/null")[1]
          allowed_panics: list[str] = []  # default-deny; add a proven-benign substring + a comment here if one ever appears

          def panic_location(line):
              # Token after "panicked at ", trailing ':' stripped — canonical across BOTH
              # the scoped JSON record ("...panicked at src/x.rs:12:5: msg") and the default
              # hook's journal line ("...panicked at src/x.rs:12:5:", payload on the next
              # line). Both derive the path from the same `Location`, so the stripped tokens
              # match. Assumes the current toolchain's `panicked at <loc>:` format.
              return line.split("panicked at ", 1)[1].split()[0].rstrip(":")

          def collect(text):
              return [l for l in text.splitlines() if "panicked at" in l and not any(a in l for a in allowed_panics)]

          reports: dict[str, str] = {}
          for line in collect(diag):
              reports[panic_location(line)] = line       # scoped record is authoritative
          for line in collect(journal):
              reports.setdefault(panic_location(line), line)  # journal-only ⇒ pre-hook-install
          assert not reports, "e2e zero-panic gate (${backend}): jaunder.service logged Rust panic(s):\n" + "\n".join(reports.values())
        '';

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
            warmupEnv ? "",
          }:
          ''
            pw_status, pw_out = machine.execute(
              "cd /tmp/e2e"
              + " && PLAYWRIGHT_BROWSERS_PATH=${pkgs.playwright-driver.browsers}"
              + " PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD=1"
              + "${warmupEnv}"
              + " JAUNDER_MAIL_CAPTURE_FILE=/var/lib/jaunder/mail.jsonl"
              + " JAUNDER_WEBSUB_CAPTURE_FILE=/var/lib/jaunder/websub.jsonl"
              + " JAUNDER_DB=${jaunderDb}"
              + " JAUNDER_E2E_TRACE_ID=${traceId}"
              + " JAUNDER_E2E_TRACEPARENT=${traceParent}"
              + " JAUNDER_E2E_OTLP_HTTP_ENDPOINT=http://127.0.0.1:4318/v1/traces"
              + " ${pkgs.nodejs}/bin/node node_modules/.bin/playwright test"
              + " --config playwright.nix.config.js"
              + " --project ${browser} --project ${browser}-admin"
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

            # OTel trace keeps its directory layout
            # ($out/otel-traces-${backend}.jsonl/otel-traces.jsonl) that
            # `cargo xtask traces run` consumes on the success path: copy_from_vm's 2nd
            # arg is the target *dir* name (cf. #152). Guarded so an early crash with
            # no trace yet doesn't abort the remaining copies.
            if machine.execute("test -s /var/lib/jaunder/otel-traces.jsonl")[0] == 0:
                machine.copy_from_vm("/var/lib/jaunder/otel-traces.jsonl", "otel-traces-${backend}.jsonl")

            machine.execute("test -s /tmp/e2e/playwright-report.json && cp /tmp/e2e/playwright-report.json /tmp/playwright-report-${backend}.json")
            _grab("/tmp/playwright-report-${backend}.json")

            machine.execute("tar czf /tmp/playwright-artifacts-${backend}.tar.gz -C /tmp/e2e test-results 2>/dev/null || true")
            _grab("/tmp/playwright-artifacts-${backend}.tar.gz")

            machine.execute("journalctl --no-pager -o short-precise > /tmp/system-journal-${backend}.log")
            _grab("/tmp/system-journal-${backend}.log")

            # Scoped diagnostic log (#144): rename to the per-backend basename first so
            # it flat-copies as jaunder-diag-${backend}.log — the xtask lift filter keys
            # on the `jaunder-diag-` prefix, so a bare `jaunder-diag.log` would be dropped.
            machine.execute("test -s /var/lib/jaunder/jaunder-diag.log && cp /var/lib/jaunder/jaunder-diag.log /tmp/jaunder-diag-${backend}.log")
            _grab("/tmp/jaunder-diag-${backend}.log")

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
            warmupEnv ? "",
            vmMemory ? 2048,
            vmCores ? null,
          }:
          pkgs.testers.nixosTest {
            name = checkName;

            # Cap the test-driver budget at 20 min (default is 3600 s). Healthy
            # runs peak at ~10.6 min (slowest single-browser combo), so this is
            # ~1.9x headroom; a boot/infra hang now fails near 20 min instead of
            # burning the full hour. See issue #130.
            globalTimeout = 1200;

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
                machine.succeed(
                  "export JAUNDER_DB=sqlite:/var/lib/jaunder/data/jaunder.db; "
                  + "test-support create-user --username testlogin --password testpassword123 && "
                  + "test-support create-user --username testnoemail --password testpassword123 && "
                  + "test-support create-user --username testoperator --password testpassword123 --operator && "
                  + "test-support set-site-config --key site.registration_policy --value open && "
                  + "test-support set-site-config --key feeds.websub_hub_url --value https://hub.test.local/ && "
                  + "test-support reset-mail --path /var/lib/jaunder/mail.jsonl"
                )

              machine.start()
              machine.wait_for_unit("otel-collector.service", timeout=60)
              machine.wait_for_unit("jaunder.service", timeout=60)
              machine.wait_for_open_port(3000, timeout=30)

              machine.succeed("cp -r ${e2ePackage} /tmp/e2e && chmod -R u+w /tmp/e2e")
              machine.succeed("cp ${nixPlaywrightConfig} /tmp/e2e/playwright.nix.config.js")

              # Seed a fresh DB and run the one browser this derivation targets.
              # Browsers run as separate derivations (one VM each) so their state
              # mutations cannot interfere; that also lets CI fan them out.
              seed_db()
              ${e2eRunAndCapture {
                backend = "sqlite";
                jaunderDb = "sqlite:/var/lib/jaunder/data/jaunder.db";
                inherit browser traceId traceParent warmupEnv;
              }}
            '';
          };

        mkE2ePostgresCheck =
          {
            checkName,
            browser,
            traceId,
            traceParent,
            warmupEnv ? "",
            vmMemory ? 2048,
            vmCores ? null,
          }:
          pkgs.testers.nixosTest {
            name = checkName;

            # Cap the test-driver budget at 20 min (default is 3600 s). Healthy
            # runs peak at ~10.6 min (slowest single-browser combo), so this is
            # ~1.9x headroom; a boot/infra hang now fails near 20 min instead of
            # burning the full hour. See issue #130.
            globalTimeout = 1200;

            nodes.machine =
              { pkgs, lib, ... }:
              {
                imports = [ self.nixosModules.jaunder ];

                virtualisation.memorySize = vmMemory;
                # Default (null) leaves the nixosTest core count alone; the #155
                # workers=4 flip sets 4 (workers>1 needs the cores; 1 vCPU
                # timeshares and starves the client render).
                virtualisation.cores = lib.mkIf (vmCores != null) vmCores;
                environment.systemPackages = [
                  pkgs.postgresql_16
                  pkgs.opentelemetry-collector-contrib
                  testSupportBin
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
                # channels/subscription_statuses/target_kinds carry migration-seeded
                # reference data (migration 0018); the non-restartable Postgres path
                # can't re-seed them, so exclude them from the wipe.
                machine.succeed(
                  "sudo -u postgres psql -d jaunder -c \"DO \\$\\$ DECLARE r record;"
                  + " BEGIN FOR r IN SELECT tablename FROM pg_tables"
                  + " WHERE schemaname = 'public' AND tablename NOT LIKE '\\\\_sqlx%'"
                  + " AND tablename NOT IN ('channels', 'subscription_statuses', 'target_kinds') LOOP"
                  + " EXECUTE 'TRUNCATE TABLE ' || quote_ident(r.tablename) || ' CASCADE';"
                  + " END LOOP; END \\$\\$;\""
                )
                machine.succeed(
                  "export JAUNDER_DB=postgres://jaunder:testpassword@127.0.0.1/jaunder; "
                  + "test-support create-user --username testlogin --password testpassword123 && "
                  + "test-support create-user --username testnoemail --password testpassword123 && "
                  + "test-support create-user --username testoperator --password testpassword123 --operator && "
                  + "test-support set-site-config --key site.registration_policy --value open && "
                  + "test-support set-site-config --key feeds.websub_hub_url --value https://hub.test.local/ && "
                  + "test-support reset-mail --path /var/lib/jaunder/mail.jsonl"
                )

              # Seed a fresh DB and run the one browser this derivation targets.
              # Browsers run as separate derivations (one VM each) so their state
              # mutations cannot interfere; that also lets CI fan them out.
              seed_db()
              ${e2eRunAndCapture {
                backend = "postgres";
                jaunderDb = "postgres://jaunder:testpassword@127.0.0.1/jaunder";
                inherit browser traceId traceParent warmupEnv;
              }}
            '';
          };

        # All e2e {backend}×{browser} combos. backend picks the VM builder;
        # browser picks the Playwright --project; traceDigit gives each combo a
        # distinct OTel trace id (the 1/2/3/4 mapping preserves the historical
        # per-combo ids). Add a row here and the warm checks, the cold diagnostic
        # packages, and the `e2e-checks` aggregate all extend automatically.
        e2eCombos = [
          { backend = "sqlite";   browser = "chromium"; traceDigit = "1"; }
          { backend = "sqlite";   browser = "firefox";  traceDigit = "2"; }
          { backend = "postgres"; browser = "chromium"; traceDigit = "3"; }
          { backend = "postgres"; browser = "firefox";  traceDigit = "4"; }
        ];

        mkE2eCombo =
          {
            backend,
            browser,
            traceDigit,
            nameSuffix ? "",
            warmupEnv ? "",
            vmMemory ? 2048,
            vmCores ? null,
          }:
          let
            mk = if backend == "sqlite" then mkE2eSqliteCheck else mkE2ePostgresCheck;
            traceId = pkgs.lib.concatStrings (pkgs.lib.genList (_: traceDigit) 32);
            traceParent =
              "00-${traceId}-${pkgs.lib.concatStrings (pkgs.lib.genList (_: traceDigit) 16)}-01";
          in
          mk {
            checkName = "jaunder-e2e-${backend}-${browser}${nameSuffix}";
            inherit
              browser
              traceId
              traceParent
              warmupEnv
              vmMemory
              vmCores
              ;
          };

        # attr name -> warm check, e.g. { "e2e-sqlite-chromium" = <drv>; ... }
        # The warm gate runs at workers=2 (#155, see nixPlaywrightConfig), so the
        # VMs are sized 3 GB / 2 vCPU: cores >= workers avoids in-guest CPU
        # starvation, and with the Firefox process-slimming prefs 3 GB clears the
        # OOM that heavier VMs hit (#61). At workers=2 the per-VM footprint is
        # small enough that a 16-core dev box (and CI's per-combo runners) run the
        # combos comfortably; see docs/observability.md #155 AC3/AC4.
        e2eWarmChecks = pkgs.lib.listToAttrs (
          map (c: {
            name = "e2e-${c.backend}-${c.browser}";
            value = mkE2eCombo (
              c
              // {
                warmupEnv = " JAUNDER_E2E_WARMUP=1";
                vmMemory = 3072;
                vmCores = 2;
              }
            );
          }) e2eCombos
        );

        # Cold-cache variants (no warmup): same combos as the warm checks but the
        # first navigation of each test pays the full cold WASM download + init.
        # NOT part of the gate — built on demand by
        # `cargo xtask traces run --cold` to capture cold-cache OTel
        # navigation traces for performance diagnostics (see docs/observability.md).
        # Pinned to workers=1 (overriding the workers=4 gate default): these
        # measure per-navigation cold cost, where worker contention would corrupt
        # the attribution, and they keep the default 2 GB VM (4 Firefox workers
        # would OOM it, #61).
        e2eColdPackages = pkgs.lib.listToAttrs (
          map (c: {
            name = "e2e-${c.backend}-${c.browser}-cold";
            value = mkE2eCombo (
              c
              // {
                nameSuffix = "-cold";
                warmupEnv = " JAUNDER_E2E_WORKERS=1";
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
          // e2eColdPackages
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
            e2eWarmChecks
            // {
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
                  virtualisation.memorySize = 2048;
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
            deny = craneLib.cargoDeny {
              inherit src cargoVendorDir;
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
                      # web/src/render/mod.rs `include_str!`s csr/index.html
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
            prettier-check =
              pkgs.runCommand "prettier-check"
                {
                  nativeBuildInputs = [ pkgs.prettier ];
                }
                ''
                  prettier --check ${end2endSrc}
                  touch $out
                '';
            ert-check =
              pkgs.runCommand "ert-check"
                {
                  nativeBuildInputs = [ emacsForCi ];
                  # The pure ERT suite exercises timezone→UTC conversion and IANA
                  # zone-name validation, which need a zone database; a bare
                  # runCommand sandbox has none, so name lookups would silently
                  # fall back to UTC. Point the C library / Emacs at tzdata (#160).
                  TZDIR = "${pkgs.tzdata}/share/zoneinfo";
                }
                ''
                  emacs --batch -Q -l ${emacsSrc}/scripts/run-tests.el
                  touch $out
                '';
            elisp-fmt-check =
              pkgs.runCommand "elisp-fmt-check"
                {
                  nativeBuildInputs = [ emacsForCi ];
                }
                ''
                  emacs --batch -Q -l ${emacsSrc}/scripts/format.el -f jaunder-fmt-check
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
              pkgs.cargo-leptos
              pkgs.cargo-llvm-cov
              pkgs.cargo-nextest
              pkgs.curl
              pkgs.dart-sass
              emacsForCi
              pkgs.jq
              pkgs.leptosfmt
              pkgs.nodejs
              pkgs.openssl
              pkgs.pkg-config
              pkgs.playwright-test
              pkgs.postgresql_16
              pkgs.prettier
              pkgs.sqlite
              pkgs.typescript
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
              # `devtool run -- <cmd>` etc. on the interactive PATH. Already built
              # for the coverage sandbox; here it serves humans/agents directly.
              devtoolBin
            ];
            shellEnv = {
              RUST_SRC_PATH = "${toolchain}/lib/rustlib/src/rust/library";
              PLAYWRIGHT_BROWSERS_PATH = "${pkgs.playwright-driver.browsers}";
              PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD = "1";
              # The host `ert` step (run via `nix develop .#ci -c cargo xtask …`)
              # computes timezone->UTC from IANA zone names, which need a zone
              # database for `encode-time` to resolve. A clean CI runner has none
              # in this shell, so provide it deterministically rather than relying
              # on the host system's own TZDIR (which masked this locally). Mirrors
              # the ert-check derivation's TZDIR (#160).
              TZDIR = "${pkgs.tzdata}/share/zoneinfo";
              # Store paths for end2end/provision-node-modules.sh. Exported as env
              # vars (rather than baked into the shellHook) so they survive `cd`
              # into a worktree — that is what lets xtask's tsc-deps step re-run the
              # provisioning script there, where the shellHook never fired.
              E2E_TYPES_NODE_MODULES = "${e2ePackage}/node_modules";
              E2E_PLAYWRIGHT_TEST = "${pkgs.playwright-test}/lib/node_modules/@playwright/test";
              shellHook = ''
                export LD_LIBRARY_PATH="${pkgs.lib.makeLibraryPath [ pkgs.openssl ]}:$LD_LIBRARY_PATH"

                # Provision end2end/node_modules (the tsc type-dep closure) so the
                # devShell `tsc` and IDEs can type-check end2end/ offline in this
                # checkout. The same script also runs as xtask's tsc-deps gate step,
                # so worktrees self-heal there; see its header for the full rationale.
                bash end2end/provision-node-modules.sh
              '';
            };
          in
          {
            # Lean shell used by CI (`nix develop .#ci -c cargo xtask validate`).
            ci = pkgs.mkShell (shellEnv // { buildInputs = ciInputs; });
            # Full interactive shell for local development.
            default = pkgs.mkShell (shellEnv // { buildInputs = ciInputs ++ devOnly; });
          };
      }
    );
}
