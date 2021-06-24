// SPDX-FileCopyrightText: 2021 Serokell <https://serokell.io>
//
// SPDX-License-Identifier: MPL-2.0

use merge::Merge;
use serde::Deserialize;
use std::fmt::{Display, Formatter};
use std::path::PathBuf;
use thiserror::Error;
use std::default::Default;

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateSettings {
    pub author: Author,
    pub update_branch: String,
    pub default_branch: String,
    pub title: String,
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
}

#[derive(Debug, Error)]
pub struct UpdateSettingsMissingField(String);

impl std::fmt::Display for UpdateSettingsMissingField {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> Result<(), std::fmt::Error> {
        write!(f, "Settings missing field {}", self.0)
    }
}

fn unoption<T>(opt: Option<T>, name: &'static str) -> Result<T, UpdateSettingsMissingField> {
    opt.ok_or(UpdateSettingsMissingField(name.to_string()))
}

impl std::convert::TryInto<UpdateSettings> for UpdateSettingsOptional {
    type Error = UpdateSettingsMissingField;

    fn try_into(self) -> Result<UpdateSettings, Self::Error> {
        Ok(UpdateSettings {
            author: unoption(self.author, "author")?,
            update_branch: unoption(self.update_branch, "update_branch")?,
            default_branch: unoption(self.default_branch, "default_branch")?,
            title: unoption(self.title, "title")?,
        })
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateState {
    pub cache_dir: PathBuf,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type")]
pub enum RepoHandle {
    #[serde(rename = "github")]
    GitHub { owner: String, repo: String, },
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
            RepoHandle::GitHub { owner, repo, .. } => {
                write!(f, "ssh://git@github.com/{}/{}", owner, repo)?;
            }
        };
        Ok(())
    }
}
