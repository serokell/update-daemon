// SPDX-FileCopyrightText: 2021 Serokell <https://serokell.io>
//
// SPDX-License-Identifier: MPL-2.0

use git2::{BranchType, FetchOptions, PushOptions, Repository, ResetType, Signature};
use git2::{Rebase, RemoteCallbacks};
use std::collections::hash_map::DefaultHasher;
use std::fs::{create_dir, remove_dir_all};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use thiserror::Error;

use log::*;

use super::types::*;

/// Calculate a hash.
/// Must be identical for identical URLs and different for different URLs.
fn calculate_hash<H: Hash>(url: H) -> String {
    let mut hasher = DefaultHasher::new();
    url.hash(&mut hasher);
    format!("{}", hasher.finish())
}

pub struct UDRepo {
    repo: Repository,
}

impl UDRepo {
    pub fn init(
        state: UpdateState,
        settings: &UpdateSettings,
        handle: &RepoHandle,
    ) -> Result<UDRepo, InitError> {
        Ok(UDRepo {
            repo: init_repo(state, settings, handle)?,
        })
    }

    pub fn path(&self) -> Option<&Path> {
        self.repo.workdir()
    }

    pub fn setup_update_branch(
        &self,
        settings: &UpdateSettings,
    ) -> Result<(), SetupUpdateBranchError> {
        Ok(setup_update_branch(settings, &self.repo)?)
    }

    pub fn commit(&self, settings: &UpdateSettings, diff: String) -> Result<(), CommitError> {
        Ok(commit(settings, &self.repo, diff)?)
    }

    pub fn push(&self, settings: &UpdateSettings) -> Result<(), PushError> {
        Ok(push(settings, &self.repo)?)
    }
}

#[derive(Debug, Error)]
pub enum InitError {
    #[error("Error in git opening existing repository: {0}")]
    OpenRepository(git2::Error),
    #[error("Error in git setting remote URL for existing repository: {0}")]
    SetRemoteUrl(git2::Error),
    #[error("Error finding remote for existing repository: {0}")]
    FindRemote(git2::Error),
    #[error("Error fetching default branch for existing repository: {0}")]
    FetchDefault(git2::Error),
    #[error("Error fetching update branch for existing repository: {0}")]
    FetchUpdate(git2::Error),
    #[error("Error creating directory for cloning: {0}")]
    CreateCloneDir(std::io::Error),
    #[error("Error cleaning up after failed clone: {0}")]
    CleanFailedClone(std::io::Error),
    #[error("Error cloning repository: {0}")]
    Clone(git2::Error),
    #[error("Error finding default branch on repository: {0}")]
    FindDefaultBranch(git2::Error),
    #[error("Error peeling to default branch commit: {0}")]
    PeelDefaultBranchCommit(git2::Error),
    #[error("Error setting HEAD the default branch: {0}")]
    SetHeadToDefaultBranch(git2::Error),
    #[error("Error resetting to default branch commit: {0}")]
    ResetToDefaultBranchCommit(git2::Error),
    #[error("Error force-resetting default branch to upstream default branch: {0}")]
    SetDefaultBranch(git2::Error),
}

/// Initialize the repository:
/// If there is a repository cloned from the same URL, open it,
/// Otherwise clone it.
/// Reset the local default branch to the upstream one.
pub fn init_repo(
    state: UpdateState,
    settings: &UpdateSettings,
    handle: &RepoHandle,
) -> Result<Repository, InitError> {
    let url = handle.to_string();
    let urlhash = calculate_hash(&url);
    let mut repo_dir = PathBuf::from(state.cache_dir);
    repo_dir.push(urlhash);

    let mut callbacks = RemoteCallbacks::new();
    callbacks
        .credentials(|_url, username, _| git2::Cred::ssh_key_from_agent(username.unwrap_or("git")));

    let mut fetch_options = FetchOptions::new();
    fetch_options.remote_callbacks(callbacks);

    let repo = if repo_dir.exists() {
        debug!("Repository {} found at {:?}", handle, repo_dir);

        let repo = Repository::open(repo_dir).map_err(InitError::OpenRepository)?;

        repo.remote_set_url("origin", &url)
            .map_err(InitError::SetRemoteUrl)?;

        repo.find_remote("origin")
            .map_err(InitError::FindRemote)?
            .fetch(&[&settings.default_branch], Some(&mut fetch_options), None)
            .map_err(InitError::FetchDefault)?;

        repo.find_remote("origin")
            .map_err(InitError::FindRemote)?
            .fetch(&[&settings.update_branch], Some(&mut fetch_options), None)
            .map_err(InitError::FetchUpdate)?;

        repo
    } else {
        debug!("Cloning {} to {:?}", handle, repo_dir);

        create_dir(&repo_dir).map_err(InitError::CreateCloneDir)?;

        let mut builder = git2::build::RepoBuilder::new();
        builder.fetch_options(fetch_options);
        match builder.clone(&url, &repo_dir) {
            Ok(repo) => repo,
            Err(e) => {
                remove_dir_all(repo_dir).map_err(InitError::CleanFailedClone)?;
                return Err(InitError::Clone(e));
            }
        }
    };

    {
        let default_branch_commit = repo
            .find_branch(
                &format!("origin/{}", &settings.default_branch),
                BranchType::Remote,
            )
            .map_err(InitError::FindDefaultBranch)?
            .into_reference()
            .peel_to_commit()
            .map_err(InitError::PeelDefaultBranchCommit)?;

        repo.reset(default_branch_commit.as_object(), ResetType::Hard, None)
            .map_err(InitError::ResetToDefaultBranchCommit)?;

        repo.set_head_detached(default_branch_commit.id())
            .map_err(InitError::SetHeadToDefaultBranch)?;

        repo.branch(&settings.default_branch, &default_branch_commit, true)
            .map_err(InitError::SetDefaultBranch)?;

        repo.set_head(format!("refs/heads/{}", settings.default_branch).as_str())
            .map_err(InitError::SetHeadToDefaultBranch)?;
    }

    Ok(repo)
}

