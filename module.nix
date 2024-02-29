# SPDX-FileCopyrightText: 2021 Serokell <https:#serokell.io>
#
# SPDX-License-Identifier: MPL-2.0

self:

{ pkgs, lib, config, ... }:
let
  cfg = config.services.update-daemon;
  repos = lib.concatLists [ (processGitLabRepos cfg.repos.gitlab) (processGitHubRepos cfg.repos.github) ];
  processGitHubRepos = repos: lib.concatLists (lib.mapAttrsToList (owner: lib.mapAttrsToList (repo: settings: {
    type = "github";
    inherit owner repo settings;
  })) repos);
  processGitLabRepos = lib.mapAttrsToList (project: settings: {
    type = "gitlab";
    inherit project settings;
  });
in {
  options.services.update-daemon = with lib;
    with types; {
      enable = mkEnableOption "A nix flake update daemon";
      package = mkOption {
        type = package;
        description = "A package from which to take update-daemon";
        default = self.packages.${pkgs.system}.update-daemon;
      };
      secretFile = mkOption {
        type = path;
        description = ''
          A file containing secrets:
          - GITHUB_TOKEN
          You can also set additional secrets to use them in agentSetup.
        '';
      };
      agentSetup = mkOption {
        type = str;
        description =
          "Bash commands to set up the ssh agent to handle authentication to git upstreams";
        default = "${pkgs.openssh}/bin/ssh-agent";
      };
      updateDates = mkOption {
        type = str;
        description =
          "A systemd.time specification for when to run the updates";
        default = "daily";
      };
      repos = {
        github = mkOption {
          type = attrsOf (attrsOf (attrs));
          description = "Github Repositories to update";
          default = { };
          example = { serokell.update-daemon = { }; };
        };
        gitlab = mkOption {
          type = attrsOf (attrs);
          description = "Gitlab Repositories to update";
          default = { };
        };
      };
      extraRepos = mkOption {
        type = listOf attrs;
        description = "Other repositories to update";
        default = [  ];
      };
      settings = {
        author = {
          name = mkOption {
            type = str;
            description = "Name to use in commits";
            default = "Flake Update Bot";
          };
          email = mkOption {
            type = str;
            description = "Email to use in commits";
          };
        };
        update_branch = mkOption {
          type = str;
          description = "The branch to push the updates to";
          default = "automatic-update";
        };
        default_branch = mkOption {
          type = str;
          description =
            "The branch to base the update on and submit the pull request for";
          default = "master";
        };
        title = mkOption {
          type = str;
          description = "GitHub pull request title";
          default = "Automatically update flake.lock to the latest version";
        };
        extra_body = mkOption {
          type = lines;
          description = "Extra lines to add to pull request body";
          default = "";
        };
        cooldown = mkOption {
          type = int;
          description = "Cooldown duration between updating pull requests (in milliseconds)";
          default = 100;
        };
        inputs = mkOption {
          type = listOf str;
          description = "List of input names to be updated, if empty, all inputs will be updated";
          default = [];
          example = [ "haskell-nix" ];
        };
        allow_missing_inputs = mkOption {
          type = bool;
          description = "If set to true, the update-daemon will not abort flake update if one of the names specified in the inputs option is not present in the flake.lock root node";
          default = false;
        };
        sign_commits = mkOption {
          type = bool;
          description = "Whether to sign commits, the signing key must be available in gpg-agent under the root user";
          default = false;
        };
        signing_key = mkOption {
          type = nullOr str;
          description = "Signing key ID or fingerprint, if not set, the default key will be used";
          default = null;
        };
      };
    };
  config = lib.mkIf cfg.enable {
    systemd.services.update-daemon = {
      description = "A daemon to update nix flakes";
      serviceConfig = {
        Type = "oneshot";
        EnvironmentFile = cfg.secretFile;
      };
      path = [ cfg.package ];
      script = ''
        ${cfg.agentSetup}
        update-daemon ${
          builtins.toFile "config.json"
          (builtins.toJSON (cfg.settings // { repos = repos ++ cfg.extraRepos; }))
        }
      '';
      startAt = cfg.updateDates;
    };
  };
}
