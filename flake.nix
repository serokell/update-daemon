# SPDX-FileCopyrightText: 2021 Serokell <https://serokell.io/>
#
# SPDX-License-Identifier: MPL-2.0

{

  nixConfig = {
    flake-registry = "https://github.com/serokell/flake-registry/raw/master/flake-registry.json";
  };

  inputs = {
    crate2nix.flake = false;
    flake-compat.flake = false;
  };

  outputs = { self, nixpkgs, crate2nix, flake-utils, nix, flake-compat, serokell-nix }:
    flake-utils.lib.eachSystem [ "x86_64-linux" ] (system:
      let
        pkgs = nixpkgs.legacyPackages.${system}.extend serokell-nix.overlay;
        crateName = "update-daemon";

        nix' = nix.defaultPackage.${system};

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
                  pkgs.lib.makeBinPath [ nix' pkgs.gitMinimal ]
                }";
            };
          };
        };

      in {
        packages.${crateName} = project.rootCrate.build;

        defaultPackage = self.packages.${system}.${crateName};

        checks = {
          trailing-whitespace = pkgs.build.checkTrailingWhitespace ./.;
          reuse-lint = pkgs.build.reuseLint ./.;
        };

        devShell = pkgs.mkShell {
          inputsFrom = builtins.attrValues self.packages.${system};
          RUST_SRC_PATH = "${pkgs.rust.packages.stable.rustPlatform.rustLibSrc}";
          buildInputs = with pkgs; [
            rustc
            rust.packages.stable.rustPlatform.rustLibSrc
            nix'
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
