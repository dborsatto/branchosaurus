// SPDX-License-Identifier: GPL-3.0-or-later

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock, RwLock};
use std::time::SystemTime;

use gix::ObjectId;
use gix::bstr::ByteSlice;

use crate::cli::GitCli;
use crate::error::Context;
use crate::{Error, Result};

/// An open repository. Cheap to clone and safe to share between threads; each operation works on
/// a thread-local gitoxide handle.
#[derive(Clone)]
pub struct Repository {
    shared: Arc<Shared>,
    git: Arc<OnceLock<std::result::Result<GitCli, String>>>,
    askpass: Option<Arc<PathBuf>>,
}

struct Shared {
    git_dir: PathBuf,
    workdir: Option<PathBuf>,
    /// The repository's own config file. gitoxide reads configuration once, when it opens a
    /// repository, so the handle is reopened whenever this file changes (git commands the app
    /// runs set upstreams, add remotes…).
    config_file: PathBuf,
    isolated: bool,
    current: RwLock<Opened>,
}

struct Opened {
    repo: gix::ThreadSafeRepository,
    config_stamp: Option<(SystemTime, u64)>,
}

impl std::fmt::Debug for Repository {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Repository")
            .field("git_dir", &self.shared.git_dir)
            .finish_non_exhaustive()
    }
}

fn config_stamp(file: &Path) -> Option<(SystemTime, u64)> {
    let metadata = std::fs::metadata(file).ok()?;
    Some((metadata.modified().ok()?, metadata.len()))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepositoryInfo {
    /// Directory name of the working tree (or of the git dir for bare repositories).
    pub name: String,
    pub workdir: Option<PathBuf>,
    pub git_dir: PathBuf,
    pub head: Head,
    pub state: Option<RepositoryState>,
    pub is_shallow: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Head {
    /// HEAD points at a branch, which may not have any commits yet.
    Branch {
        name: String,
        target: Option<ObjectId>,
    },
    Detached {
        target: ObjectId,
    },
}

impl Head {
    pub fn target(&self) -> Option<ObjectId> {
        match self {
            Head::Branch { target, .. } => *target,
            Head::Detached { target } => Some(*target),
        }
    }

    pub fn branch_name(&self) -> Option<&str> {
        match self {
            Head::Branch { name, .. } => Some(name),
            Head::Detached { .. } => None,
        }
    }
}

/// An operation that was started and not finished, which the UI should surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RepositoryState {
    Merge,
    Rebase,
    CherryPick,
    Revert,
    Bisect,
    ApplyMailbox,
}

impl Repository {
    /// Opens the repository containing `path`, searching parent directories like `git` does.
    pub fn discover(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let inner =
            gix::ThreadSafeRepository::discover(path).map_err(|err| Error::NotARepository {
                path: path.to_owned(),
                source: err.into(),
            })?;
        Ok(Self::from_gix(inner, false))
    }

    /// Opens the repository at exactly `path` (a working tree or a git dir).
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let inner = gix::ThreadSafeRepository::open(path).map_err(|err| Error::NotARepository {
            path: path.to_owned(),
            source: err.into(),
        })?;
        Ok(Self::from_gix(inner, false))
    }

    /// Opens the repository ignoring system and user configuration and environment variables,
    /// so tests see only the repository's own configuration.
    #[doc(hidden)]
    pub fn open_isolated(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let inner = gix::ThreadSafeRepository::open_opts(path, gix::open::Options::isolated())
            .map_err(|err| Error::NotARepository {
                path: path.to_owned(),
                source: err.into(),
            })?;
        Ok(Self::from_gix(inner, true))
    }

    /// Uses `git` instead of locating one.
    pub fn with_git(mut self, git: GitCli) -> Self {
        let cell = OnceLock::new();
        let _ = cell.set(Ok(git));
        self.git = Arc::new(cell);
        self
    }

    /// Uses `program` as the credential-prompt helper instead of the `branchosaurus-askpass`
    /// next to the running executable.
    pub fn with_askpass_program(mut self, program: impl Into<PathBuf>) -> Self {
        self.askpass = Some(Arc::new(program.into()));
        self
    }

    pub(crate) fn askpass_helper(&self) -> Option<&Path> {
        self.askpass.as_deref().map(PathBuf::as_path)
    }

    fn from_gix(repo: gix::ThreadSafeRepository, isolated: bool) -> Self {
        let local = repo.to_thread_local();
        let config_file = local.common_dir().join("config");
        let shared = Shared {
            git_dir: repo.git_dir().to_owned(),
            workdir: repo.work_dir().map(Path::to_owned),
            config_file: config_file.clone(),
            isolated,
            current: RwLock::new(Opened {
                repo,
                config_stamp: config_stamp(&config_file),
            }),
        };
        Self {
            shared: Arc::new(shared),
            git: Arc::default(),
            askpass: crate::askpass::default_helper().map(Arc::new),
        }
    }

    /// A gitoxide handle for this thread, with up-to-date configuration.
    pub(crate) fn local(&self) -> gix::Repository {
        let stamp = config_stamp(&self.shared.config_file);
        {
            let opened = self
                .shared
                .current
                .read()
                .expect("repository lock poisoned");
            if opened.config_stamp == stamp {
                return opened.repo.to_thread_local();
            }
        }
        let mut opened = self
            .shared
            .current
            .write()
            .expect("repository lock poisoned");
        if opened.config_stamp != stamp {
            let options = if self.shared.isolated {
                gix::open::Options::isolated()
            } else {
                gix::open::Options::default()
            };
            let path = self.shared.workdir.as_ref().unwrap_or(&self.shared.git_dir);
            if let Ok(repo) = gix::ThreadSafeRepository::open_opts(path, options) {
                opened.repo = repo;
            }
            opened.config_stamp = stamp;
        }
        opened.repo.to_thread_local()
    }

    pub fn git_dir(&self) -> &Path {
        &self.shared.git_dir
    }

    pub fn workdir(&self) -> Option<&Path> {
        self.shared.workdir.as_deref()
    }

    pub(crate) fn require_workdir(&self) -> Result<&Path> {
        self.workdir().ok_or(Error::BareRepository)
    }

    /// The git executable, located on first use.
    pub fn git(&self) -> Result<&GitCli> {
        self.git
            .get_or_init(|| GitCli::locate().map_err(|err| err.to_string()))
            .as_ref()
            .map_err(|message| Error::GitUnavailable(message.clone()))
    }

    /// Runs git in the working tree (or the git dir of a bare repository).
    pub(crate) fn run_git<I, S>(&self, args: I, stdin: Option<&[u8]>) -> Result<Vec<u8>>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<std::ffi::OsStr>,
    {
        let dir = self.workdir().unwrap_or_else(|| self.git_dir());
        self.git()?.run_with_stdin(dir, args, stdin)
    }

    pub fn info(&self) -> Result<RepositoryInfo> {
        let repo = self.local();
        let workdir = repo.workdir().map(Path::to_owned);
        let name_source = workdir.as_deref().unwrap_or_else(|| repo.git_dir());
        let name = name_source
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| name_source.display().to_string());
        Ok(RepositoryInfo {
            name,
            workdir,
            git_dir: repo.git_dir().to_owned(),
            head: head(&repo)?,
            state: repo.state().map(Into::into),
            is_shallow: repo.is_shallow().unwrap_or(false),
        })
    }

    /// Whether `status` must go through `git status`, because the repository uses a feature
    /// gitoxide does not implement (fsmonitor, sparse index, split index).
    pub(crate) fn needs_cli_status(&self) -> bool {
        let repo = self.local();
        let config = repo.config_snapshot();
        let fsmonitor = config
            .string("core.fsmonitor")
            .is_some_and(|value| !matches!(value.to_str(), Ok("false" | "0" | "no" | "off" | "")));
        fsmonitor
            || config.boolean("index.sparse").unwrap_or(false)
            || config.boolean("core.splitIndex").unwrap_or(false)
            || repo.git_dir().join("sharedindex").exists()
    }
}

