{ self, nixpkgs }:
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
  e2eOtelCollectorEnv = captureEnv // {
    OTELCOL_GRPC_ENDPOINT = "127.0.0.1:4317";
    OTELCOL_HTTP_ENDPOINT = "127.0.0.1:4318";
  };

  mkJaunderModule =
    package:
    {
      lib,
      pkgs,
      config,
      ...
    }:
    let
      cfg = config.services.jaunder;
      targetSystem = pkgs.stdenv.hostPlatform.system;
      jaunderBin =
        if package == null then self.packages.${targetSystem}.jaunder else package;
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

  mkJaunderStackModule =
    {
      lib,
      pkgs,
      config,
      ...
    }:
    let
      cfg = config.services.jaunder.stack;
      nonWhitespace = value: value != null && builtins.match "^[[:space:]]*$" value == null;
      caddyToken = value: builtins.match "^[A-Za-z0-9._-]+$" value != null;
      bcryptHash = builtins.match "^\\$2[ab]\\$(0[4-9]|[12][0-9]|3[01])\\$[./0-9A-Za-z]{53}$" cfg.observability.basicAuth.passwordHash != null;
      validUnpaddedBase64 = value:
        let
          length = builtins.stringLength value;
          remainder = length - 4 * builtins.div length 4;
          trailingCharacter = builtins.substring (length - 1) 1 value;
        in
        builtins.match "^[A-Za-z0-9+/]+$" value != null
        && remainder != 1
        && (remainder != 2 || builtins.match "^[AQgw]$" trailingCharacter != null)
        && (remainder != 3 || builtins.match "^[AEIMQUYcgkosw048]$" trailingCharacter != null);
      decodedBase64Length = value:
        let
          length = builtins.stringLength value;
        in
        3 * builtins.div length 4 + (if length - 4 * builtins.div length 4 == 2 then 1 else if length - 4 * builtins.div length 4 == 3 then 2 else 0);
      argon2idParts = builtins.match "^\\$argon2id\\$v=19\\$m=([1-9][0-9]*),t=([1-9][0-9]*),p=([1-9][0-9]*)\\$([A-Za-z0-9+/]+)\\$([A-Za-z0-9+/]+)$" cfg.observability.basicAuth.passwordHash;
      argon2idHash =
        argon2idParts != null
        && (let
          memoryCost = builtins.fromJSON (builtins.elemAt argon2idParts 0);
          parallelism = builtins.fromJSON (builtins.elemAt argon2idParts 2);
          salt = builtins.elemAt argon2idParts 3;
          hash = builtins.elemAt argon2idParts 4;
        in
        memoryCost >= 8 * parallelism
        && validUnpaddedBase64 salt
        && decodedBase64Length salt >= 8
        && validUnpaddedBase64 hash
        && decodedBase64Length hash >= 4);
      postgresqlDatabase = cfg.database == "postgresql";
      applicationHostName = if nonWhitespace cfg.hostName then cfg.hostName else "invalid-stack-host.invalid";
      observabilityHostName = if nonWhitespace cfg.observability.hostName then cfg.observability.hostName else "invalid-observability-host.invalid";
      hashAlgorithm = if bcryptHash then "bcrypt" else "argon2id";
      observabilityIngress = lib.optionalString (nonWhitespace cfg.observability.hostName) ''
        # The stores remain credential-free on loopback; this is their only remote access path.
        basic_auth ${hashAlgorithm} {
          ${cfg.observability.basicAuth.username} ${cfg.observability.basicAuth.passwordHash}
        }
        reverse_proxy /metrics* 127.0.0.1:8428
        reverse_proxy /logs* 127.0.0.1:9428
        reverse_proxy /traces* 127.0.0.1:10428
      '';
    in
    {
      imports = [ (mkJaunderModule null) ];

      options.services.jaunder.stack = {
        enable = lib.mkEnableOption "the single-host Jaunder deployment stack";
        hostName = lib.mkOption {
          type = lib.types.nullOr lib.types.str;
          default = null;
          description = "Public application host name served by Caddy.";
        };
        database = lib.mkOption {
          type = lib.types.enum [ "sqlite" "postgresql" ];
          default = "sqlite";
          description = "Database managed by the stack.";
        };
        observability = {
          hostName = lib.mkOption {
            type = lib.types.nullOr lib.types.str;
            default = null;
            description = "Optional HTTPS operator host for the loopback observability UIs.";
          };
          basicAuth = {
            username = lib.mkOption {
              type = lib.types.str;
              default = "";
              description = "Basic Auth username for the optional observability host.";
            };
            passwordHash = lib.mkOption {
              type = lib.types.str;
              default = "";
              description = "bcrypt or Argon2id password hash for the optional observability host.";
            };
          };
        };
      };

      config = lib.mkIf cfg.enable {
        assertions = [
          {
            jaunderStack = true;
            assertion = nonWhitespace cfg.hostName;
            message = "services.jaunder.stack.hostName must be non-whitespace when the stack is enabled.";
          }
          {
            jaunderStack = true;
            assertion = cfg.observability.hostName == null || nonWhitespace cfg.observability.hostName;
            message = "services.jaunder.stack.observability.hostName must be non-whitespace when configured.";
          }
          {
            jaunderStack = true;
            assertion = cfg.observability.hostName == null || nonWhitespace cfg.observability.basicAuth.username;
            message = "services.jaunder.stack.observability.basicAuth.username must be non-whitespace when an observability host is configured.";
          }
          {
            jaunderStack = true;
            assertion = cfg.observability.hostName == null || caddyToken cfg.observability.basicAuth.username;
            message = "services.jaunder.stack.observability.basicAuth.username must contain only ASCII letters, digits, periods, hyphens, or underscores when an observability host is configured.";
          }
          {
            jaunderStack = true;
            assertion = cfg.observability.hostName == null || cfg.observability.hostName != cfg.hostName;
            message = "services.jaunder.stack.observability.hostName must differ from services.jaunder.stack.hostName when configured.";
          }
          {
            jaunderStack = true;
            assertion = cfg.observability.hostName == null || nonWhitespace cfg.observability.basicAuth.passwordHash;
            message = "services.jaunder.stack.observability.basicAuth.passwordHash must be non-whitespace when an observability host is configured.";
          }
          {
            jaunderStack = true;
            assertion = cfg.observability.hostName == null || bcryptHash || argon2idHash;
            message = "services.jaunder.stack.observability.basicAuth.passwordHash must be a complete $2a$/$2b$ bcrypt or $argon2id$ encoded hash.";
          }
        ];

        networking.firewall.allowedTCPPorts = [ 80 443 ];
        services.caddy = lib.mkIf (nonWhitespace cfg.hostName) {
          enable = true;
          virtualHosts = {
            ${applicationHostName}.extraConfig = ''
              reverse_proxy 127.0.0.1:3000
            '';
          } // lib.optionalAttrs (nonWhitespace cfg.observability.hostName) {
            ${observabilityHostName}.extraConfig = observabilityIngress;
          };
        };

        services.jaunder = {
          enable = true;
          bind = "127.0.0.1:3000";
          prod = true;
          db = if postgresqlDatabase then "postgresql:///jaunder?host=/run/postgresql" else "sqlite:/var/lib/jaunder/data/jaunder.db";
        };
        systemd.services.jaunder.environment = {
          JAUNDER_LOG_FORMAT = "json";
          JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT = "http://127.0.0.1:4317";
        };

        # Prefixes are native store routes, so collector traffic bypasses Caddy and Basic Auth.
        services.victoriametrics = {
          enable = true;
          listenAddress = "127.0.0.1:8428";
          extraOptions = [ "-http.pathPrefix=/metrics" ];
        };
        services.victorialogs = {
          enable = true;
          listenAddress = "127.0.0.1:9428";
          extraOptions = [ "-http.pathPrefix=/logs" ];
        };
        services.victoriatraces = {
          enable = true;
          listenAddress = "127.0.0.1:10428";
          extraOptions = [ "-http.pathPrefix=/traces" ];
        };

        services.opentelemetry-collector = {
          enable = true;
          package = pkgs.opentelemetry-collector-contrib;
          settings = {
            receivers = {
              otlp.protocols = {
                grpc.endpoint = "127.0.0.1:4317";
                http.endpoint = "127.0.0.1:4318";
              };
              journald.units = [ "jaunder.service" ];
            };
            processors = {
              batch = { };
              "transform/jaunder-json".log_statements = [
                {
                  context = "log";
                  statements = [ "merge_maps(attributes, ParseJSON(body), \"upsert\") where IsMatch(body, \"^\\\\{\")" ];
                }
              ];
            };
            exporters = {
              prometheusremotewrite.endpoint = "http://127.0.0.1:8428/metrics/api/v1/write";
              "otlphttp/victoriatraces".traces_endpoint = "http://127.0.0.1:10428/traces/insert/opentelemetry/v1/traces";
              "otlphttp/victorialogs".logs_endpoint = "http://127.0.0.1:9428/logs/insert/opentelemetry/v1/logs";
            };
            service.pipelines = {
              metrics = {
                receivers = [ "otlp" ];
                processors = [ "batch" ];
                exporters = [ "prometheusremotewrite" ];
              };
              traces = {
                receivers = [ "otlp" ];
                processors = [ "batch" ];
                exporters = [ "otlphttp/victoriatraces" ];
              };
              logs = {
                receivers = [ "journald" ];
                processors = [ "transform/jaunder-json" "batch" ];
                exporters = [ "otlphttp/victorialogs" ];
              };
            };
          };
        };

        services.postgresql = lib.mkIf postgresqlDatabase {
          enable = true;
          ensureDatabases = [ "jaunder" ];
          ensureUsers = [
            {
              name = "jaunder";
              ensureDBOwnership = true;
            }
          ];
        };
        systemd.services.jaunder.after = lib.mkIf postgresqlDatabase [ "postgresql.target" ];
        systemd.services.jaunder.requires = lib.mkIf postgresqlDatabase [ "postgresql.target" ];
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
        pkgs.postgresql_18
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
        package = pkgs.postgresql_18;
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
        pkgs.postgresql_18
      ];

      system.stateVersion = "26.05";
    };

  postgresTestingVmConfiguration = nixpkgs.lib.nixosSystem {
    system = postgresTestingVmSystem;
    modules = [ postgresTestingVmModule ];
  };
  productionBaselineVmModule =
    {
      backend,
      package ? null,
    }:
    {
      lib,
      pkgs,
      config,
      ...
    }:
    let
      jaunderBin =
        if package == null then self.packages.${pkgs.stdenv.hostPlatform.system}.jaunder else package;
    in
    {
      imports = [ (mkJaunderModule package) ];

      networking.hostName = "jaunder-production-baseline-${backend}";
      boot.loader.grub.devices = [ "nodev" ];
      boot.kernelParams = [ "console=ttyS0" ];
      fileSystems."/" = {
        device = "/dev/vda";
        fsType = "ext4";
      };
      virtualisation.vmVariant = {
        virtualisation = {
          graphics = false;
          memorySize = 2048;
          diskSize = 4096;
        };
      };
      # Both listeners are reachable only through lifecycle-owned QEMU forwards
      # bound to host loopback.
      networking.firewall.allowedTCPPorts = [
        3000
        39000
      ];

      systemd.services.jaunder-baseline-control = {
        wantedBy = [ "multi-user.target" ];
        after = [ "network.target" ];
        path = [
          config.systemd.package
          pkgs.coreutils
          pkgs.gnugrep
          self.packages.${pkgs.stdenv.hostPlatform.system}.test-support
        ];
        serviceConfig = {
          ExecStart = "${pkgs.socat}/bin/socat TCP-LISTEN:39000,bind=0.0.0.0,reuseaddr,fork EXEC:${pkgs.bash}/bin/bash,stderr";
          Restart = "always";
        };
      };

      # Harness-only seeding runs through the lifecycle control channel. It is
      # deliberately absent from the deployable Jaunder package and service.
      environment.systemPackages = [ self.packages.${pkgs.stdenv.hostPlatform.system}.test-support ];

      services.jaunder = {
        enable = true;
        bind = "0.0.0.0:3000";
        prod = true;
        db =
          if backend == "sqlite" then
            "sqlite:/var/lib/jaunder/data/jaunder.db"
          else
            "postgres://jaunder@127.0.0.1/jaunder";
      };

      services.postgresql = lib.mkIf (backend == "postgres") {
        enable = true;
        package = pkgs.postgresql_16;
        authentication = ''
          local all all trust
          host all all 127.0.0.1/32 trust
          host all all ::1/128 trust
        '';
      };

      systemd.services.jaunder-baseline-postgres-bootstrap = lib.mkIf (backend == "postgres") {
        description = "Create the isolated Jaunder PostgreSQL role and database";
        after = [ "postgresql.service" ];
        requires = [ "postgresql.service" ];
        before = [ "jaunder.service" ];
        unitConfig.ConditionPathExists = "!/var/lib/jaunder/.postgres-bootstrapped";
        serviceConfig.Type = "oneshot";
        script = ''
          install -d -m 0700 -o jaunder -g jaunder /var/lib/jaunder/baseline-secrets
          ${pkgs.openssl}/bin/openssl rand -hex 32 > /var/lib/jaunder/baseline-secrets/db-password
          chown jaunder:jaunder /var/lib/jaunder/baseline-secrets/db-password
          chmod 0600 /var/lib/jaunder/baseline-secrets/db-password
          ${jaunderBin}/bin/jaunder create-pg-db \
            --bootstrap-db postgres://postgres@127.0.0.1/postgres \
            --app-db postgres://jaunder@127.0.0.1/jaunder \
            --app-role-password "$(cat /var/lib/jaunder/baseline-secrets/db-password)"
          touch /var/lib/jaunder/.postgres-bootstrapped
        '';
      };

      systemd.services.jaunder = lib.mkIf (backend == "postgres") {
        after = [ "jaunder-baseline-postgres-bootstrap.service" ];
        requires = [ "jaunder-baseline-postgres-bootstrap.service" ];
        environment.JAUNDER_DB_PASSWORD_FILE = "/var/lib/jaunder/baseline-secrets/db-password";
      };

      system.stateVersion = "26.05";
    };

  productionBaselineSqliteConfiguration = nixpkgs.lib.nixosSystem {
    system = interactiveTestingVmSystem;
    modules = [ (productionBaselineVmModule { backend = "sqlite"; }) ];
  };

  productionBaselinePostgresConfiguration = nixpkgs.lib.nixosSystem {
    system = interactiveTestingVmSystem;
    modules = [ (productionBaselineVmModule { backend = "postgres"; }) ];
  };
  productionBaselineVm =
    {
      system,
      package,
      backend,
    }:
    nixpkgs.lib.nixosSystem {
      inherit system;
      modules = [
        (productionBaselineVmModule {
          inherit backend package;
        })
      ];
    };
