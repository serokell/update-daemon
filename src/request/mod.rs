// SPDX-FileCopyrightText: 2021 Serokell <https://serokell.io>
//
// SPDX-License-Identifier: MPL-2.0

use super::types::*;
use log::warn;
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
    submit: bool,
) -> Result<(), RequestError> {
    match handle {
        RepoHandle::GitHub { base_url, owner, repo, token_env_var, .. } => {
            github::submit_or_update_pull_request(settings, base_url, owner, repo, token_env_var, diff, submit).await?;
        }
        RepoHandle::GitNone { url } => {
            warn!("Not sending a pull request for {}", url);
        }
    }
    Ok(())
}

#[derive(Debug, Error)]
pub enum ErrorReportError {
    #[error("An error during github operation")]
    GithubError(#[from] github::PullRequestError),
}

pub async fn submit_error_report(
    settings: UpdateSettings,
    handle: RepoHandle,
    report: String,
) -> Result<(), ErrorReportError> {
    match handle {
        RepoHandle::GitHub { base_url, owner, repo, token_env_var, .. } => {
            github::submit_issue_or_pull_request_comment(
                settings,
                base_url,
                owner,
                repo,
                token_env_var,
                "Failed to automatically update flake.lock".to_string(),
                report,
            )
            .await?;
        }
        RepoHandle::GitNone { url } => {
            warn!("Not submitting an error report for {}", url);
        }
    }
    Ok(())
}