pub(crate) fn head(repo: &gix::Repository) -> Result<Head> {
    let head = repo.head().context("reading HEAD")?;
    let target = head.id().map(|id| id.detach());
    Ok(match head.referent_name() {
        Some(name) => Head::Branch {
            name: name.shorten().to_str_lossy().into_owned(),
            target,
        },
        None => Head::Detached {
            target: target.ok_or_else(|| Error::NotFound("HEAD commit".into()))?,
        },
    })
}

impl From<gix::state::InProgress> for RepositoryState {
    fn from(value: gix::state::InProgress) -> Self {
        use gix::state::InProgress;
        match value {
            InProgress::ApplyMailbox | InProgress::ApplyMailboxRebase => Self::ApplyMailbox,
            InProgress::Bisect => Self::Bisect,
            InProgress::CherryPick | InProgress::CherryPickSequence => Self::CherryPick,
            InProgress::Merge => Self::Merge,
            InProgress::Rebase | InProgress::RebaseInteractive => Self::Rebase,
            InProgress::Revert | InProgress::RevertSequence => Self::Revert,
        }
    }
}

/// Parses a full hexadecimal object id.
pub fn parse_object_id(hex: &str) -> Result<ObjectId> {
    ObjectId::from_hex(hex.as_bytes()).map_err(|_| Error::InvalidObjectId(hex.to_owned()))
}
