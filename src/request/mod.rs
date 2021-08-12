// SPDX-FileCopyrightText: 2021 Serokell <https://serokell.io>
//
// SPDX-License-Identifier: MPL-2.0

use super::types::*;
use thiserror::Error;
use log::warn;

mod github;

#[derive(Debug, Error)]
pub enum RequestError {
    #[error("An error during github operation")]
    GithubError(#[from] github::PullRequestError),
}

pub async fn submit_or_update_request(
    settings: UpdateSettings,
    handle: RepoHandle,
    diff: String,
    submit: bool,
) -> Result<(), RequestError> {
    match handle {
        RepoHandle::GitHub { owner, repo } => {
            github::submit_or_update_pull_request(settings, owner, repo, diff, submit).await?;
        },
        RepoHandle::GitNone { url } => {
            warn!("Not sending a pull request for {}", url);
        },
    }
    Ok(())
}
