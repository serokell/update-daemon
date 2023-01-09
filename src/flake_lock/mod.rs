// SPDX-FileCopyrightText: 2021 Serokell <https://serokell.io>
//
// SPDX-License-Identifier: MPL-2.0

use indexmap::map::IndexMap;
use serde::{Deserialize, Serialize};
use std::fmt::{Display, Formatter};
use std::str::FromStr;
use thiserror::Error;

#[cfg(test)]
mod tests;

/// A structure representing the flake.lock file
#[derive(Serialize, Deserialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Lock {
    nodes: IndexMap<String, Node>,
    version: u32,
    root: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Node {
    flake: Option<bool>,
    locked: Option<Locked>,
    inputs: Option<IndexMap<String, Input>>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(untagged)]
pub enum Input {
    Simple(String),
    Follows(Vec<String>),
}

/// A structure representing the locked input
// Order is important here: Git inputs also contain the narHash but shouldn't be parsed as Other
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(untagged)]
pub enum Locked {
    #[serde(rename_all = "camelCase")]
    Git {
        r#type: String,
        owner: Option<String>,
        repo: Option<String>,
        rev: String,
        nar_hash: String,
        last_modified: Option<i64>,
    },
    #[serde(rename_all = "camelCase")]
    Other {
        nar_hash: String,
        last_modified: Option<i64>,
    },
}

