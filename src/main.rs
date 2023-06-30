// SPDX-FileCopyrightText: 2021 Serokell <https://serokell.io>
//
// SPDX-License-Identifier: MPL-2.0

use std::path::Path;

use std::process::Command;
use std::sync::Arc;
use xdg::BaseDirectories;

use log::*;
use thiserror::Error;

use clap::Clap;

use serde::Deserialize;
use serde_json::from_str;

mod git;
use git::UDRepo;
mod flake_lock;
use flake_lock::Lock;
mod types;
use types::*;
mod request;

use merge::Merge;

use std::convert::TryInto;
use std::time::{Duration, Instant};
use tokio::sync::Mutex as TMutex;

#[derive(Debug, Error)]
enum FlakeUpdateError {
    #[error("Error while running the command: {0}")]
    Command(#[from] std::io::Error),
    #[error("Command output was not valid UTF-8: {0}")]
    Utf8(#[from] std::str::Utf8Error),
    #[error("Command was terminated or exited with a non-zero status {0:?} and the following output: \n {1}")]
    ExitStatus(Option<i32>, String),
    #[error("Input {0} is missing from the flake.lock root nodes. Check spelling or consider using the allow_missing_inputs configuration option.")]
    MissingInput(String),
}

fn flake_update(workdir: &Path, settings: &UpdateSettings, lock: &Lock) -> Result<(), FlakeUpdateError> {
    let mut nix_flake_update = Command::new("nix");
    nix_flake_update.arg("flake");

    // If a list of inputs to update is not provided, update all inputs
    if settings.inputs.is_empty() {
        nix_flake_update.arg("update");
    // Otherwise, update only the specified inputs
    } else {
        nix_flake_update.arg("lock");
        for input in settings.inputs.iter() {
            // Abort flake update if input is missing from the flake.lock root nodes
            // and allow_missing_inputs is not set
            if !settings.allow_missing_inputs && lock.get_root_dep(input.clone()).is_none() {
                return Err(FlakeUpdateError::MissingInput(input.clone()))
            };
            nix_flake_update.arg("--update-input");
            nix_flake_update.arg(input);
        };
    };

    nix_flake_update.arg("--no-warn-dirty");
    nix_flake_update.current_dir(workdir.to_str().unwrap());
    let output = nix_flake_update.output()?;

    info!("{}", std::str::from_utf8(&output.stdout)?);

    if !output.status.success() {
        return Err(FlakeUpdateError::ExitStatus(
            output.status.code(),
            std::str::from_utf8(&output.stderr)?.to_string(),
        ));
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
    FlakeUpdate(#[from] FlakeUpdateError),
    #[error("Error while making a diff between lockfiles: {0}")]
    LockDiffError(#[from] flake_lock::LockDiffError),
    #[error("Error during git commit: {0}")]
    CommitError(#[from] git::CommitError),
    #[error("Error during git push: {0}")]
    PushError(#[from] git::PushError),
    #[error("Error during request submission: {0}")]
    RequestError(#[from] request::RequestError),
}

async fn wait_for_delay(last_ts: Instant, delay: Duration) {
    let time_passed = Instant::now().duration_since(last_ts);
    if time_passed < delay {
        tokio::time::sleep(delay - time_passed).await;
    }
}

async fn update_repo(
    handle: RepoHandle,
    state: UpdateState,
    settings: UpdateSettings,
    previous_update: Arc<TMutex<Instant>>,
) -> Result<(), UpdateError> {
    info!("Updating {}", handle);

    let repo = UDRepo::init(state, &settings, &handle)?;
    let workdir = repo.path().unwrap();

    let default_branch_lock = flake_lock::get_lock(workdir)?;

    repo.setup_update_branch(&settings)?;

    let before = flake_lock::get_lock(workdir)?;

    flake_update(workdir, &settings, &before)?;

    let after = flake_lock::get_lock(workdir)?;

    let diff = before.diff(&after)?;
    let diff_default = default_branch_lock.diff(&after)?;

    let mut body = diff_default.markdown();
    body.push_str(&format!(
        "\nLast updated: {}\n\n{}",
        chrono::Utc::now(),
        settings.extra_body
    ));

    let delay = settings.cooldown;

    if diff.len() > 0 {
        info!("{}:\n{}", handle, diff.spaced());
        repo.commit(&settings, diff.spaced())?;
        repo.push(&settings)?;

        let mut locked_ts = previous_update.lock().await;
        wait_for_delay(*locked_ts, delay).await;
        let res = request::submit_or_update_request(settings, handle, body, true).await;
        *locked_ts = Instant::now();
        res?;
    } else {
        info!("{}: Nothing to update", handle);
        if diff_default.len() > 0 {
            repo.push(&settings)?;

            let mut locked_ts = previous_update.lock().await;
            wait_for_delay(*locked_ts, delay).await;
            let res = request::submit_or_update_request(settings, handle, body, true).await;
            *locked_ts = Instant::now();
            res?;
        }
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
#[allow(clippy::large_enum_variant)]
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
    settings: UpdateSettingsOptional,
    repos: Vec<Repo>,
}

fn good_panic<E, O>(description: &'static str, code: i32) -> Box<dyn Fn(E) -> O>
where
    E: std::fmt::Display,
{
    Box::new(move |err| {
        error!("{}: {}", description, err.to_string());
        std::process::exit(code);
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
            .unwrap_or_else(good_panic("Unable to generate a diff", 65));
        debug!("diff:\n{:#?}", diff);
        println!("{}", diff.spaced());
        std::process::exit(0);
    }

    let xdg = BaseDirectories::new().unwrap();
    let cache_dir = xdg
        .create_cache_directory("update-daemon")
        .unwrap_or_else(good_panic("Failed to create a cache directory", 77));
    let config_file = xdg.find_config_file("update-daemon/config.json");

    let config: Config = from_str(
        std::fs::read_to_string(options.config.unwrap_or_else(|| {
            config_file
                .expect("Unable to find a configuration file")
                .to_string_lossy()
                .to_string()
        }))
        .unwrap_or_else(good_panic("Unable to read the configuration file", 66))
        .as_str(),
    )
    .unwrap_or_else(good_panic("Unable to parse the configuration file", 78));

    match options.subcmd {
        Some(SubCommand::CheckConfig) => {
            info!("Config parsed successfully: \n{:#?}", config);
            let settings: Result<UpdateSettings, _> = config.settings.try_into();
            match settings {
                Err(e) => warn!("The default settings are incomplete, you must complete them for each separate repo: {}", e),
                Ok(s) => info!("Default settings are complete:\n{:#?}", s)
            }

            std::process::exit(0);
        }
        _ => {
            debug!("{:?}", config);
        }
    }

    let ts = Arc::new(TMutex::new(Instant::now()));
    let mut handles = Vec::new();

    for repo in config.clone().repos {
        let state = UpdateState {
            cache_dir: cache_dir.clone(),
        };

        let mut settings = repo
            .clone()
            .settings
            .unwrap_or_default();

        settings.merge(config.clone().settings);

        let repo_longlived = repo.clone();

        let ts_copy1 = Arc::clone(&ts);
        let ts_copy2 = Arc::clone(&ts);
        let handle = tokio::spawn(async move {
            match settings.try_into() {
                Err(e) => {
                    error!("{}: {}", repo_longlived.handle, e);
                    Err(())
                }
                Ok(settings) => match update_repo(
                    repo.handle.clone(),
                    state,
                    (&settings as &UpdateSettings).clone(),
                    ts_copy1,
                )
                .await
                {
                    Err(e) => {
                        error!("{}: {}", repo_longlived.handle, e);

                        let delay = (&settings as &UpdateSettings).cooldown;
                        let mut locked_ts = ts_copy2.lock().await;
                        wait_for_delay(*locked_ts, delay).await;
                        let result = request::submit_error_report(
                            settings,
                            repo.handle,
                            format!(
                                "I tried updating flake.lock, but failed:\n\n```\n{}\n```",
                                e
                            ),
                        )
                        .await;

                        *locked_ts = Instant::now();

                        if let Err(e) = result {
                            error!("An error occurred while submitting the error report: {}", e);
                        }
                        Err(())
                    }
                    Ok(()) => Ok(()),
                },
            }
        });
        handles.push(handle);
    }
    if futures::future::join_all(handles)
        .await
        .iter()
        .all(|res| matches!(res, Ok(r) if r.is_ok()))
    {
        std::process::exit(0);
    } else {
        error!("Errors occured, please see above logs");
        std::process::exit(1);
    };
}
