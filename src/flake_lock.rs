use serde::{Deserialize, Serialize};
use std::fmt::{Display, Formatter};
use std::{collections::HashMap, str::FromStr};
use thiserror::Error;

/// A structure representing the flake.lock file
#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Lock {
    nodes: HashMap<String, Node>,
    version: u32,
    root: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Node {
    flake: Option<bool>,
    locked: Option<Locked>,
    inputs: Option<HashMap<String, Input>>,
}


#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(untagged)]
pub enum Input {
    Simple(String),
    Follows(Vec<String>)
}

/// A structure representing the locked input
// Order is important here: Git inputs also contain the narHash but shouldn't be parsed as Other
#[derive(Serialize, Deserialize, Debug, Clone)]
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
            Locked::Git { nar_hash, ..  } => nar_hash,
            Locked::Other { nar_hash, .. } => nar_hash,
        }
    }
}

#[derive(Debug, Clone)]
pub enum InputChange {
    Add(Locked),
    Update { old: Locked, new: Locked },
    Delete,
}

#[derive(Debug)]
pub struct LockDiff(HashMap<String, InputChange>);

impl Lock {


    // Reimplemented from flake-compat
    fn resolve_input(&self, node: Input) -> String {
        match node {
            Input::Simple(locked) => locked,
            Input::Follows(path) => self.get_input_by_path(self.root.clone(), path),
        }
    }

    fn get_input_by_path(&self, name: String, path: Vec<String>) -> String {
        let mut name = name.clone();
        for input in path {
            name = self.resolve_input(self.nodes.get(&name).unwrap().clone().inputs.unwrap().get(&input).unwrap().clone());
        }
        name
    }

    fn root_deps(&self) -> HashMap<String, Input> {
        self.nodes.get(&self.root.clone()).expect("No root node in the lock").clone().inputs.expect("No inputs on root node").clone()
    }

    fn get_dep(&self, dep: Input) -> Option<Locked> {
        self.nodes.get(&self.resolve_input(dep))?.locked.clone()
    }

    fn get_root_dep(&self, name: String) -> Option<Locked> {
        self.get_dep(self.root_deps().get(&name)?.clone())
    }

    pub fn diff(self, new: &Self) -> LockDiff {
        let mut diff: HashMap<String, InputChange> = HashMap::new();

        for (key, input_a) in self.root_deps() {

            let value_a = self.get_dep(input_a).unwrap();

            match new.get_root_dep(key.clone()) {
                Some(value_b) => {
                    if value_a.clone().get_hash() != value_b.clone().get_hash() {
                        diff.insert(
                            key,
                            InputChange::Update {
                                old: value_a,
                                new: value_b,
                            },
                        );
                    }
                },
                None => {
                    diff.insert(key, InputChange::Delete);
                }
            }
        }
        for (key, _) in new.root_deps() {
            if self.get_root_dep(key.clone()).is_none() {
                diff.insert(key.clone(), InputChange::Add(new.get_root_dep(key).unwrap()));
            }
        }
        LockDiff(diff)
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
    let naive = chrono::NaiveDateTime::from_timestamp(date, 0);

    let datetime: chrono::DateTime<chrono::Utc> = chrono::DateTime::from_utc(naive, chrono::Utc);

    datetime.format("%Y-%m-%d").to_string()
}

fn show_hash_and_date(
    f: &mut Formatter,
    hash: &String,
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

struct Link(Option<String>);

impl Link {
    fn get(change: InputChange) -> Self {
        Link(match change {
            InputChange::Update {
                old: Locked::Git {
                    r#type: type_old,
                    owner: Some(owner_old),
                    repo: Some(repo_old),
                    rev: rev_old,
                    ..
                },
                new: Locked::Git {
                    r#type: type_new,
                    owner: Some(owner_new),
                    repo: Some(repo_new),
                    rev: rev_new,
                    ..
                },
            } => {
                // Maybe we should trust that the commit shas are the same even if type/owner/repo change?
                if type_new == type_old && owner_new == owner_old && repo_new == repo_old {
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
                } else {
                    None
                }
            },
            InputChange::Add(Locked::Git { r#type, owner: Some(owner), repo: Some(repo), rev, .. }) => {
                match r#type.as_str() {
                    "github" => Some(format!("https://github.com/{}/{}/tree/{}", owner, repo, rev)),
                    "gitlab" => Some(format!("https://gitlab.com/{}/{}/-/tree/{}", owner, repo, rev)),
                    _ => None,
                }
            },
            _ => None,
        })
    }
}

impl Display for Link {
    fn fmt(&self, f: &mut Formatter) -> Result<(), std::fmt::Error> {
        match &self.0 {
            Some(difflink) => {
                write!(f, "[link]({})", difflink)?;
            }
            None => {
                write!(f, "_none_")?;
            }
        }
        Ok(())
    }
}

impl Display for InputChange {
    fn fmt(&self, f: &mut Formatter) -> Result<(), std::fmt::Error> {
        match self.clone() {
            InputChange::Add(l) => write!(f, "| (new) | `{}` |", l)?,
            InputChange::Update { old, new } => write!(
                f,
                "| `{}` | `{}` |",
                old,
                new
            )?,
            InputChange::Delete => write!(f, "| (deleted) | (deleted) |")?,
        };
        Ok(())
    }
}

impl Display for LockDiff {
    fn fmt(&self, f: &mut Formatter) -> Result<(), std::fmt::Error> {
        writeln!(f, "| input | old | new | diff |")?;
        writeln!(f, "|-------|-----|-----|------|")?;
        for (name, change) in self.0.clone() {
            writeln!(f, "| {} {} {} |", name, change.clone(), Link::get(change))?;
        }
        Ok(())
    }
}
