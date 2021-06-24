# SPDX-FileCopyrightText: 2021 Serokell <https://serokell.io/>
#
# SPDX-License-Identifier: MPL-2.0

{
  inputs = {
    nixpkgs.url = "github:serokell/nixpkgs";
    crate2nix = {
      url = "github:kolloch/crate2nix";
      flake = false;
    };
    flake-utils.url = "github:numtide/flake-utils";
    flake-compat = {
      url = "github:edolstra/flake-compat";
      flake = false;
    };
  };

  outputs = { self, nixpkgs, crate2nix, flake-utils, ... }:
    flake-utils.lib.eachSystem [ "x86_64-linux" ] (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
        crateName = "update-daemon";

        inherit (import "${crate2nix}/tools.nix" { inherit pkgs; })
          generatedCargoNix;

        project = import (generatedCargoNix {
          name = crateName;
          src = ./.;
        }) {
          inherit pkgs;
          defaultCrateOverrides = pkgs.defaultCrateOverrides // {
            # Crate dependency overrides go here
            update-daemon = oa: {
              nativeBuildInputs = [ pkgs.makeWrapper ];
              postInstall =
                "wrapProgram $out/bin/update-daemon --prefix PATH : ${
                  pkgs.lib.makeBinPath [ pkgs.nixUnstable pkgs.gitMinimal ]
                }";
            };
          };
        };

      in {
        packages.${crateName} = project.rootCrate.build;

        defaultPackage = self.packages.${system}.${crateName};

        devShell = pkgs.mkShell {
          inputsFrom = builtins.attrValues self.packages.${system};
          buildInputs = with pkgs; [
            nixUnstable
            cargo
            rust-analyzer
            rustfmt
            clippy
            openssl
            pkg-config
            reuse
          ];
        };
      }) // {
        nixosModules.update-daemon = import ./module.nix self;
        nixosConfigurations.container = nixpkgs.lib.nixosSystem {
          system = "x86_64-linux";
          modules = [
            self.nixosModules.update-daemon

            ({ config, pkgs, lib, ... }: {
              system.configurationRevision = lib.mkIf (self ? rev) self.rev;
              boot.isContainer = true;
              networking.useDHCP = false;
              networking.firewall.allowedTCPPorts = [ 80 ];
              networking.hostName = "update-daemon";

              services.update-daemon = {
                enable = true;
                secretFile = "/run/secrets/update-daemon/environment";
                package = self.packages.x86_64-linux.update-daemon;
                repos.github.serokell.update-daemon = { };
                settings = {
                  author.email = "operations@serokell.io";
                  author.name = "Update Bot";
                };
              };
            })
          ];
        };
      };
}
