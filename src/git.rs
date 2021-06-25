// SPDX-FileCopyrightText: 2021 Serokell <https://serokell.io>
//
// SPDX-License-Identifier: MPL-2.0

use git2::RemoteCallbacks;
pub use git2::{
    BranchType, FetchOptions, ObjectType, PushOptions, Repository, ResetType, Signature,
};
use std::collections::hash_map::DefaultHasher;
use std::fs::{create_dir, remove_dir_all};
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
pub use std::sync::Arc;
pub use std::sync::Mutex;
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

#[derive(Debug, Error)]
pub enum InitError {
    #[error("Error in git opening existing repository: {0}")]
    OpenRepository(git2::Error),
    #[error("Error in git setting remote URL for existing repository: {0}")]
    SetRemoteUrl(git2::Error),
    #[error("Error finding remote for existing repository: {0}")]
    FindRemote(git2::Error),
    #[error("Error fetching for existing repository: {0}")]
    Fetch(git2::Error),
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
    #[error("Error resetting to default branch commit: {0}")]
    ResetToDefaultBranchCommit(git2::Error),
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
    let url = handle.to_string();
    let urlhash = calculate_hash(&url);
    let mut repo_dir = PathBuf::from(state.cache_dir);
    repo_dir.push(urlhash);

    let default_branch_name = settings.default_branch;
    let update_branch_name = settings.update_branch;

    let mut callbacks = RemoteCallbacks::new();
    callbacks
        .credentials(|_url, username, _| git2::Cred::ssh_key_from_agent(username.unwrap_or("git")));

    let mut fetch_options = FetchOptions::new();
    fetch_options.remote_callbacks(callbacks);

    let repo_cell = Arc::new(Mutex::new(if repo_dir.exists() {
        debug!("Repository {} found at {:?}", handle, repo_dir);

        let repo = Repository::open(repo_dir).map_err(InitError::OpenRepository)?;

        repo.remote_set_url("origin", url.as_str())
            .map_err(InitError::SetRemoteUrl)?;

        repo.find_remote("origin")
            .map_err(InitError::FindRemote)?
            .fetch(&[&default_branch_name], Some(&mut fetch_options), None)
            .map_err(InitError::Fetch)?;


        repo.find_remote("origin")
            .map_err(InitError::FindRemote)?
            .fetch(&[&update_branch_name], Some(&mut fetch_options), None)
            .map_err(InitError::Fetch)?;


        repo
    } else {
        debug!("Cloning {} to {:?}", handle, repo_dir);

        create_dir(&repo_dir).map_err(InitError::CreateCloneDir)?;

        let mut builder = git2::build::RepoBuilder::new();
        builder.fetch_options(fetch_options);
        match builder.clone(url.as_str(), &repo_dir) {
            Ok(repo) => repo,
            Err(e) => {
                remove_dir_all(repo_dir).map_err(InitError::CleanFailedClone)?;
                return Err(InitError::Clone(e));
            }
        }
    }));

    {
        let repo = repo_cell.lock().unwrap();

        let default_branch_commit = repo
            .find_branch(
                format!("origin/{}", &default_branch_name).as_str(),
                BranchType::Remote,
            )
            .map_err(InitError::FindDefaultBranch)?
            .into_reference()
            .peel_to_commit()
            .map_err(InitError::PeelDefaultBranchCommit)?;

        repo.reset(default_branch_commit.as_object(), ResetType::Hard, None)
            .map_err(InitError::ResetToDefaultBranchCommit)?;
    }

    Ok(repo_cell)
}

