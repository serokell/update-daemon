// SPDX-FileCopyrightText: 2021 Serokell <https://serokell.io>
//
// SPDX-License-Identifier: MPL-2.0

use std::process::{Command, Stdio};
use xdg::BaseDirectories;

use log::*;
use thiserror::Error;

use clap::Clap;

use serde::Deserialize;
use serde_json::from_str;

mod git;
use git::*;
mod types;
use types::*;
mod request;
use request::submit_or_update_request;

#[derive(Debug, Error)]
enum FlakeUpdateError {
    #[error("Unable to find a git workdir")]
    GitError,
    #[error("Command exited with a non-zero exit status")]
    ExitStatusError(#[from] std::io::Error),
    #[error("Error while decoding output")]
    OutputDecodeError(#[from] std::str::Utf8Error),
}

fn flake_update<'a>(repo: Arc<Mutex<Repository>>) -> Result<String, FlakeUpdateError> {
    let mut nix_flake_update = Command::new("nix");
    nix_flake_update.arg("flake");
    nix_flake_update.arg("update");
    nix_flake_update.arg("--no-warn-dirty");
    nix_flake_update.current_dir(
        repo.lock()
            .unwrap()
            .workdir()
            .ok_or(FlakeUpdateError::GitError)?
            .to_str()
            .unwrap(),
    );
    nix_flake_update.stderr(Stdio::piped());
    let output = nix_flake_update.output()?;

    let output = std::str::from_utf8(&output.stderr)?;

    let lines: Vec<&str> = output.split("\n").skip(1).collect();

    Ok(lines.join("\n"))
}

#[derive(Debug, Error)]
enum UpdateError {
    #[error("Error during repository initialisation: {0}")]
    InitError(#[from] git::InitError),
    #[error("Error during flake update: {0}")]
    FlakeUpdateError(#[from] FlakeUpdateError),
    #[error("Error during git commit: {0}")]
    CommitError(#[from] git::CommitError),
    #[error("Error during git push: {0}")]
    PushError(#[from] git::PushError),
    #[error("Error during request submission: {0}")]
    RequestError(#[from] request::RequestError),
}

async fn update_repo(
    handle: RepoHandle,
    state: UpdateState,
    settings: UpdateSettings,
) -> Result<(), UpdateError> {
    info!("Updating {}", handle);
    let repo = init_repo(state, settings.clone(), handle.clone())?;
    let diff = flake_update(repo.clone())?;
    if diff.len() > 1 {
        info!("{}:\n{}", handle, diff);
        commit(settings.clone(), repo.clone(), diff.clone())?;
        push(settings.clone(), repo.clone())?;
        submit_or_update_request(settings, handle, diff).await?;
    } else {
        info!("{}: Nothing to update", handle);
    }
    Ok(())
}

/// Submit "pull requests" (currently only Github supported) with nix flake updates
#[derive(Debug, Clap)]
#[clap(version = "0.1.0", author = "Serokell <https://serokell.io/>")]
struct Options {
    /// The configuration file
    #[clap()]
    config: Option<String>,
    /// Verbosity level
    #[clap(default_value = "info", long, short)]
    verbosity: log::LevelFilter,
}

#[derive(Debug, Clone, Deserialize)]
struct Config {
    #[serde(flatten)]
    settings: UpdateSettings,
    repos: Vec<RepoHandle>,
}

#[tokio::main]
async fn main() {
    let options: Options = Options::parse();

    let mut builder = pretty_env_logger::formatted_builder();

    builder.filter_level(options.verbosity).init();

    let xdg = BaseDirectories::new().unwrap();
    let cache_dir = xdg.create_cache_directory("update-daemon").unwrap();
    let config_file = xdg.find_config_file("update-daemon/config.json");

    let config : Config = from_str(
        std::fs::read_to_string(
            options
                .config
                .unwrap_or(config_file.unwrap().to_string_lossy().to_string()),
        )
        .unwrap()
        .as_str(),
    )
    .unwrap();

    debug!("{:?}", config);

    let mut handles = Vec::new();

    for repo in config.clone().repos {
        let state = UpdateState { cache_dir: cache_dir.clone() };
        let settings = config.settings.clone();

        let handle = tokio::spawn(async {
            match update_repo(repo, state, settings).await {
                Ok(()) => {},
                Err(e) => {
                    warn!("{0}", e);
                }
            }
        });
        handles.push(handle);
    }
    futures::future::join_all(handles).await;
}
