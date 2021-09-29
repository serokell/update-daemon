// SPDX-FileCopyrightText: 2021 Serokell <https://serokell.io>
//
// SPDX-License-Identifier: MPL-2.0

use super::super::types::UpdateSettings;
use thiserror::Error;

use log::*;

const GITHUB_BASE_URL: &str = "https://api.github.com";

#[derive(Debug, Error)]
pub enum PullRequestError {
    #[error("Error during a github operation: {0:?}")]
    GithubError(#[from] octocrab::Error),
    #[error("Couldn't get a GITHUB_TOKEN env var: {0}")]
    TokenError(#[from] std::env::VarError),
}

pub async fn submit_or_update_pull_request(
    settings: UpdateSettings,
    base_url: Option<String>,
    owner: String,
    repo: String,
    token_env_var: Option<String>,
    body: String,
    submit: bool,
) -> Result<(), PullRequestError> {
    let crab = octocrab::OctocrabBuilder::new()
        .base_url(base_url.unwrap_or(GITHUB_BASE_URL.to_string()))?
        .personal_token(std::env::var(
            token_env_var.unwrap_or("GITHUB_TOKEN".to_string()),
        )?)
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

    // If there is a PR already, update it and be done
    if let Some(pr) = page.items.pop() {
        crab.issues(owner, repo)
            .update(pr.number as u64)
            .title(settings.title.as_str())
            .body(&body)
            .send()
            .await?;
        info!("Updated PR {}", pr.html_url);
    }
    // If there isn't, submit only when `submit` is passed
    else if submit {
        let pr = crab
            .pulls(owner.clone(), repo.clone())
            .create(
                settings.title,
                settings.update_branch,
                settings.default_branch,
            )
            .body(body)
            .maintainer_can_modify(true)
            .send()
            .await?;
        crab.issues(owner, repo).update(pr.number).send().await?;
        info!("Submitted PR {}", pr.html_url);
    }
    Ok(())
}

pub async fn submit_issue_or_pull_request_comment(
    settings: UpdateSettings,
    base_url: Option<String>,
    owner: String,
    repo: String,
    token_env_var: Option<String>,
    title: String,
    body: String,
) -> Result<(), PullRequestError> {
    let crab = octocrab::OctocrabBuilder::new()
        .base_url(base_url.unwrap_or(GITHUB_BASE_URL.to_string()))?
        .personal_token(std::env::var(
            token_env_var.unwrap_or("GITHUB_TOKEN".to_string()),
        )?)
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

    // If there is a PR already, comment on it
    if let Some(pr) = page.items.pop() {
        crab.issues(owner, repo)
            .create_comment(pr.number as u64, body)
            .await?;
    } else {
        let me = crab.current().user().await?.login;

        // FIXME: technically this might match unrelated issues if the user is not uniquely used by this bot
        let query = format!("state:open is:issue author:{} repo:{}/{}", me, owner, repo);

        let mut page = crab
            .search()
            .issues_and_pull_requests(query.as_str())
            .send()
            .await?;

        if let Some(issue) = page.items.pop() {
            crab.issues(owner, repo)
                .create_comment(issue.number as u64, body)
                .await?;
        } else {
            crab.issues(owner, repo)
                .create(title)
                .body(body)
                .send()
                .await?;
        }
    }

    Ok(())
}