impl Locked {
    fn get_hash(self) -> String {
        match self {
            Locked::Git { nar_hash, .. } => nar_hash,
            Locked::Other { nar_hash, .. } => nar_hash,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputChange {
    Add(Locked),
    Update { old: Locked, new: Locked },
    Delete,
}

#[derive(Debug, PartialEq, Eq)]
pub struct LockDiff(IndexMap<String, InputChange>);

#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum LockDiffError {
    #[error("Node {0} is in the list of inputs of node {1} but not in the lockfile")]
    MissingNodeError(String, String),
    #[error("There is no root node in the lockfile")]
    MissingRootNode,
}

impl Lock {
    // Reimplemented from flake-compat
    fn resolve_input(&self, node: Input) -> Option<String> {
        match node {
            Input::Simple(locked) => Some(locked),
            Input::Follows(path) => self.get_input_by_path(self.root.clone(), path),
        }
    }

    fn get_input_by_path(&self, name: String, path: Vec<String>) -> Option<String> {
        let mut name = name;
        for input in path {
            name =
                self.resolve_input(self.nodes.get(&name)?.clone().inputs?.get(&input)?.clone())?;
        }
        Some(name)
    }

    fn root_deps(&self) -> Option<IndexMap<String, Input>> {
        Some(
            self.nodes
                .get(&self.root.clone())?
                .clone()
                .inputs
                .unwrap_or_default()
        )
    }

    fn get_dep(&self, dep: Input) -> Option<Locked> {
        self.nodes.get(&self.resolve_input(dep)?)?.locked.clone()
    }

    fn get_root_dep(&self, name: String) -> Option<Locked> {
        self.get_dep(self.root_deps()?.get(&name)?.clone())
    }

    pub fn diff(&self, new: &Self) -> Result<LockDiff, LockDiffError> {
        let mut diff: IndexMap<String, InputChange> = IndexMap::new();

        for (key, input_a) in new.root_deps().ok_or(LockDiffError::MissingRootNode)? {
            let value_a = new.get_dep(input_a).ok_or_else(|| LockDiffError::MissingNodeError(
                key.clone(),
                "root".to_string(),
            ))?;

            match self.get_root_dep(key.clone()) {
                Some(value_b) => {
                    if value_a.clone().get_hash() != value_b.clone().get_hash() {
                        diff.insert(
                            key,
                            InputChange::Update {
                                old: value_b,
                                new: value_a,
                            },
                        );
                    }
                }
                None => {
                    diff.insert(key, InputChange::Add(value_a));
                }
            }
        }
        for (key, _) in self.root_deps().ok_or(LockDiffError::MissingRootNode)? {
            if !new.nodes.contains_key(&key) {
                diff.insert(key.clone(), InputChange::Delete);
            }
        }
        Ok(LockDiff(diff))
    }
}

impl FromStr for Lock {
    type Err = serde_json::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        serde_json::from_str(s)
    }
}

#[derive(Debug, Error)]
pub enum GetLockError {
    #[error("Error during reading the flake.lock file: {0}")]
    IOError(#[from] std::io::Error),
    #[error("Failed to parse flake.lock: {0}")]
    ParseError(#[from] serde_json::Error),
}

pub fn get_lock(repo: &std::path::Path) -> Result<Lock, GetLockError> {
    let mut repo = repo.to_path_buf();
    repo.push("flake.lock");
    Ok(serde_json::from_str(
        std::fs::read_to_string(repo)?.as_str(),
    )?)
}

impl LockDiff {
    pub fn len(&self) -> usize {
        self.0.len()
    }
}

fn format_date(date: i64) -> String {
    let naive = chrono::NaiveDateTime::from_timestamp_opt(date, 0).unwrap();

    let datetime: chrono::DateTime<chrono::Utc> = chrono::DateTime::from_utc(naive, chrono::Utc);

    datetime.format("%Y-%m-%d").to_string()
}

fn show_hash_and_date(
    f: &mut Formatter,
    hash: &str,
    last_modified: &Option<i64>,
) -> Result<(), std::fmt::Error> {
    match last_modified {
        Some(last_modified) => write!(
            f,
            "{} ({})",
            hash.get(..10).unwrap(),
            format_date(*last_modified)
        )?,
        None => write!(f, "{}", hash.get(..10).unwrap())?,
    }
    Ok(())
}

impl Display for Locked {
    fn fmt(&self, f: &mut Formatter) -> Result<(), std::fmt::Error> {
        match self {
            Locked::Git {
                rev, last_modified, ..
            } => show_hash_and_date(f, rev, last_modified)?,
            Locked::Other {
                nar_hash,
                last_modified,
            } => show_hash_and_date(f, nar_hash, last_modified)?,
        };
        Ok(())
    }
}

impl InputChange {
    fn link(&self) -> Option<String> {
        match self {
            InputChange::Update {
                old:
                    Locked::Git {
                        r#type: type_old,
                        owner: Some(owner_old),
                        repo: Some(repo_old),
                        rev: rev_old,
                        ..
                    },
                new:
                    Locked::Git {
                        r#type: type_new,
                        owner: Some(owner_new),
                        repo: Some(repo_new),
                        rev: rev_new,
                        ..
                    },
            } if type_new == type_old
                && owner_new.to_lowercase() == owner_old.to_lowercase()
                && repo_new.to_lowercase() == repo_old.to_lowercase() =>
            {
                match type_new.as_str() {
                    "github" => Some(format!(
                        "https://github.com/{}/{}/compare/{}...{}?expand=1",
                        owner_new, repo_new, rev_old, rev_new
                    )),
                    "gitlab" => Some(format!(
                        "https://gitlab.com/{}/{}/compare/{}...{}",
                        owner_new, repo_new, rev_old, rev_new
                    )),
                    _ => None,
                }
            }

            InputChange::Add(Locked::Git {
                r#type,
                owner: Some(owner),
                repo: Some(repo),
                rev,
                ..
            }) => match r#type.as_str() {
                "github" => Some(format!(
                    "https://github.com/{}/{}/tree/{}",
                    owner, repo, rev
                )),
                "gitlab" => Some(format!(
                    "https://gitlab.com/{}/{}/-/tree/{}",
                    owner, repo, rev
                )),
                _ => None,
            },
            _ => None,
        }
    }

    pub fn markdown(&self) -> String {
        let change = match self.clone() {
            InputChange::Add(l) => format!("(new) | `{}`", l),
            InputChange::Update { old, new } => format!("`{}` | `{}`", old, new),
            InputChange::Delete => "(deleted) | (deleted)".to_string(),
        };
        format!(
            "{} | {}",
            change,
            self.link()
                .map(|l| format!("[link]({})", l))
                .unwrap_or_else(|| "_none_".to_string())
        )
    }

    pub fn spaced(&self) -> String {
        match self {
            InputChange::Add(l) => format!("{:<23}    {}", "(new)", l),
            InputChange::Update { old, new } => format!("{:<23} -> {}", old, new),
            InputChange::Delete => format!("{0:<23}    {0}", "(deleted)"),
        }
    }
}

impl LockDiff {
    pub fn markdown(&self) -> String {
        let mut s = String::new();
        s.push_str("| input | old | new | diff |\n");
        s.push_str("|-------|-----|-----|------|\n");
        for (name, change) in self.0.clone() {
            s.push_str(format!("| {} | {} |\n", name, change.markdown()).as_str());
        }
        s
    }

    pub fn spaced(&self) -> String {
        let max = self
            .0
            .clone()
            .keys()
            .into_iter()
            .map(|l| l.len())
            .max()
            .unwrap_or(0);
        let mut s = String::new();
        for (name, change) in self.0.clone() {
            let mut name = name.clone();
            while name.len() < max {
                name.push(' ');
            }
            s.push_str(format!("{} {}\n", name, change.spaced()).as_str());
        }
        s
    }
}
