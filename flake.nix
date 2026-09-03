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
    atom-fork = {
      url = "github:jaunder-org/atom/921118c311d2117956d86e25052918e7c549ef00";
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
    }:
    let
      nixosLayer = import ./nix/nixos.nix { inherit self nixpkgs; };
    in
    {
      inherit (nixosLayer) nixosModules nixosConfigurations;
    }
    // flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
        packageLayer = import ./nix/packages.nix { inherit system pkgs fenix crane atom-fork; };
        checkLayer = import ./nix/checks.nix {
          inherit self system pkgs;
          nixosInternals = nixosLayer.internals;
          packageInternals = packageLayer.internals;
        };
        devShells = import ./nix/dev-shells.nix {
          inherit system pkgs;
          packageInternals = packageLayer.internals;
        };
      in
      {
        packages = packageLayer.packages // checkLayer.packages;
        apps = nixosLayer.appsForSystem { inherit system pkgs; };
        checks = checkLayer.checks;
        inherit devShells;
      }
    );
}
