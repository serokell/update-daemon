<!--
   - SPDX-FileCopyrightText: 2021 Serokell <https://serokell.io>
   -
   - SPDX-License-Identifier: MPL-2.0
   -->

# update-daemon

[![License: MPL 2.0](https://img.shields.io/badge/License-MPL%202.0-brightgreen.svg)](https://opensource.org/licenses/MPL-2.0)

update-daemon is a oneshot "daemon" that updates Nix flakes in multiple repositories
and sends "pull requests" (currently github and gitlab supported).

## Usage

The recommended way to set up update-daemon is by using the [provided NixOS
module](./module.nix). It also serves as documentation for config file
fields, in case you want to configure the daemon manually.

### Usage notes

- Currently, update-daemon runs as root and uses `/root/.cache/update-daemon` for caching repositories;
- By default, configuration will be read from `$XDG_CONFIG_HOME/update-daemon/config.json`, but you can override that by providing the configuration as a CLI argument;
- Flakes are fetched and updated in parallel;
- In case any of the flakes fail to update, update-daemon will exit with a non-zero exit code (but still finish updating all the other flakes), and submit an error report either as a comment on the PR or the issue;
- In case the PR already exists, update-daemon will force-push a single commit there, unless "human" commits are on the same branch compared to master, in which case it will fail.

## Hacking

`nix develop` (or `nix-shell`) should drop you in a shell with all the
tools needed for hacking on update-daemon in `PATH`. Otherwise, install
`rustc` and `cargo` manually, and make sure you have unstable `nix` and
`git` in `PATH`.

## Tests

Currently, there are tests for flake lock parsing, diffing and display.
To run them, `cargo check`.

## About Serokell

update-daemon is maintained and funded with ❤️ by [Serokell](https://serokell.io/)
The names and logo for Serokell are trademark of Serokell OÜ.

We love open source software! See [our other projects](https://serokell.io/community?utm_source=github) or [hire us](https://serokell.io/hire-us?utm_source=github) to design, develop and grow your idea!
