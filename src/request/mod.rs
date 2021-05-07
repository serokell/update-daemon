// SPDX-FileCopyrightText: 2021 Serokell <https://serokell.io>
//
// SPDX-License-Identifier: MPL-2.0

use super::types::*;
use thiserror::Error;

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
) -> Result<(), RequestError> {
    match handle {
        RepoHandle::GitHub { owner, repo } => {
            github::submit_or_update_pull_request(settings, owner, repo, diff).await?;
        }
    }
    Ok(())
}
