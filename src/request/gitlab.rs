// SPDX-FileCopyrightText: 2021 Serokell <https://serokell.io>
//
// SPDX-License-Identifier: MPL-2.0

use super::super::types::UpdateSettings;
use thiserror::Error;

use log::*;

use gitlab::api::projects::merge_requests::*;
use gitlab::api::*;

#[derive(Debug, Error)]
#[allow(clippy::enum_variant_names)]
pub enum MergeRequestError {
    #[error("Error during a gitlab operation: {0}")]
    GitlabError(#[from] gitlab::GitlabError),
    #[error("Error during a gitlab API call: {0}")]
    GitlabApiError(
        #[from] gitlab::api::ApiError<<gitlab::AsyncGitlab as gitlab::api::RestClient>::Error>,
    ),
    #[error("Couldn't create the endpoint: {0}")]
    GitlabEndpointError(String),
    #[error("Couldn't get a gitlab token from env var: {0}")]
    TokenError(#[from] std::env::VarError),
}

pub async fn submit_or_update_merge_request(
    settings: UpdateSettings,
    base_url: Option<String>,
    project: String,
    token_env_var: Option<String>,
    body: String,
    submit: bool,
) -> Result<(), MergeRequestError> {
    let gitlab = gitlab::Gitlab::builder(
        base_url.unwrap_or_else(|| "gitlab.com".to_string()),
        std::env::var(token_env_var.unwrap_or_else(|| "GITLAB_TOKEN".to_string()))?,
    )
    .build_async()
    .await?;

    let mr_search = MergeRequests::builder()
        .project(project.clone())
        .state(MergeRequestState::Opened)
        .target_branch(&settings.default_branch)
        .source_branch(&settings.update_branch)
        .build()
        .map_err(|_| MergeRequestError::GitlabEndpointError("building merge request".to_string()))?;

    let mut mr_page: Vec<gitlab::types::MergeRequest> = mr_search.query_async(&gitlab).await?;

    if let Some(mr) = mr_page.pop() {
        let mr_edit = EditMergeRequest::builder()
            .project(mr.project_id.value())
            .merge_request(mr.iid.value())
            .title(settings.title)
            .description(body)
            .build()
            .map_err(|_| MergeRequestError::GitlabEndpointError("building merge request".to_string()))?;

        let mr: gitlab::types::MergeRequest = mr_edit.query_async(&gitlab).await?;

        info!("Updated MR {}", mr.web_url);
    } else if submit {
        let mr_create = CreateMergeRequest::builder()
            .project(project)
            .target_branch(&settings.default_branch)
            .source_branch(&settings.update_branch)
            .title(settings.title)
            .description(body)
            .build()
            .map_err(|_| MergeRequestError::GitlabEndpointError("creating merge request".to_string()))?;

        let mr: gitlab::types::MergeRequest = mr_create.query_async(&gitlab).await?;

        info!("Created MR {}", mr.web_url);
    }

    Ok(())
}

pub async fn submit_issue_or_merge_request_comment(
    settings: UpdateSettings,
    base_url: Option<String>,
    project: String,
    token_env_var: Option<String>,
    title: String,
    body: String,
) -> Result<(), MergeRequestError> {
    let gitlab = gitlab::Gitlab::builder(
        base_url.unwrap_or_else(|| "gitlab.com".to_string()),
        std::env::var(token_env_var.unwrap_or_else(|| "GITLAB_TOKEN".to_string()))?,
    )
    .build_async()
    .await?;

    let mr_search = MergeRequests::builder()
        .project(project.clone())
        .state(MergeRequestState::Opened)
        .target_branch(&settings.default_branch)
        .source_branch(&settings.update_branch)
        .build()
        .map_err(|_| MergeRequestError::GitlabEndpointError("building merge request".to_string()))?;

    let mut mr_page: Vec<gitlab::types::MergeRequest> = mr_search.query_async(&gitlab).await?;

    // If there is a MR already, comment on it
    if let Some(mr) = mr_page.pop() {
        let mr_note_create = notes::CreateMergeRequestNote::builder()
            .project(mr.project_id.value())
            .merge_request(mr.iid.value())
            .body(body)
            .build()
            .map_err(|_| MergeRequestError::GitlabEndpointError("building merge request note".to_string()))?;

        let _ : gitlab::types::Note = mr_note_create.query_async(&gitlab).await?;
    } else {
        // let me = crab.current().user().await?.login;

        let me_query = users::CurrentUser::builder()
            .build()
            .map_err(|_| MergeRequestError::GitlabEndpointError("building current user".to_string()))?;

        let me: gitlab::types::User = me_query.query_async(&gitlab).await?;

        // FIXME: technically this might match unrelated issues if the user is not uniquely used by this bot

        let issue_search = projects::issues::Issues::builder()
            .project(project.clone())
            .state(projects::issues::IssueState::Opened)
            .author(me.id.value())
            .build()
            .map_err(|_| MergeRequestError::GitlabEndpointError("building issue".to_string()))?;

        let mut issues: Vec<gitlab::types::Issue> = issue_search.query_async(&gitlab).await?;

        if issues.len() > 1 {
            warn!("More than one issue; picking the first one and hoping for the best");
        }

        if let Some(issue) = issues.pop() {
            let issue_note_create = projects::issues::notes::CreateIssueNote::builder()
                .project(issue.project_id.value())
                .issue(issue.iid.value())
                .body(body)
                .build()
                .map_err(|_| MergeRequestError::GitlabEndpointError("creating issue".to_string()))?;

            let _ : gitlab::types::Note = issue_note_create.query_async(&gitlab).await?;
        } else {
            let issue_create = projects::issues::CreateIssue::builder()
                .project(project)
                .title(title)
                .description(body)
                .build()
                .map_err(|_| MergeRequestError::GitlabEndpointError("creating issue".to_string()))?;

            let _ : gitlab::types::Issue = issue_create.query_async(&gitlab).await?;
        }
    }

    Ok(())
}
