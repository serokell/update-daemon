// SPDX-FileCopyrightText: 2021 Serokell <https://serokell.io>
//
// SPDX-License-Identifier: MPL-2.0

use git2::RemoteCallbacks;
use git2::{BranchType, FetchOptions, PushOptions, Repository, ResetType, Signature};
use gpgme::{Context, Protocol};
use std::collections::hash_map::DefaultHasher;
use std::fs::{create_dir, remove_dir_all};
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::str;
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
        setup_update_branch(settings, &self.repo)
    }

    pub fn commit(&self, settings: &UpdateSettings, diff: String) -> Result<(), CommitError> {
        commit(settings, &self.repo, diff)
    }

    pub fn push(&self, settings: &UpdateSettings) -> Result<(), PushError> {
        push(settings, &self.repo)
    }

    pub fn soft_reset_to_default(&self, settings: &UpdateSettings) -> Result<(), ResetError> {
        soft_reset_to_default(settings, &self.repo)
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
    #[error("Error connecting to the remote: {0}")]
    ConnectRemote(git2::Error),
    #[error("Error pruning: {0}")]
    Prune(git2::Error),
    #[error("Error disconnecting from the remote: {0}")]
    DisconnectRemote(git2::Error),
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
    #[error("Error force-checking out the default branch: {0}")]
    ForceCheckoutDefaultBranch(#[from] ForceCheckoutBranchError),
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
    let mut repo_dir = state.cache_dir;
    repo_dir.push(urlhash);

    /// RemoteCallbacks is non-cloneable but we have to use it twice, hence this
    /// function
    fn callbacks<'a>() -> git2::RemoteCallbacks<'a> {
        let mut callbacks = RemoteCallbacks::new();
        callbacks.credentials(|_url, username, _| {
            git2::Cred::ssh_key_from_agent(username.unwrap_or("git"))
        });
        callbacks
    }

    let mut fetch_options = FetchOptions::new();
    fetch_options.remote_callbacks(callbacks());

    let repo = if repo_dir.exists() {
        debug!("Repository {} found at {:?}", handle, repo_dir);

        let repo = Repository::open(repo_dir).map_err(InitError::OpenRepository)?;

        {
            repo.remote_set_url("origin", &url)
                .map_err(InitError::SetRemoteUrl)?;

            let mut remote = repo.find_remote("origin").map_err(InitError::FindRemote)?;

            remote
                .connect_auth(git2::Direction::Fetch, Some(callbacks()), None)
                .map_err(InitError::ConnectRemote)?;

            remote.prune(None).map_err(InitError::Prune)?;

            remote.disconnect().map_err(InitError::DisconnectRemote)?;

            remote
                .fetch(&[&settings.default_branch], Some(&mut fetch_options), None)
                .map_err(InitError::FetchDefault)?;

            remote
                .fetch(&[&settings.update_branch], Some(&mut fetch_options), None)
                .map_err(InitError::FetchUpdate)?;
        }

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
        let default_branch = repo
            .find_branch(
                &format!("origin/{}", settings.default_branch),
                BranchType::Remote,
            )
            .map_err(InitError::FindDefaultBranch)?;

        force_checkout_branch(&repo, &settings.default_branch, &default_branch)?;
    }

    Ok(repo)
}

#[derive(Debug, Error)]
pub enum SetupUpdateBranchError {
    #[error("Error finding default branch on repository: {0}")]
    FindDefaultBranch(git2::Error),
    #[error("Error peeling to update branch commit: {0}")]
    PeelUpdateBranchCommit(git2::Error),
    #[error("Error peeling to default branch commit: {0}")]
    PeelDefaultBranchCommit(git2::Error),
    #[error("There are human commits in the update branch")]
    HumanCommitsInUpdateBranch,
    #[error("Failed to force-checkout update branch: {0}")]
    ForceCheckoutUpdateBranch(#[from] ForceCheckoutBranchError),
    #[error("Failed to count ahead/behind for the update branch: {0}")]
    GraphAheadBehind(git2::Error),
}

pub fn setup_update_branch(
    settings: &UpdateSettings,
    repo: &Repository,
) -> Result<(), SetupUpdateBranchError> {
    let update_branch = repo.find_branch(
        &format!("origin/{}", settings.update_branch),
        BranchType::Remote,
    );

    let default_branch = repo
        .find_branch(
            &format!("origin/{}", &settings.default_branch),
            BranchType::Remote,
        )
        .map_err(SetupUpdateBranchError::FindDefaultBranch)?;

    let branch = if let Ok(b) = update_branch {
        let update_branch_commit = b
            .get()
            .peel_to_commit()
            .map_err(SetupUpdateBranchError::PeelUpdateBranchCommit)?;
        let default_branch_commit = default_branch
            .get()
            .peel_to_commit()
            .map_err(SetupUpdateBranchError::PeelDefaultBranchCommit)?;
        // NB: we need to handle the case of update branch even with default
        // branch specially, otherwise we can get spurious "human commits"
        // errors where the update branch doesn't even have commits.
        if update_branch_commit.id() != default_branch_commit.id()
            && update_branch_commit.author().email() != Some(&settings.author.email)
        {
            return Err(SetupUpdateBranchError::HumanCommitsInUpdateBranch);
        }
        let (_ahead, behind) = repo
            .graph_ahead_behind(update_branch_commit.id(), default_branch_commit.id())
            .map_err(SetupUpdateBranchError::GraphAheadBehind)?;
        if behind > 0 {
            // update branch is outdated, reset to default, as we'll have to force-push anyway
            default_branch
        } else {
            // update branch isn't outdated, so use it
            b
        }
    } else {
        default_branch
    };

    force_checkout_branch(repo, &settings.update_branch, &branch)?;

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
    #[error("Error creating commit object: {0}")]
    Buffer(git2::Error),
    #[error("Error signing commit: {0}")]
    Sign(gpgme::Error),
    #[error("Error converting Utf8: {0}")]
    Utf8(str::Utf8Error),
    #[error("Error getting secret key from gpg-agent: {0}")]
    KeyGet(gpgme::Error),
    #[error("Error adding signer key: {0}")]
    SignerAdd(gpgme::Error),
    #[error("Error updating reference: {0}")]
    ReferenceUpdate(git2::Error),
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
        .add_all(["*"], git2::IndexAddOption::DEFAULT, None)
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

    let message = format!("{}\n\n{}", settings.title, diff);

    if settings.sign_commits {
        // Create commit object
        let commit_buf = repo.commit_create_buffer(
            &author,
            &author,
            &message,
            &tree,
            &[parent],
        )
        .map_err(CommitError::Buffer)?;

        let mut ctx = Context::from_protocol(Protocol::OpenPgp)
            .map_err(CommitError::Sign)?;
        let mut outbuf = Vec::new();

        // If the configuration specifies a signing key ID or fingerprint,
        // obtain the secret key from the gpg-agent and add it to the list of signers
        if let Some(signing_key) = &settings.signing_key {
            let key = ctx.get_secret_key(signing_key).map_err(CommitError::KeyGet)?;
            ctx.add_signer(&key).map_err(CommitError::SignerAdd)?;
        };

        // Sign commit
        ctx.set_armor(true);
        ctx.sign_detached(&*commit_buf, &mut outbuf).map_err(CommitError::Sign)?;
        let out = str::from_utf8(&outbuf).map_err(CommitError::Utf8)?;

        let commit_content = str::from_utf8(&commit_buf)
            .map_err(CommitError::Utf8)?
            .to_string();

        // Create a signed commit
        let commit = repo.commit_signed(
            &commit_content,
            &out,
            None,
        )
        .map_err(CommitError::Commit)?;

        // Move HEAD to the newly created commit
        repo.reference(
            &format!("refs/heads/{}", &settings.update_branch),
            commit,
            true,
            &message,
        )
        .map_err(CommitError::ReferenceUpdate)?;

    } else {
        repo.commit(
            Some("HEAD"),
            &author,
            &author,
            &message,
            &tree,
            &[parent],
        )
        .map_err(CommitError::Commit)?;
    };

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

#[derive(Debug, Error)]
pub enum ResetError {
    #[error("Error soft-resetting update branch to default: {0}")]
    Reset(git2::Error),
    #[error("Error finding default branch on repository: {0}")]
    FindDefaultBranch(git2::Error),
    #[error("Error peeling to default branch commit: {0}")]
    PeelDefaultBranchCommit(git2::Error),
}

pub fn soft_reset_to_default(
    settings: &UpdateSettings,
    repo: &Repository,
) -> Result<(), ResetError> {
    let commit = repo
        .find_branch(
            &format!("origin/{}", &settings.default_branch),
            BranchType::Remote,
        )
        .map_err(ResetError::FindDefaultBranch)?
        .into_reference()
        .peel_to_commit()
        .map_err(ResetError::PeelDefaultBranchCommit)?;
    repo.reset(commit.as_object(), ResetType::Soft, None)
        .map_err(ResetError::Reset)?;
    Ok(())
}

#[derive(Debug, Error)]
pub enum ForceCheckoutBranchError {
    #[error("Error resetting update branch: {0}")]
    SetBranch(git2::Error),
    #[error("Error (re)setting to update branch commit: {0}")]
    ResetToBranchCommit(git2::Error),
    #[error("Error setting head to update branch: {0}")]
    SetBranchHead(git2::Error),
    #[error("Error peeling to branch commit: {0}")]
    PeelBranchCommit(git2::Error),
}

pub fn force_checkout_branch(
    repo: &git2::Repository,
    new_branch_name: &str,
    branch: &git2::Branch<'_>,
) -> Result<(), ForceCheckoutBranchError> {
    let commit = branch
        .get()
        .peel_to_commit()
        .map_err(ForceCheckoutBranchError::PeelBranchCommit)?;

    repo.reset(commit.as_object(), ResetType::Hard, None)
        .map_err(ForceCheckoutBranchError::ResetToBranchCommit)?;

    repo.set_head_detached(commit.id())
        .map_err(ForceCheckoutBranchError::SetBranchHead)?;

    repo.branch(new_branch_name, &commit, true)
        .map_err(ForceCheckoutBranchError::SetBranch)?;

    repo.set_head(&format!("refs/heads/{}", new_branch_name))
        .map_err(ForceCheckoutBranchError::SetBranchHead)?;

    Ok(())
}
