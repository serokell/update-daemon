// SPDX-FileCopyrightText: 2021 Serokell <https://serokell.io>
//
// SPDX-License-Identifier: MPL-2.0

use serde::Deserialize;
use std::fmt::{Display, Formatter};
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateSettings {
    pub author: Author,
    pub update_branch: String,
    pub default_branch: String,
    pub assignees: Vec<String>,
    pub title: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Author {
    pub name: String,
    pub email: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateState {
    pub cache_dir: PathBuf,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type")]
pub enum RepoHandle {
    #[serde(rename = "github")]
    GitHub { owner: String, repo: String },
}

impl Display for RepoHandle {
    fn fmt(&self, f: &mut Formatter) -> Result<(), std::fmt::Error> {
        match self {
            RepoHandle::GitHub { owner, repo } => {
                write!(f, "ssh://git@github.com/{}/{}", owner, repo)?;
            }
        };
        Ok(())
    }
}
