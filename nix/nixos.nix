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
      defaultPackage = self.packages.${targetSystem}.jaunder;
    in
    let
      jaunderBin = cfg.package;
    in
    {
      options.services.jaunder = {
        enable = lib.mkEnableOption "the Jaunder service";

        package = lib.mkOption {
          type = lib.types.package;
          default = defaultPackage;
          defaultText = lib.literalExpression "self.packages.\${system}.jaunder";
          description = "Jaunder package used by the service and service account.";
        };

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
    backend:
    {
      lib,
      pkgs,
      config,
      ...
    }:
    {
      imports = [ self.nixosModules.jaunder ];

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
          ${config.services.jaunder.package}/bin/jaunder create-pg-db \
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
    modules = [ (productionBaselineVmModule "sqlite") ];
  };

  productionBaselinePostgresConfiguration = nixpkgs.lib.nixosSystem {
    system = interactiveTestingVmSystem;
    modules = [ (productionBaselineVmModule "postgres") ];
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
        (productionBaselineVmModule backend)
        {
          services.jaunder.package = package;
        }
      ];
    };
in
{
  nixosModules.jaunder = jaunderModule;
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