in
{
  nixosModules.jaunder = mkJaunderModule null;
  nixosModules.jaunder-stack = mkJaunderStackModule;
  nixosConfigurations.interactive-testing-vm = interactiveTestingVmConfiguration;
  nixosConfigurations.postgres-testing-vm = postgresTestingVmConfiguration;
  nixosConfigurations.production-baseline-sqlite = productionBaselineSqliteConfiguration;
  nixosConfigurations.production-baseline-postgres = productionBaselinePostgresConfiguration;
  inherit productionBaselineVm;


  packagesForSystem =
    { system, pkgs }:
    pkgs.lib.optionalAttrs (pkgs.stdenv.isLinux && system == interactiveTestingVmSystem) {
      production-baseline-sqlite-vm = productionBaselineSqliteConfiguration.config.system.build.vm;
      production-baseline-postgres-vm = productionBaselinePostgresConfiguration.config.system.build.vm;
      production-baseline-proxy = pkgs.writeShellApplication {
        name = "production-baseline-proxy";
        runtimeInputs = [ pkgs.caddy ];
        text = ''
          exec caddy run --config "$1" --adapter caddyfile
        '';
      };
    };

  appsForSystem =
    { system, pkgs }:
    let
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
    in
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
        production-baseline-sqlite = {
          type = "app";
          program = "${productionBaselineSqliteConfiguration.config.system.build.vm}/bin/run-jaunder-production-baseline-sqlite-vm";
        };
        production-baseline-postgres = {
          type = "app";
          program = "${productionBaselinePostgresConfiguration.config.system.build.vm}/bin/run-jaunder-production-baseline-postgres-vm";
        };
      };

  internals = {
    inherit captureEnv e2eOtelCollectorEnv;
  };
}