#[derive(Debug, Error)]
pub enum SetupUpdateBranchError {
    #[error("Error during a git operation: {0}")]
    GitError(#[from] git2::Error),
    #[error("Error finding default branch on repository: {0}")]
    FindDefaultBranch(git2::Error),
    #[error("Error finding update branch on repository: {0}")]
    FindUpdateBranch(git2::Error),
    #[error("Error peeling to default branch commit: {0}")]
    PeelDefaultBranchCommit(git2::Error),
    #[error("Error creating a new branch pointing to default branch commit: {0}")]
    BranchToUpdateBranchWithDefault(git2::Error),
    #[error("Error creating a new branch pointing to remote update branch: {0}")]
    BranchToUpdateBranchWithRemoteBranch(git2::Error),
    #[error("Error initializing rebase: {0}")]
    InitializeRebase(git2::Error),
    #[error("Error creating signature for rebase commits: {0}")]
    SigningForRebaseCommits(git2::Error),
    #[error("Error opening current rebase: {0}")]
    OpenRebase(git2::Error),
    #[error("Error committing rebase patch: {0}")]
    RebaseCommit(git2::Error),
    #[error("Error finishing rebase: {0}")]
    FinishRebase(git2::Error),
    #[error("Error setting head to update branch: {0}")]
    SetUpdateBranchHead(git2::Error),
    #[error("Error peeling to remote update branch commit: {0}")]
    PeelRemoteUpdateBranchCommit(git2::Error),
    #[error("Error peeling to local update branch commit: {0}")]
    PeelLocalUpdateBranchCommit(git2::Error),
    #[error("Error finding annotated commit for update branch commit: {0}")]
    FindAnnotatedUpdateBranchCommit(git2::Error),
    #[error("Error finding annotated commit for default branch commit: {0}")]
    FindAnnotatedDefaultBranchCommit(git2::Error),
}

pub fn setup_update_branch(
    settings: UpdateSettings,
    repo_cell: Arc<Mutex<Repository>>,
) -> Result<(), SetupUpdateBranchError> {
    let repo = repo_cell.lock().unwrap();
    let default_branch_name = settings.default_branch;
    let update_branch_name = settings.update_branch;
    let update_branch = repo.find_branch(
        format!("origin/{}", update_branch_name.clone()).as_str(),
        BranchType::Remote,
    );
    let default_branch_commit = repo
        .find_branch(
            format!("origin/{}", &default_branch_name).as_str(),
            BranchType::Remote,
        )
        .map_err(SetupUpdateBranchError::FindDefaultBranch)?
        .into_reference()
        .peel_to_commit()
        .map_err(SetupUpdateBranchError::PeelDefaultBranchCommit)?;

    match update_branch {
        Err(_) => {
            // TODO: handle errors we care about here?
            repo.branch(&update_branch_name, &default_branch_commit, true)
                .map_err(SetupUpdateBranchError::BranchToUpdateBranchWithDefault)?;
        }
        Ok(remote_update_branch) => {
            repo.branch(
                &update_branch_name,
                &remote_update_branch
                    .into_reference()
                    .peel_to_commit()
                    .map_err(SetupUpdateBranchError::PeelRemoteUpdateBranchCommit)?,
                true,
            )
            .map_err(SetupUpdateBranchError::BranchToUpdateBranchWithRemoteBranch)?;

            let local_update_branch = repo
                .find_branch(&update_branch_name, BranchType::Local)
                .map_err(SetupUpdateBranchError::FindUpdateBranch)?;

            let update_branch_commit = local_update_branch
                .into_reference()
                .peel_to_commit()
                .map_err(SetupUpdateBranchError::PeelLocalUpdateBranchCommit)?;

            let update_annotated_commit = repo
                .find_annotated_commit(update_branch_commit.id())
                .map_err(SetupUpdateBranchError::FindAnnotatedUpdateBranchCommit)?;

            let default_annotated_commit =
                repo.find_annotated_commit(default_branch_commit.id())
                    .map_err(SetupUpdateBranchError::FindAnnotatedDefaultBranchCommit)?;

            let rebase = repo
                .rebase(
                    Some(&update_annotated_commit),
                    None,
                    Some(&default_annotated_commit),
                    None,
                )
                .map_err(SetupUpdateBranchError::InitializeRebase)?;

            let committer = Signature::now(&settings.author.name, &settings.author.email)
                .map_err(SetupUpdateBranchError::SigningForRebaseCommits)?;

            for _ in rebase {
                repo.open_rebase(None)
                    .map_err(SetupUpdateBranchError::OpenRebase)?
                    .commit(None, &committer, None)
                    .map_err(SetupUpdateBranchError::RebaseCommit)?;
            }

            repo.open_rebase(None)
                .map_err(SetupUpdateBranchError::OpenRebase)?
                .finish(None)
                .map_err(SetupUpdateBranchError::FinishRebase)?;
        }
    };
    repo.set_head(format!("refs/heads/{}", update_branch_name).as_str())
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
    settings: UpdateSettings,
    repo: Arc<Mutex<Repository>>,
    diff: String,
) -> Result<(), CommitError> {
    let repo = repo.lock().unwrap();
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
        format!("{}\n\n{}", settings.title, diff).as_str(),
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
pub fn push(settings: UpdateSettings, repo: Arc<Mutex<Repository>>) -> Result<(), PushError> {
    let repo = repo.lock().unwrap();

    let mut remote = repo.find_remote("origin").map_err(PushError::FindRemote)?;

    let mut callbacks = RemoteCallbacks::new();
    callbacks
        .credentials(|_url, username, _| git2::Cred::ssh_key_from_agent(username.unwrap_or("git")));

    let mut push_options = PushOptions::new();
    push_options.remote_callbacks(callbacks);
    remote
        .push(
            //         ↓ force-push
            &[format!("+refs/heads/{0}:refs/heads/{0}", settings.update_branch).as_str()],
            Some(&mut push_options),
        )
        .map_err(PushError::Push)?;

    Ok(())
}
