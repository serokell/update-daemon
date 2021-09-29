// SPDX-FileCopyrightText: 2021 Serokell <https://serokell.io>
//
// SPDX-License-Identifier: MPL-2.0

use super::super::types::UpdateSettings;
use thiserror::Error;

use log::*;

use gitlab::api::projects::merge_requests::*;
use gitlab::api::*;

#[derive(Debug, Error)]
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
        base_url.unwrap_or("gitlab.com".to_string()),
        std::env::var(token_env_var.unwrap_or("GITLAB_TOKEN".to_string()))?,
    )
    .build_async()
    .await?;

    let mr_search = MergeRequests::builder()
        .project(project.clone())
        .state(MergeRequestState::Opened)
        .target_branch(&settings.default_branch)
        .source_branch(&settings.update_branch)
        .build()
        .map_err(MergeRequestError::GitlabEndpointError)?;

    let mut mr_page: Vec<gitlab::types::MergeRequest> = mr_search.query_async(&gitlab).await?;

    if let Some(mr) = mr_page.pop() {
        let mr_edit = EditMergeRequest::builder()
            .project(mr.project_id.value())
            .merge_request(mr.iid.value())
            .title(settings.title)
            .description(body)
            .build()
            .map_err(MergeRequestError::GitlabEndpointError)?;

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
            .map_err(MergeRequestError::GitlabEndpointError)?;

        let mr: gitlab::types::MergeRequest = mr_create.query_async(&gitlab).await?;

        info!("Created MR {}", mr.web_url);
    }

    Ok(())
}
