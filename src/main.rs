// SPDX-FileCopyrightText: 2021 Serokell <https://serokell.io>
//
// SPDX-License-Identifier: MPL-2.0

use std::process::Command;
use xdg::BaseDirectories;

use log::*;
use thiserror::Error;

use clap::Clap;

use serde::Deserialize;
use serde_json::from_str;

mod git;
use git::*;
mod flake_lock;
mod types;
use types::*;
mod request;
use request::submit_or_update_request;

#[derive(Debug, Error)]
enum FlakeUpdateError {
    #[error("Unable to find a git workdir")]
    GitError,
    #[error("Error while running the command: {0}")]
    CommandError(#[from] std::io::Error),
    #[error("Command was terminated or exited with a non-zero status: {0:?}")]
    ExitStatusError(Option<i32>),
}

fn flake_update<'a>(repo: Arc<Mutex<Repository>>) -> Result<(), FlakeUpdateError> {
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
    let status = nix_flake_update.status()?;

    if ! status.success() {
        return Err(FlakeUpdateError::ExitStatusError(status.code()));
    }

    Ok(())
}

#[derive(Debug, Error)]
enum UpdateError {
    #[error("Error during repository initialisation: {0}")]
    InitError(#[from] git::InitError),
    #[error("Failed to get flake lock information: {0}")]
    GetLockError(#[from] flake_lock::GetLockError),
    #[error("Error during update branch setup: {0}")]
    SetupUpdateBranchError(#[from] git::SetupUpdateBranchError),
    #[error("Error during flake update: {0}")]
    FlakeUpdateError(#[from] FlakeUpdateError),
    #[error("Error while making a diff between lockfiles: {0}")]
    LockDiffError(#[from] flake_lock::LockDiffError),
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
    let workdir = repo
        .clone()
        .lock()
        .unwrap()
        .workdir()
        .unwrap()
        .to_path_buf();
    let default_branch_lock = flake_lock::get_lock(&workdir.clone())?;
    setup_update_branch(settings.clone(), repo.clone())?;
    let before = flake_lock::get_lock(&workdir.clone())?;
    flake_update(repo.clone())?;
    let after = flake_lock::get_lock(&workdir)?;
    let diff = before.diff(&after)?;
    if diff.len() > 0 {
        let diff_default = default_branch_lock.diff(&after)?;
        info!("{}:\n{}", handle, diff.spaced());
        commit(settings.clone(), repo.clone(), diff.spaced())?;
        push(settings.clone(), repo.clone())?;
        submit_or_update_request(settings, handle, diff_default.markdown()).await?;
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
    #[clap(subcommand)]
    subcmd: Option<SubCommand>,
}

#[derive(Debug, Clap)]
enum SubCommand {
    #[clap()]
    CheckConfig,
    #[clap()]
    DiffLocks {
        old: flake_lock::Lock,
        new: flake_lock::Lock,
    },
}

#[derive(Debug, Clone, Deserialize)]
struct Config {
    #[serde(flatten)]
    settings: UpdateSettings,
    repos: Vec<RepoHandle>,
}

fn good_panic<E, O>(description: &'static str) -> Box<dyn Fn(E) -> O>
where
    E: std::fmt::Display,
{
    Box::new(move |err| {
        error!("{}: {}", description, err.to_string());
        std::process::exit(1);
    })
}

#[tokio::main]
async fn main() {
    let options: Options = Options::parse();

    let mut builder = pretty_env_logger::formatted_builder();

    builder.filter_level(options.verbosity).init();

    if let Some(SubCommand::DiffLocks { old, new }) = options.subcmd {
        debug!("old:\n{:#?}", old);
        debug!("new:\n{:#?}", new);
        let diff = old
            .diff(&new)
            .unwrap_or_else(good_panic("Unable to generate a diff"));
        debug!("diff:\n{:#?}", diff);
        println!("{}", diff.spaced());
        std::process::exit(0);
    }

    let xdg = BaseDirectories::new().unwrap();
    let cache_dir = xdg
        .create_cache_directory("update-daemon")
        .unwrap_or_else(good_panic("Failed to create a cache directory"));
    let config_file = xdg.find_config_file("update-daemon/config.json");

    let config: Config = from_str(
        std::fs::read_to_string(options.config.unwrap_or_else(|| {
            config_file
                .expect("Unable to find a configuration file")
                .to_string_lossy()
                .to_string()
        }))
        .unwrap_or_else(good_panic("Unable to read the configuration file"))
        .as_str(),
    )
    .unwrap_or_else(good_panic("Unable to parse the configuration file"));

    match options.subcmd {
        Some(SubCommand::CheckConfig) => {
            info!("Config parsed successfully: \n{:#?}", config);
            std::process::exit(0);
        }
        _ => {
            debug!("{:?}", config);
        }
    }

    let mut handles = Vec::new();

    for repo in config.clone().repos {
        let state = UpdateState {
            cache_dir: cache_dir.clone(),
        };
        let settings = config.settings.clone();

        let repo_longlived = repo.clone();

        let handle = tokio::spawn(async move {
            match update_repo(repo, state, settings).await {
                Ok(()) => {}
                Err(e) => {
                    warn!("{}: {}", repo_longlived, e);
                }
            }
        });
        handles.push(handle);
    }
    futures::future::join_all(handles).await;
}
