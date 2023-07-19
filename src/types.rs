// SPDX-FileCopyrightText: 2021 Serokell <https://serokell.io>
//
// SPDX-License-Identifier: MPL-2.0

use merge::Merge;
use serde::Deserialize;
use std::default::Default;
use std::fmt::{Display, Formatter};
use std::path::PathBuf;
use std::time::Duration;
use thiserror::Error;

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateSettings {
    pub author: Author,
    pub update_branch: String,
    pub default_branch: String,
    pub title: String,
    pub extra_body: String,
    pub cooldown: Duration,
    pub inputs: Vec<String>,
    pub allow_missing_inputs: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Author {
    pub name: String,
    pub email: String,
}

#[derive(Debug, Clone, Deserialize, Default, Merge)]
pub struct UpdateSettingsOptional {
    pub author: Option<Author>,
    pub update_branch: Option<String>,
    pub default_branch: Option<String>,
    pub title: Option<String>,
    pub extra_body: Option<String>,
    pub cooldown: Option<u64>,
    pub inputs: Option<Vec<String>>,
    pub allow_missing_inputs: Option<bool>,
}

#[derive(Debug, Error)]
pub struct UpdateSettingsMissingField(String);

impl std::fmt::Display for UpdateSettingsMissingField {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> Result<(), std::fmt::Error> {
        write!(f, "Settings missing field {}", self.0)
    }
}

fn unoption<T>(opt: Option<T>, name: &'static str) -> Result<T, UpdateSettingsMissingField> {
    opt.ok_or_else(|| UpdateSettingsMissingField(name.to_string()))
}

impl std::convert::TryInto<UpdateSettings> for UpdateSettingsOptional {
    type Error = UpdateSettingsMissingField;

    fn try_into(self) -> Result<UpdateSettings, Self::Error> {
        Ok(UpdateSettings {
            author: unoption(self.author, "author")?,
            update_branch: self.update_branch.unwrap_or_else(|| "automatic-update".to_string()),
            default_branch: self.default_branch.unwrap_or_else(|| "master".to_string()),
            title: self
                .title
                .unwrap_or_else(|| "Automatically update flake.lock".to_string()),
            extra_body: self.extra_body.unwrap_or_default(),
            // what if negative number in config?
            cooldown: Duration::from_millis(unoption(self.cooldown, "cooldown")?),
            inputs: self.inputs.unwrap_or_default(),
            allow_missing_inputs: self.allow_missing_inputs.unwrap_or(false),
        })
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateState {
    pub cache_dir: PathBuf,
}

#[derive(Debug, Clone, Deserialize)]
#[allow(clippy::enum_variant_names)]
#[serde(tag = "type")]
/// Supported repository types.
/// All repositories are fetched and pushed using git, but pull requests are submitted differently.
pub enum RepoHandle {
    #[serde(rename = "github")]
    /// GitHub: fetches with ssh, submits pull requests using GitHub API.
    GitHub {
        base_url: Option<String>,
        ssh_url: Option<String>,
        token_env_var: Option<String>,
        owner: String,
        repo: String,
    },
    #[serde(rename = "gitlab")]
    /// GitLab: fetches with ssh, submits merge requests using GitLab API.
    GitLab {
        base_url: Option<String>,
        ssh_url: Option<String>,
        token_env_var: Option<String>,
        project: String,
    },
    #[serde(rename = "git+none")]
    /// Pure git with **no pull request support**.
    /// Useful for debugging.
    GitNone { url: String },
}

#[derive(Debug, Clone, Deserialize)]
pub struct Repo {
    pub settings: Option<UpdateSettingsOptional>,
    #[serde(flatten)]
    pub handle: RepoHandle,
}

impl Display for RepoHandle {
    fn fmt(&self, f: &mut Formatter) -> Result<(), std::fmt::Error> {
        match self {
            RepoHandle::GitHub {
                owner,
                repo,
                ssh_url,
                ..
            } => {
                write!(
                    f,
                    "ssh://{}/{}/{}",
                    ssh_url.as_ref().unwrap_or(&"git@github.com".to_string()),
                    owner,
                    repo
                )?;
            }
            RepoHandle::GitLab {
                project, ssh_url, ..
            } => {
                write!(
                    f,
                    "ssh://{}/{}",
                    ssh_url.as_ref().unwrap_or(&"git@gitlab.com".to_string()),
                    project
                )?;
            }
            RepoHandle::GitNone { url, .. } => {
                write!(f, "{}", url)?;
            }
        };
        Ok(())
    }
}