#[derive(Debug, Error)]
pub enum SetupUpdateBranchError {
    #[error("Error during a git operation: {0}")]
    GitError(#[from] git2::Error),
    #[error("Error finding default branch on repository: {0}")]
    FindDefaultBranch(git2::Error),
    #[error("Error peeling to default branch commit: {0}")]
    PeelDefaultBranchCommit(git2::Error),
    #[error("Error creating a new branch pointing to default branch commit: {0}")]
    BranchToUpdateBranchWithDefault(git2::Error),
    #[error("Error setting head to update branch: {0}")]
    SetUpdateBranchHead(git2::Error),
    #[error("Error setting head to default branch: {0}")]
    SetDefaultBranchHead(git2::Error),
    #[error("Error resetting to default branch commit: {0}")]
    ResetToDefaultBranchCommit(git2::Error),
    #[error("Error initializing rebase: {0}")]
    InitializeRebase(git2::Error),
    #[error("Error peeling to local update branch commit: {0}")]
    PeelLocalUpdateBranchCommit(git2::Error),
    #[error("Error finding annotated commit for update branch commit: {0}")]
    FindAnnotatedUpdateBranchCommit(git2::Error),
    #[error("Error creating signature for rebase commits: {0}")]
    SigningForRebaseCommits(git2::Error),
    #[error("Error finding annotated commit for default branch commit: {0}")]
    FindAnnotatedDefaultBranchCommit(git2::Error),
    #[error("Error getting next rebase patch: {0}")]
    RebaseNext(git2::Error),
    #[error("Error committing rebase patch: {0}")]
    RebaseCommit(git2::Error),
    #[error("Error finishing rebase: {0}")]
    FinishRebase(git2::Error),
    #[error("Error getting HEAD: {0}")]
    GetHead(git2::Error),
    #[error("Error peeling HEAD to commit: {0}")]
    PeelHead(git2::Error),
    #[error("Error creating a new branch pointing to remote update branch: {0}")]
    BranchToUpdateBranchWithRemoteBranch(git2::Error),
}

fn safe_abort(rebase: &mut Rebase) {
    match rebase.abort() {
        Err(e) => error!("Rebase abort failed: {}", e),
        _ => {}
    }
}

pub fn setup_update_branch(
    settings: &UpdateSettings,
    repo: &Repository,
) -> Result<(), SetupUpdateBranchError> {
    let update_branch = repo.find_branch(
        &format!("origin/{}", settings.update_branch),
        BranchType::Remote,
    );

    let default_branch_commit = repo
        .find_branch(
            &format!("origin/{}", &settings.default_branch),
            BranchType::Remote,
        )
        .map_err(SetupUpdateBranchError::FindDefaultBranch)?
        .into_reference()
        .peel_to_commit()
        .map_err(SetupUpdateBranchError::PeelDefaultBranchCommit)?;

    match update_branch {
        Err(_) => {
            // no update branch exists, creating new one from default
            // TODO: handle errors we care about here?
            repo.branch(&settings.update_branch, &default_branch_commit, true)
                .map_err(SetupUpdateBranchError::BranchToUpdateBranchWithDefault)?;
        }
        Ok(remote_update_branch) => {
            // update branch exists, we should try to:
            // 1. rebase update branch on top of default

            let update_branch_commit = &remote_update_branch
                .into_reference()
                .peel_to_commit()
                .map_err(SetupUpdateBranchError::PeelLocalUpdateBranchCommit)?;

            let update_annotated_commit = repo
                .find_annotated_commit(update_branch_commit.id())
                .map_err(SetupUpdateBranchError::FindAnnotatedUpdateBranchCommit)?;

            let default_annotated_commit =
                repo.find_annotated_commit(default_branch_commit.id())
                    .map_err(SetupUpdateBranchError::FindAnnotatedDefaultBranchCommit)?;

            let mut rebase = repo
                .rebase(
                    Some(&update_annotated_commit),
                    None,
                    Some(&default_annotated_commit),
                    None,
                )
                .map_err(SetupUpdateBranchError::InitializeRebase)?;

            let committer = Signature::now(&settings.author.name, &settings.author.email)
                .map_err(SetupUpdateBranchError::SigningForRebaseCommits)?;

            let checkout_to_default = || -> Result<(), SetupUpdateBranchError> {
                repo.set_head(&format!("refs/heads/{}", settings.default_branch))
                    .map_err(SetupUpdateBranchError::SetDefaultBranchHead)?;

                repo.reset(default_branch_commit.as_object(), ResetType::Hard, None)
                    .map_err(SetupUpdateBranchError::ResetToDefaultBranchCommit)?;
                Ok(())
            };

            while let Some(op) = rebase.next() {
                match op {
                    Ok(_) => {}
                    Err(e) => {
                        safe_abort(&mut rebase);
                        checkout_to_default()?;
                        return Err(SetupUpdateBranchError::RebaseNext(e));
                    }
                }

                match rebase.commit(None, &committer, None) {
                    Ok(_) => {}
                    Err(e) => {
                        safe_abort(&mut rebase);
                        checkout_to_default()?;
                        return Err(SetupUpdateBranchError::RebaseCommit(e));
                    }
                }
            }

            match rebase.finish(None) {
                Ok(_) => {}
                Err(e) => {
                    checkout_to_default()?;
                    return Err(SetupUpdateBranchError::FinishRebase(e));
                }
            }

            let head = repo.head().map_err(SetupUpdateBranchError::GetHead)?;

            repo.branch(
                &settings.update_branch,
                &head
                    .peel_to_commit()
                    .map_err(SetupUpdateBranchError::PeelHead)?,
                true,
            )
            .map_err(SetupUpdateBranchError::BranchToUpdateBranchWithRemoteBranch)?;
        }
    };

    repo.set_head(&format!("refs/heads/{}", settings.update_branch))
        .map_err(SetupUpdateBranchError::SetUpdateBranchHead)?;

    Ok(())
}

#[derive(Debug, Error)]
pub enum CommitError {
    #[error("Error getting index file: {0}")]
    Index(git2::Error),
    #[error("Error adding files to index: {0}")]
    IndexAdd(git2::Error),
    #[error("Error writing index file: {0}")]
    IndexWrite(git2::Error),
    #[error("Error creating signature for commit: {0}")]
    Signature(git2::Error),
    #[error("Error writing index as tree: {0}")]
    WriteTree(git2::Error),
    #[error("Error finding tree: {0}")]
    FindTree(git2::Error),
    #[error("Error retrieving head: {0}")]
    Head(git2::Error),
    #[error("Error peeling head to commit: {0}")]
    PeelHead(git2::Error),
    #[error("Error creating new commit: {0}")]
    Commit(git2::Error),
}

/// Stage all changed files and add them to index.
/// `diff` is going to be the commit message.
pub fn commit(
    settings: &UpdateSettings,
    repo: &Repository,
    diff: String,
) -> Result<(), CommitError> {
    let mut index = repo.index().map_err(CommitError::Index)?;

    index
        .add_all(&["*"], git2::IndexAddOption::DEFAULT, None)
        .map_err(CommitError::IndexAdd)?;
    index.write().map_err(CommitError::IndexWrite)?;

    let author = Signature::now(&settings.author.name, &settings.author.email)
        .map_err(CommitError::Signature)?;

    let tree = repo
        .find_tree(index.write_tree().map_err(CommitError::WriteTree)?)
        .map_err(CommitError::FindTree)?;

    let parent = &repo
        .head()
        .map_err(CommitError::Head)?
        .peel_to_commit()
        .map_err(CommitError::PeelHead)?;

    repo.commit(
        Some("HEAD"),
        &author,
        &author,
        &format!("{}\n\n{}", settings.title, diff),
        &tree,
        &[parent],
    )
    .map_err(CommitError::Commit)?;

    Ok(())
}

#[derive(Debug, Error)]
pub enum PushError {
    #[error("Error finding remote for existing repository: {0}")]
    FindRemote(git2::Error),
    #[error("Error pushing to remote: {0}")]
    Push(git2::Error),
}

/// Push the changes to the `origin` remote.
pub fn push(settings: &UpdateSettings, repo: &Repository) -> Result<(), PushError> {
    let mut remote = repo.find_remote("origin").map_err(PushError::FindRemote)?;

    let mut callbacks = RemoteCallbacks::new();
    callbacks
        .credentials(|_url, username, _| git2::Cred::ssh_key_from_agent(username.unwrap_or("git")));

    let mut push_options = PushOptions::new();
    push_options.remote_callbacks(callbacks);
    remote
        .push(
            //         ↓ force-push
            &[&format!(
                "+refs/heads/{0}:refs/heads/{0}",
                settings.update_branch
            )],
            Some(&mut push_options),
        )
        .map_err(PushError::Push)?;

    Ok(())
}
