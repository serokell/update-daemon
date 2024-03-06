# SPDX-FileCopyrightText: 2021 Serokell <https://serokell.io/>
#
# SPDX-License-Identifier: MPL-2.0

{

  nixConfig = {
    flake-registry = "https://github.com/serokell/flake-registry/raw/master/flake-registry.json";
  };

  inputs = {
    flake-compat.flake = false;
    naersk.url = "github:nix-community/naersk";
  };

  outputs = { self, nixpkgs, flake-utils, serokell-nix, naersk, ... }:
    flake-utils.lib.eachSystem [ "x86_64-linux" ] (system:
      let
        pkgs = nixpkgs.legacyPackages.${system}.extend serokell-nix.overlay;
        naersk' = pkgs.callPackage naersk {};
        nix = pkgs.nix;

        update-daemon = naersk'.buildPackage {
          src = builtins.path {
            path = ./.;
            name = "update-daemon-src";
          };

          nativeBuildInputs = [
            pkgs.pkg-config
            pkgs.makeWrapper
          ];

          buildInputs = [
            pkgs.openssl
            pkgs.libgit2
            pkgs.libgpg-error
            pkgs.gpgme
          ];

          postInstall =
            "wrapProgram $out/bin/update-daemon --prefix PATH : ${
              pkgs.lib.makeBinPath [ nix pkgs.gitMinimal ]
            }";

          cargoTestCommands = x: x ++ [
            # pedantic clippy
            ''cargo clippy --all --all-features --tests -- \
                -D clippy::pedantic \
                -D warnings \
                -A clippy::module-name-repetitions \
                -A clippy::too-many-lines \
                -A clippy::cast-possible-wrap \
                -A clippy::cast-possible-truncation \
                -A clippy::nonminimal_bool''
          ];
        };
      in {
        packages = {
          inherit update-daemon;
          default = update-daemon;
        };

        checks = {
          trailing-whitespace = pkgs.build.checkTrailingWhitespace ./.;
          reuse-lint = pkgs.build.reuseLint ./.;
          inherit update-daemon;
        };

        devShells.default = pkgs.mkShell {
          #RUST_LOG = "trace";
          inputsFrom = builtins.attrValues self.packages.${system};
          RUST_SRC_PATH = "${pkgs.rust.packages.stable.rustPlatform.rustLibSrc}";
          buildInputs = with pkgs; [
            rustc
            rust.packages.stable.rustPlatform.rustLibSrc
            nix
            cargo
            rust-analyzer
            rustfmt
            clippy
            openssl
            pkg-config
            reuse
            libgit2
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
                repos = {
                  github.serokell.update-daemon = { };
                  gitlab."tezosagora/agora" = { };
                };
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
