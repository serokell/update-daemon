// SPDX-FileCopyrightText: 2021 Serokell <https://serokell.io>
//
// SPDX-License-Identifier: MPL-2.0

use git2::RemoteCallbacks;
pub use git2::{
    BranchType, FetchOptions, ObjectType, PushOptions, Repository, ResetType, Signature, Time,
};
use std::collections::hash_map::DefaultHasher;
use std::fs::{create_dir, remove_dir_all};
use std::hash::{Hash, Hasher};
pub use std::sync::Arc;
pub use std::sync::Mutex;
use std::time::SystemTime;
use thiserror::Error;

use log::*;

use super::types::*;

/// Calculate a hash.
/// Must be identical for identical URLs and different for different URLs.
fn calculate_hash(url: String) -> String {
    let mut hasher = DefaultHasher::new();
    url.hash(&mut hasher);
    format!("{}", hasher.finish())
}

#[derive(Debug, Error)]
pub enum AuthorError {
    #[error("Error during a git operation: {0}")]
    GitError(#[from] git2::Error),
    #[error("Error during a time operation: {0}")]
    SystemTimeError(#[from] std::time::SystemTimeError),
}

/// Generate a git signature from UpdateSettings, with the current time
fn make_signature<'a>(settings: UpdateSettings) -> Result<Signature<'a>, AuthorError> {
    let now = Time::new(
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)?
            .as_secs() as i64,
        0,
    );
    let author = Signature::new(
        settings.author_name.as_str(),
        settings.author_email.as_str(),
        &now,
    )?;
    Ok(author)
}

#[derive(Debug, Error)]
pub enum InitError {
    #[error("Error during a git operation: {0}")]
    GitError(#[from] git2::Error),
    #[error("IO error: {0}")]
    IOError(#[from] std::io::Error),
    #[error("Error during creating author's signatuire: {0}")]
    AuthorError(#[from] AuthorError),
}

/// Initialize the repository:
/// If there is a repository cloned from the same URL, open it,
/// Otherwise clone it.
/// If there is a remote branch called `settings.update_branch`,
/// make a local branch with the same name from it and rebase the local branch on the default branch
/// Otherwise create a new branch called `settings.update_branch` from the default branch.
/// Finally, check out the update branch.
pub fn init_repo(
    state: UpdateState,
    settings: UpdateSettings,
    handle: RepoHandle,
) -> Result<Arc<Mutex<Repository>>, InitError> {
    let url = format!("{}", handle);
    let urlhash = calculate_hash(url.clone());
    let mut repo_dir = state.cache_dir.clone();
    let default_branch_name = settings.clone().default_branch;
    let update_branch_name = settings.clone().update_branch;
    repo_dir.push(urlhash);
    let mut callbacks = RemoteCallbacks::new();
    callbacks
        .credentials(|_url, username, _| git2::Cred::ssh_key_from_agent(username.unwrap_or("git")));
    let mut fetch_options = FetchOptions::new();
    fetch_options.remote_callbacks(callbacks);
    let repo_cell = Arc::new(Mutex::new(if repo_dir.exists() {
        debug!("Repository {} found at {:?}", handle, repo_dir);
        let repo = Repository::open(repo_dir.clone())?;
        repo.remote_set_url("origin", url.as_str())?;
        repo.find_remote("origin")?.fetch(
            &[default_branch_name.clone()],
            Some(&mut fetch_options),
            None,
        )?;
        repo
    } else {
        debug!("Cloning {} to {:?}", handle, repo_dir);
        create_dir(repo_dir.clone())?;
        let mut builder = git2::build::RepoBuilder::new();
        builder.fetch_options(fetch_options);
        match builder.clone(url.as_str(), &repo_dir.clone()) {
            Ok(repo) => repo,
            Err(e) => {
                remove_dir_all(repo_dir)?;
                return Err(e)?;
            }
        }
    }));
    {
        let repo = repo_cell.lock().unwrap();

        let default_branch_commit = repo
            .find_branch(
                format!("origin/{}", default_branch_name.clone()).as_str(),
                BranchType::Remote,
            )?
            .into_reference()
            .peel(ObjectType::Commit)?;

        repo.reset(&default_branch_commit, ResetType::Hard, None)?;
        repo.set_head(format!("refs/heads/{}", default_branch_name.clone()).as_str())?;

        let update_branch = repo.find_branch(
            format!("origin/{}", update_branch_name.clone()).as_str(),
            BranchType::Remote,
        );

        match update_branch {
            Err(_) => {
                repo.branch(
                    update_branch_name.clone().as_str(),
                    default_branch_commit.as_commit().unwrap(),
                    true,
                )?;
            }
            Ok(remote_update_branch) => {
                repo.branch(
                    update_branch_name.clone().as_str(),
                    remote_update_branch
                        .into_reference()
                        .peel(ObjectType::Commit)?
                        .as_commit()
                        .unwrap(),
                    true,
                )?;
                let local_update_branch = repo.find_branch(
                    format!("{}", update_branch_name.clone()).as_str(),
                    BranchType::Local,
                )?;
                let update_branch_commit = local_update_branch
                    .into_reference()
                    .peel(ObjectType::Commit)?;
                let update_annotated_commit =
                    repo.find_annotated_commit(update_branch_commit.id())?;
                let default_annotated_commit =
                    repo.find_annotated_commit(default_branch_commit.id())?;
                let rebase = repo.rebase(
                    Some(&update_annotated_commit),
                    None,
                    Some(&default_annotated_commit),
                    None,
                )?;
                for _ in rebase {
                    let commiter = make_signature(settings.clone())?;
                    repo.open_rebase(None)?.commit(None, &commiter, None)?;
                }
                repo.open_rebase(None)?.finish(None)?;
            }
        }
        repo.set_head(format!("refs/heads/{}", update_branch_name).as_str())?;
    }
    Ok(repo_cell)
}

#[derive(Debug, Error)]
pub enum CommitError {
    #[error("Error during a git operation: {0}")]
    GitError(#[from] git2::Error),
    #[error("Error during creating author's signatuire: {0}")]
    AuthorError(#[from] AuthorError),
}

/// Stage all changed files and add them to index.
/// `diff` is going to be the commit message.
pub fn commit(
    settings: UpdateSettings,
    repo: Arc<Mutex<Repository>>,
    diff: String,
) -> Result<(), CommitError> {
    let repo = repo.lock().unwrap();
    let mut index = repo.index()?;

    index.add_all(&["*"], git2::IndexAddOption::DEFAULT, None)?;
    index.write()?;

    let author = make_signature(settings.clone())?;
    let tree = repo.find_tree(index.write_tree()?)?;
    let parent = &repo.head()?.peel_to_commit()?;
    repo.commit(
        Some("HEAD"),
        &author,
        &author,
        format!("{}\n\n{}", settings.title, diff).as_str(),
        &tree,
        &[parent],
    )?;
    Ok(())
}

#[derive(Debug, Error)]
pub enum PushError {
    #[error("Error during a git operation: {0}")]
    GitError(#[from] git2::Error),
}

/// Push the changes to the `origin` remote.
pub fn push(settings: UpdateSettings, repo: Arc<Mutex<Repository>>) -> Result<(), PushError> {
    let repo = repo.lock().unwrap();
    let mut remote = repo.find_remote("origin")?;
    let mut callbacks = RemoteCallbacks::new();
    callbacks
        .credentials(|_url, username, _| git2::Cred::ssh_key_from_agent(username.unwrap_or("git")));
    let mut push_options = PushOptions::new();
    push_options.remote_callbacks(callbacks);
    remote.push(
        //         ↓ force-push
        &[format!("+refs/heads/{0}:refs/heads/{0}", settings.update_branch).as_str()],
        Some(&mut push_options),
    )?;
    Ok(())
}
