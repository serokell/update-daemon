// SPDX-FileCopyrightText: 2021 Serokell <https://serokell.io>
//
// SPDX-License-Identifier: MPL-2.0

use super::super::types::UpdateSettings;
use thiserror::Error;

use log::*;

#[derive(Debug, Error)]
pub enum PullRequestError {
    #[error("Error during a github operation: {0:?}")]
    GithubError(#[from] octocrab::Error),
    #[error("Couldn't get a GITHUB_TOKEN env var: {0}")]
    TokenError(#[from] std::env::VarError),
}

pub async fn submit_or_update_pull_request(
    settings: UpdateSettings,
    owner: String,
    repo: String,
    diff: String,
) -> Result<(), PullRequestError> {
    let crab = octocrab::OctocrabBuilder::new()
        .personal_token(std::env::var("GITHUB_TOKEN")?)
        .build()?;
    let query = format!(
        "head:{} base:{} is:pr state:open repo:{}/{}",
        settings.update_branch, settings.default_branch, owner, repo
    );
    let mut page = crab
        .search()
        .issues_and_pull_requests(query.as_str())
        .send()
        .await?;
    if let Some(pr) = page.items.pop() {
        crab.issues(owner, repo)
            .update(pr.number as u64)
            .title(settings.title.as_str())
            .body(&diff)
            .send()
            .await?;
        info!("Updated PR {}", pr.html_url);
    } else {
        let pr = crab
            .pulls(owner.clone(), repo.clone())
            .create(
                settings.title,
                settings.update_branch,
                settings.default_branch,
            )
            .body(diff)
            .send()
            .await?;
        crab.issues(owner, repo).update(pr.number).send().await?;
        info!("Submitted PR {}", pr.html_url);
    }

    Ok(())
}
