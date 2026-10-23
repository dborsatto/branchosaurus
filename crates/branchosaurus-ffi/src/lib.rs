// SPDX-License-Identifier: GPL-3.0-or-later

//! UniFFI bindings for the shells that are not written in Rust (Swift on macOS, C# on Windows).
//!
//! The types here mirror `branchosaurus-core` one to one: object ids become hex strings, paths
//! become bytes plus a display string, durations become milliseconds. Calls block; shells make
//! them off their UI thread, except [`History::rows`], which is cheap by design.

use std::sync::Arc;

use branchosaurus_core as core;

uniffi::setup_scaffolding!();

// MARK: Errors

#[derive(Debug, uniffi::Error)]
pub enum BranchosaurusError {
    NotARepository {
        message: String,
    },
    GitUnavailable {
        message: String,
    },
    GitCommandFailed {
        message: String,
    },
    NotFound {
        message: String,
    },
    BareRepository {
        message: String,
    },
    Cancelled {
        message: String,
    },
    /// The operation can't be done on this file or in this state; the message says why.
    Unsupported {
        message: String,
    },
    /// The repository changed since the data the operation refers to was read. Refresh.
    OutOfDate {
        message: String,
    },
    Internal {
        message: String,
    },
}

impl std::fmt::Display for BranchosaurusError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (Self::NotARepository { message }
        | Self::GitUnavailable { message }
        | Self::GitCommandFailed { message }
        | Self::NotFound { message }
        | Self::BareRepository { message }
        | Self::Cancelled { message }
        | Self::Unsupported { message }
        | Self::OutOfDate { message }
        | Self::Internal { message }) = self;
        f.write_str(message)
    }
}

impl std::error::Error for BranchosaurusError {}

impl From<core::Error> for BranchosaurusError {
    fn from(error: core::Error) -> Self {
        let message = error.to_string();
        match error {
            core::Error::NotARepository { .. } => Self::NotARepository { message },
            core::Error::GitUnavailable(_) => Self::GitUnavailable { message },
            core::Error::GitCommand { .. } => Self::GitCommandFailed { message },
            core::Error::NotFound(_) | core::Error::InvalidObjectId(_) => {
                Self::NotFound { message }
            }
            core::Error::BareRepository => Self::BareRepository { message },
            core::Error::Cancelled => Self::Cancelled { message },
            core::Error::Unsupported(_) => Self::Unsupported { message },
            core::Error::OutOfDate(_) => Self::OutOfDate { message },
            core::Error::Git { .. } | core::Error::Io { .. } => Self::Internal { message },
        }
    }
}

type Result<T> = std::result::Result<T, BranchosaurusError>;

/// The version of the core library, for "About" windows and bug reports.
#[uniffi::export]
pub fn core_version() -> String {
    env!("CARGO_PKG_VERSION").to_owned()
}

/// What to show for a commit's author: their initials on a color, and where their picture may
/// be. `pixels` is the picture size to ask for.
#[uniffi::export]
pub fn avatar(name: String, email: String, pixels: u32) -> Avatar {
    let avatar = core::avatar(&name, &email, pixels);
    Avatar {
        initials: avatar.initials,
        color: avatar.color,
        image_url: avatar.image_url,
    }
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct Avatar {
    /// One or two letters, in upper case.
    pub initials: String,
    /// Index into the app's palette (wrapping around), the same for every commit by this email.
    pub color: u16,
    /// GitHub for its no-reply addresses, Gravatar otherwise; Gravatar answers 404 for people
    /// without a picture. `None` without an email.
    pub image_url: Option<String>,
}

// MARK: Shared records

/// A repository-relative path. `bytes` is exact; `display` is for showing to people.
#[derive(Clone, Debug, PartialEq, Eq, Hash, uniffi::Record)]
pub struct RepoPath {
    pub bytes: Vec<u8>,
    pub display: String,
}

impl From<core::RepoPath> for RepoPath {
    fn from(path: core::RepoPath) -> Self {
        Self {
            display: path.display().into_owned(),
            bytes: path.into_bytes(),
        }
    }
}

impl From<RepoPath> for core::RepoPath {
    fn from(path: RepoPath) -> Self {
        core::RepoPath::new(path.bytes)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum ChangeKind {
    Added,
    Deleted,
    Modified,
    Renamed,
    Copied,
    TypeChanged,
    Untracked,
}

impl From<core::ChangeKind> for ChangeKind {
    fn from(kind: core::ChangeKind) -> Self {
        match kind {
            core::ChangeKind::Added => Self::Added,
            core::ChangeKind::Deleted => Self::Deleted,
            core::ChangeKind::Modified => Self::Modified,
            core::ChangeKind::Renamed => Self::Renamed,
            core::ChangeKind::Copied => Self::Copied,
            core::ChangeKind::TypeChanged => Self::TypeChanged,
            core::ChangeKind::Untracked => Self::Untracked,
        }
    }
}

impl From<ChangeKind> for core::ChangeKind {
    fn from(kind: ChangeKind) -> Self {
        match kind {
            ChangeKind::Added => Self::Added,
            ChangeKind::Deleted => Self::Deleted,
            ChangeKind::Modified => Self::Modified,
            ChangeKind::Renamed => Self::Renamed,
            ChangeKind::Copied => Self::Copied,
            ChangeKind::TypeChanged => Self::TypeChanged,
            ChangeKind::Untracked => Self::Untracked,
        }
    }
}

// MARK: Repository

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct RepositoryInfo {
    pub name: String,
    pub workdir: Option<String>,
    pub git_dir: String,
    pub head: Head,
    pub state: Option<RepositoryState>,
    pub is_shallow: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum Head {
    Branch {
        name: String,
        target: Option<String>,
    },
    Detached {
        target: String,
    },
}

impl From<core::Head> for Head {
    fn from(head: core::Head) -> Self {
        match head {
            core::Head::Branch { name, target } => Self::Branch {
                name,
                target: target.map(|id| id.to_string()),
            },
            core::Head::Detached { target } => Self::Detached {
                target: target.to_string(),
            },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum RepositoryState {
    Merge,
    Rebase,
    CherryPick,
    Revert,
    Bisect,
    ApplyMailbox,
}

impl From<core::RepositoryState> for RepositoryState {
    fn from(state: core::RepositoryState) -> Self {
        match state {
            core::RepositoryState::Merge => Self::Merge,
            core::RepositoryState::Rebase => Self::Rebase,
            core::RepositoryState::CherryPick => Self::CherryPick,
            core::RepositoryState::Revert => Self::Revert,
            core::RepositoryState::Bisect => Self::Bisect,
            core::RepositoryState::ApplyMailbox => Self::ApplyMailbox,
        }
    }
}

#[derive(Debug, uniffi::Object)]
pub struct Repository {
    inner: core::Repository,
}

#[uniffi::export]
impl Repository {
    /// Opens the repository containing `path`.
    #[uniffi::constructor]
    pub fn discover(path: String) -> Result<Arc<Self>> {
        Ok(Arc::new(Self {
            inner: core::Repository::discover(path)?,
        }))
    }

    pub fn info(&self) -> Result<RepositoryInfo> {
        let info = self.inner.info()?;
        Ok(RepositoryInfo {
            name: info.name,
            workdir: info.workdir.map(|path| path.display().to_string()),
            git_dir: info.git_dir.display().to_string(),
            head: info.head.into(),
            state: info.state.map(Into::into),
            is_shallow: info.is_shallow,
        })
    }

    pub fn refs(&self) -> Result<Refs> {
        Ok(self.inner.refs()?.into())
    }

    pub fn status(&self) -> Result<WorkingCopyStatus> {
        Ok(self.inner.status()?.into())
    }

    /// Indexes the whole history. Blocks until done; `observer` hears about progress from the
    /// calling thread, and `cancel` stops the work early with a `Cancelled` error.
    pub fn load_history(
        &self,
        observer: Box<dyn HistoryObserver>,
        cancel: Arc<CancelToken>,
    ) -> Result<Arc<History>> {
        let history = core::History::load(&self.inner, &cancel.inner, &mut |progress| {
            observer.on_progress(progress.into())
        })?;
        Ok(Arc::new(History { inner: history }))
    }

    pub fn commit_details(&self, id: String) -> Result<CommitDetails> {
        let id = core::parse_object_id(&id)?;
        Ok(self.inner.commit_details(id)?.into())
    }

    pub fn commit_file_diff(&self, change: FileChange) -> Result<FileDiff> {
        Ok(self.inner.commit_file_diff(&change.try_into()?)?.into())
    }

    pub fn staged_diff(&self, entry: StatusEntry) -> Result<FileDiff> {
        Ok(self.inner.staged_diff(&entry.into())?.into())
    }

    pub fn unstaged_diff(&self, entry: StatusEntry) -> Result<FileDiff> {
        Ok(self.inner.unstaged_diff(&entry.into())?.into())
    }

    // MARK: Staging and committing

    pub fn stage(&self, entries: Vec<StatusEntry>) -> Result<()> {
        Ok(self.inner.stage(&core_entries(entries))?)
    }

    pub fn stage_all(&self) -> Result<()> {
        Ok(self.inner.stage_all()?)
    }

    pub fn unstage(&self, entries: Vec<StatusEntry>) -> Result<()> {
        Ok(self.inner.unstage(&core_entries(entries))?)
    }

    pub fn unstage_all(&self) -> Result<()> {
        Ok(self.inner.unstage_all()?)
    }

    /// Throws away unstaged changes. Untracked files go to the trash.
    pub fn discard(&self, entries: Vec<StatusEntry>) -> Result<()> {
        Ok(self
            .inner
            .discard(&core_entries(entries), core::UntrackedRemoval::MoveToTrash)?)
    }

    pub fn stage_lines(&self, entry: StatusEntry, selection: LineSelection) -> Result<()> {
        Ok(self.inner.stage_lines(&entry.into(), &selection.into())?)
    }

    pub fn unstage_lines(&self, entry: StatusEntry, selection: LineSelection) -> Result<()> {
        Ok(self.inner.unstage_lines(&entry.into(), &selection.into())?)
    }

    pub fn discard_lines(&self, entry: StatusEntry, selection: LineSelection) -> Result<()> {
        Ok(self.inner.discard_lines(&entry.into(), &selection.into())?)
    }

    /// Commits what is staged (or replaces the last commit). Returns the new commit's id.
    pub fn commit(&self, message: String, amend: bool) -> Result<String> {
        Ok(self.inner.commit(&message, amend)?.to_string())
    }

    // MARK: Branches

    pub fn checkout(&self, target: CheckoutTarget) -> Result<()> {
        let target = match target {
            CheckoutTarget::LocalBranch { name } => core::CheckoutTarget::LocalBranch(name),
            CheckoutTarget::RemoteBranch { name } => core::CheckoutTarget::RemoteBranch(name),
            CheckoutTarget::Commit { id } => {
                core::CheckoutTarget::Commit(core::parse_object_id(&id)?)
            }
        };
        Ok(self.inner.checkout(&target)?)
    }

    /// Creates a branch at `start` (a commit id), or at HEAD.
    pub fn create_branch(&self, name: String, start: Option<String>, checkout: bool) -> Result<()> {
        let start = start.map(|id| core::parse_object_id(&id)).transpose()?;
        Ok(self.inner.create_branch(&name, start, checkout)?)
    }

    // MARK: Remotes

    pub fn remotes(&self) -> Result<Vec<String>> {
        Ok(self.inner.remotes()?)
    }

    /// Fetches `remote`, or all remotes.
    pub fn fetch(
        &self,
        remote: Option<String>,
        observer: Box<dyn RemoteObserver>,
        cancel: Arc<CancelToken>,
    ) -> Result<()> {
        run_remote(observer, &cancel, |options| {
            self.inner.fetch(remote.as_deref(), options)
        })
    }

    pub fn pull(
        &self,
        mode: PullMode,
        observer: Box<dyn RemoteObserver>,
        cancel: Arc<CancelToken>,
    ) -> Result<()> {
        let mode = match mode {
            PullMode::Default => core::PullMode::Default,
            PullMode::FastForwardOnly => core::PullMode::FastForwardOnly,
            PullMode::Merge => core::PullMode::Merge,
            PullMode::Rebase => core::PullMode::Rebase,
        };
        run_remote(observer, &cancel, |options| self.inner.pull(mode, options))
    }

    /// Pushes the current branch; see `push_target` for where to.
    pub fn push(
        &self,
        force: bool,
        observer: Box<dyn RemoteObserver>,
        cancel: Arc<CancelToken>,
    ) -> Result<PushTarget> {
        run_remote(observer, &cancel, |options| self.inner.push(force, options)).map(Into::into)
    }

    pub fn push_target(&self) -> Result<PushTarget> {
        Ok(self.inner.push_target()?.into())
    }

    // MARK: Maintenance and watching

    /// Writes git's commit-graph file, which makes loading history much faster.
    pub fn write_commit_graph(&self, cancel: Arc<CancelToken>) -> Result<()> {
        Ok(self.inner.write_commit_graph(&cancel.inner)?)
    }

    /// Reports changes to the working tree, index and refs until the watcher is stopped or
    /// released.
    pub fn watch(&self, observer: Box<dyn WatchObserver>) -> Result<Arc<Watcher>> {
        let watcher = self.inner.watch(move |event| {
            observer.on_change(WatchEvent {
                working_copy: event.working_copy,
                index: event.index,
                refs: event.refs,
            })
        })?;
        Ok(Arc::new(Watcher {
            inner: std::sync::Mutex::new(Some(watcher)),
        }))
    }
}

fn core_entries(entries: Vec<StatusEntry>) -> Vec<core::StatusEntry> {
    entries.into_iter().map(Into::into).collect()
}

/// Whether `name` can be used for a new branch.
#[uniffi::export]
pub fn is_valid_branch_name(name: String) -> bool {
    core::is_valid_branch_name(&name)
}

// MARK: Write records

#[derive(Clone, Debug, Default, PartialEq, Eq, uniffi::Record)]
pub struct LineSelection {
    /// Removed lines, by their number in the old version.
    pub removed: Vec<u32>,
    /// Added lines, by their number in the new version.
    pub added: Vec<u32>,
}

impl From<LineSelection> for core::LineSelection {
    fn from(selection: LineSelection) -> Self {
        Self {
            removed: selection.removed,
            added: selection.added,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum CheckoutTarget {
    LocalBranch {
        name: String,
    },
    /// Creates a local branch with the same name, tracking this one.
    RemoteBranch {
        name: String,
    },
    /// Leaves HEAD detached.
    Commit {
        id: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum PullMode {
    /// As configured (`pull.rebase`, `pull.ff`).
    Default,
    FastForwardOnly,
    Merge,
    Rebase,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct PushTarget {
    pub remote: String,
    pub remote_ref: String,
    pub set_upstream: bool,
}

impl From<core::PushTarget> for PushTarget {
    fn from(target: core::PushTarget) -> Self {
        Self {
            remote: target.remote,
            remote_ref: target.remote_ref,
            set_upstream: target.set_upstream,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct RemoteProgress {
    pub phase: String,
    pub percent: Option<u8>,
    pub message: String,
}

/// Hears about a fetch, pull or push, and answers git's credential prompts.
#[uniffi::export(callback_interface)]
pub trait RemoteObserver: Send + Sync {
    fn on_progress(&self, progress: RemoteProgress);
    /// Asks the person for `prompt`; `secret` means a password field. `None` cancels.
    fn ask_credential(&self, prompt: String, secret: bool) -> Option<String>;
}

/// Adapts the foreign observer to the core's credential trait.
struct ObserverCredentials(Arc<dyn RemoteObserver>);

impl core::CredentialPrompter for ObserverCredentials {
    fn ask(&self, prompt: &str, secret: bool) -> Option<String> {
        self.0.ask_credential(prompt.to_owned(), secret)
    }
}

fn run_remote<T>(
    observer: Box<dyn RemoteObserver>,
    cancel: &CancelToken,
    operation: impl FnOnce(core::RemoteOptions<'_>) -> core::Result<T>,
) -> Result<T> {
    let observer: Arc<dyn RemoteObserver> = Arc::from(observer);
    let mut progress = {
        let observer = observer.clone();
        move |progress: &core::RemoteProgress| {
            observer.on_progress(RemoteProgress {
                phase: progress.phase.clone(),
                percent: progress.percent,
                message: progress.message.clone(),
            })
        }
    };
    Ok(operation(core::RemoteOptions {
        progress: &mut progress,
        credentials: Some(Arc::new(ObserverCredentials(observer))),
        cancel: &cancel.inner,
    })?)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Record)]
pub struct WatchEvent {
    pub working_copy: bool,
    pub index: bool,
    pub refs: bool,
}

#[uniffi::export(callback_interface)]
pub trait WatchObserver: Send + Sync {
    fn on_change(&self, event: WatchEvent);
}

#[derive(Debug, uniffi::Object)]
pub struct Watcher {
    inner: std::sync::Mutex<Option<core::RepositoryWatcher>>,
}

#[uniffi::export]
impl Watcher {
    pub fn stop(&self) {
        self.inner.lock().expect("watcher lock poisoned").take();
    }
}

// MARK: Refs

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct Refs {
    pub head: Option<Head>,
    pub local_branches: Vec<LocalBranch>,
    pub remote_branches: Vec<RemoteBranch>,
    pub tags: Vec<Tag>,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct LocalBranch {
    pub name: String,
    pub full_name: String,
    pub target: String,
    pub upstream: Option<String>,
    pub is_head: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct RemoteBranch {
    pub remote: String,
    pub name: String,
    pub full_name: String,
    pub target: String,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct Tag {
    pub name: String,
    pub full_name: String,
    pub target: String,
}

impl From<core::Refs> for Refs {
    fn from(refs: core::Refs) -> Self {
        Self {
            head: refs.head.map(Into::into),
            local_branches: refs
                .local_branches
                .into_iter()
                .map(|branch| LocalBranch {
                    name: branch.name,
                    full_name: branch.full_name,
                    target: branch.target.to_string(),
                    upstream: branch.upstream,
                    is_head: branch.is_head,
                })
                .collect(),
            remote_branches: refs
                .remote_branches
                .into_iter()
                .map(|branch| RemoteBranch {
                    remote: branch.remote,
                    name: branch.name,
                    full_name: branch.full_name,
                    target: branch.target.to_string(),
                })
                .collect(),
            tags: refs
                .tags
                .into_iter()
                .map(|tag| Tag {
                    name: tag.name,
                    full_name: tag.full_name,
                    target: tag.target.to_string(),
                })
                .collect(),
        }
    }
}

// MARK: History

#[derive(Debug, Default, uniffi::Object)]
pub struct CancelToken {
    inner: core::CancelToken,
}

#[uniffi::export]
impl CancelToken {
    #[uniffi::constructor]
    pub fn new() -> Arc<Self> {
        Arc::default()
    }

    pub fn cancel(&self) {
        self.inner.cancel();
    }

    pub fn is_cancelled(&self) -> bool {
        self.inner.is_cancelled()
    }
}

#[uniffi::export(callback_interface)]
pub trait HistoryObserver: Send + Sync {
    fn on_progress(&self, progress: HistoryProgress);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum HistoryPhase {
    Indexing,
    Layout,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Record)]
pub struct HistoryProgress {
    pub phase: HistoryPhase,
    pub commits: u32,
}

impl From<core::HistoryProgress> for HistoryProgress {
    fn from(progress: core::HistoryProgress) -> Self {
        Self {
            phase: match progress.phase {
                core::HistoryPhase::Indexing => HistoryPhase::Indexing,
                core::HistoryPhase::Layout => HistoryPhase::Layout,
            },
            commits: progress.commits,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Record)]
pub struct HistoryStats {
    pub commits: u32,
    pub commits_from_commit_graph: u32,
    pub commits_decoded: u32,
    pub index_millis: u64,
    pub layout_millis: u64,
}

#[derive(Debug, uniffi::Object)]
pub struct History {
    inner: core::History,
}

#[uniffi::export]
impl History {
    pub fn row_count(&self) -> u32 {
        self.inner.len()
    }

    /// The widest the graph gets, in lanes.
    pub fn max_lanes(&self) -> u16 {
        self.inner.max_lanes()
    }

    /// Rows `start..start + count`, clamped to the history. Cheap enough for the UI thread.
    pub fn rows(&self, start: u32, count: u32) -> Result<Vec<HistoryRow>> {
        Ok(self
            .inner
            .rows(start, count)?
            .into_iter()
            .map(Into::into)
            .collect())
    }

    pub fn row_of(&self, id: String) -> Option<u32> {
        let id = core::parse_object_id(&id).ok()?;
        self.inner.row_of(&id)
    }

    pub fn stats(&self) -> HistoryStats {
        let stats = self.inner.stats();
        HistoryStats {
            commits: stats.commits,
            commits_from_commit_graph: stats.index.from_commit_graph,
            commits_decoded: stats.index.decoded,
            index_millis: stats.index_time.as_millis() as u64,
            layout_millis: stats.layout_time.as_millis() as u64,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct HistoryRow {
    pub id: String,
    pub summary: String,
    pub author_name: String,
    pub author_email: String,
    /// Seconds since the epoch.
    pub author_time: i64,
    /// When the commit was made, in seconds since the epoch. History is sorted by this.
    pub commit_time: i64,
    /// The checked-out branch (or detached HEAD) first, then other local branches, branches only
    /// on remotes, and tags.
    pub refs: Vec<RefLabel>,
    pub graph: GraphRow,
}

impl From<core::HistoryRow> for HistoryRow {
    fn from(row: core::HistoryRow) -> Self {
        Self {
            id: row.id.to_string(),
            summary: row.summary,
            author_name: row.author_name,
            author_email: row.author_email,
            author_time: row.author_time,
            commit_time: row.commit_time,
            refs: row
                .refs
                .into_iter()
                .map(|label| RefLabel {
                    display_name: label.display_name(),
                    kind: match label.kind {
                        core::RefLabelKind::Head => RefLabelKind::Head,
                        core::RefLabelKind::Branch => RefLabelKind::Branch,
                        core::RefLabelKind::Tag => RefLabelKind::Tag,
                    },
                    name: label.name,
                    local: label.local,
                    remotes: label.remotes,
                    is_head: label.is_head,
                })
                .collect(),
            graph: GraphRow {
                node_lane: row.graph.node_lane,
                node_color: row.graph.node_color,
                lane_count: row.graph.lane_count,
                edges: row
                    .graph
                    .edges
                    .into_iter()
                    .map(|edge| GraphEdge {
                        from_lane: edge.from_lane,
                        to_lane: edge.to_lane,
                        color: edge.color,
                        kind: match edge.kind {
                            core::EdgeKind::Through => EdgeKind::Through,
                            core::EdgeKind::IntoNode => EdgeKind::IntoNode,
                            core::EdgeKind::OutOfNode => EdgeKind::OutOfNode,
                        },
                    })
                    .collect(),
            },
        }
    }
}

/// A name on a commit. A local branch and remote branches of the same name on the same commit
/// (`main`, `origin/main`) are one label.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct RefLabel {
    pub kind: RefLabelKind,
    /// The branch name without a remote (`main`), the tag name, or `HEAD`.
    pub name: String,
    /// As git spells it: `main`, or `origin/main` for a branch only on a remote.
    pub display_name: String,
    /// Branches: a local branch of this name points here.
    pub local: bool,
    /// Branches: the remotes whose branch of this name points here, e.g. `origin`.
    pub remotes: Vec<String>,
    /// The branch HEAD points to, or the detached HEAD itself.
    pub is_head: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum RefLabelKind {
    /// A detached HEAD.
    Head,
    Branch,
    Tag,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct GraphRow {
    pub node_lane: u16,
    pub node_color: u16,
    pub lane_count: u16,
    pub edges: Vec<GraphEdge>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Record)]
pub struct GraphEdge {
    pub from_lane: u16,
    pub to_lane: u16,
    pub color: u16,
    pub kind: EdgeKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum EdgeKind {
    /// Top of the row to the bottom, in the same lane, passing by the commit.
    Through,
    /// Top of the row in `from_lane` into the commit's dot in `to_lane`.
    IntoNode,
    /// The commit's dot in `from_lane` to the bottom of the row in `to_lane`.
    OutOfNode,
}

// MARK: Commits

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct CommitDetails {
    pub id: String,
    pub parents: Vec<String>,
    pub author: Signature,
    pub committer: Signature,
    pub summary: String,
    pub message: String,
    pub changes: Vec<FileChange>,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct Signature {
    pub name: String,
    pub email: String,
    pub time: i64,
    /// Offset from UTC in seconds.
    pub offset: i32,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FileChange {
    pub path: RepoPath,
    pub old_path: Option<RepoPath>,
    pub kind: ChangeKind,
    pub old: Option<BlobRef>,
    pub new: Option<BlobRef>,
    pub stats: Option<LineStats>,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct BlobRef {
    pub id: String,
    pub mode: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Record)]
pub struct LineStats {
    pub additions: u32,
    pub deletions: u32,
}

impl From<core::CommitDetails> for CommitDetails {
    fn from(details: core::CommitDetails) -> Self {
        let signature = |signature: core::Signature| Signature {
            name: signature.name,
            email: signature.email,
            time: signature.time,
            offset: signature.offset,
        };
        Self {
            id: details.id.to_string(),
            parents: details.parents.iter().map(ToString::to_string).collect(),
            author: signature(details.author),
            committer: signature(details.committer),
            summary: details.summary,
            message: details.message,
            changes: details.changes.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<core::FileChange> for FileChange {
    fn from(change: core::FileChange) -> Self {
        let blob = |blob: core::BlobRef| BlobRef {
            id: blob.id.to_string(),
            mode: blob.mode,
        };
        Self {
            path: change.path.into(),
            old_path: change.old_path.map(Into::into),
            kind: change.kind.into(),
            old: change.old.map(blob),
            new: change.new.map(blob),
            stats: change.stats.map(|stats| LineStats {
                additions: stats.additions,
                deletions: stats.deletions,
            }),
        }
    }
}

impl TryFrom<FileChange> for core::FileChange {
    type Error = BranchosaurusError;

    fn try_from(change: FileChange) -> Result<Self> {
        let blob = |blob: Option<BlobRef>| -> Result<Option<core::BlobRef>> {
            blob.map(|blob| {
                Ok(core::BlobRef {
                    id: core::parse_object_id(&blob.id)?,
                    mode: blob.mode,
                })
            })
            .transpose()
        };
        Ok(Self {
            path: change.path.into(),
            old_path: change.old_path.map(Into::into),
            kind: change.kind.into(),
            old: blob(change.old)?,
            new: blob(change.new)?,
            stats: change.stats.map(|stats| core::LineStats {
                additions: stats.additions,
                deletions: stats.deletions,
            }),
        })
    }
}

// MARK: Status

#[derive(Clone, Debug, Default, PartialEq, Eq, uniffi::Record)]
pub struct WorkingCopyStatus {
    pub staged: Vec<StatusEntry>,
    pub unstaged: Vec<StatusEntry>,
    pub conflicted: Vec<RepoPath>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, uniffi::Record)]
pub struct StatusEntry {
    pub path: RepoPath,
    pub old_path: Option<RepoPath>,
    pub kind: ChangeKind,
}

impl From<core::WorkingCopyStatus> for WorkingCopyStatus {
    fn from(status: core::WorkingCopyStatus) -> Self {
        let entries = |entries: Vec<core::StatusEntry>| {
            entries
                .into_iter()
                .map(|entry| StatusEntry {
                    path: entry.path.into(),
                    old_path: entry.old_path.map(Into::into),
                    kind: entry.kind.into(),
                })
                .collect()
        };
        Self {
            staged: entries(status.staged),
            unstaged: entries(status.unstaged),
            conflicted: status.conflicted.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<StatusEntry> for core::StatusEntry {
    fn from(entry: StatusEntry) -> Self {
        Self {
            path: entry.path.into(),
            old_path: entry.old_path.map(Into::into),
            kind: entry.kind.into(),
        }
    }
}

// MARK: Diffs

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FileDiff {
    pub path: RepoPath,
    pub old_path: Option<RepoPath>,
    pub old_mode: Option<u32>,
    pub new_mode: Option<u32>,
    /// The language the lines are highlighted as, e.g. `Rust`. `None` when the language is
    /// unknown or the diff is too big to highlight quickly.
    pub language: Option<String>,
    pub content: DiffContent,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum DiffContent {
    Text {
        hunks: Vec<Hunk>,
        additions: u32,
        deletions: u32,
    },
    Binary {
        old_size: Option<u64>,
        new_size: Option<u64>,
    },
    TooLarge {
        old_size: u64,
        new_size: u64,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct Hunk {
    pub old_start: u32,
    pub old_lines: u32,
    pub new_start: u32,
    pub new_lines: u32,
    pub header: String,
    pub lines: Vec<DiffLine>,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct DiffLine {
    pub kind: LineKind,
    pub old_line: Option<u32>,
    pub new_line: Option<u32>,
    pub text: String,
    pub highlights: Vec<Highlight>,
    pub missing_newline: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum LineKind {
    Context,
    Added,
    Removed,
}

/// A highlighted range of a line's text, in UTF-16 code units (the unit of `NSString` and C#
/// strings), so apps can use it without converting.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Record)]
pub struct Highlight {
    pub start: u32,
    pub end: u32,
    pub kind: HighlightKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum HighlightKind {
    Keyword,
    String,
    Comment,
    Number,
    Constant,
    Type,
    Function,
    Attribute,
    Tag,
    Property,
    Operator,
}

impl From<core::HighlightKind> for HighlightKind {
    fn from(kind: core::HighlightKind) -> Self {
        match kind {
            core::HighlightKind::Keyword => Self::Keyword,
            core::HighlightKind::String => Self::String,
            core::HighlightKind::Comment => Self::Comment,
            core::HighlightKind::Number => Self::Number,
            core::HighlightKind::Constant => Self::Constant,
            core::HighlightKind::Type => Self::Type,
            core::HighlightKind::Function => Self::Function,
            core::HighlightKind::Attribute => Self::Attribute,
            core::HighlightKind::Tag => Self::Tag,
            core::HighlightKind::Property => Self::Property,
            core::HighlightKind::Operator => Self::Operator,
        }
    }
}

/// Converts highlights from UTF-8 byte offsets to UTF-16 offsets in one pass over the line.
fn utf16_highlights(text: &str, highlights: Vec<core::Highlight>) -> Vec<Highlight> {
    let mut chars = text.char_indices().peekable();
    let mut utf16 = 0u32;
    // The UTF-16 offset of a byte offset at or after every earlier one.
    let mut offset = |byte: u32| {
        while let Some(&(index, c)) = chars.peek() {
            if index >= byte as usize {
                break;
            }
            utf16 += c.len_utf16() as u32;
            chars.next();
        }
        utf16
    };
    highlights
        .into_iter()
        .map(|highlight| Highlight {
            start: offset(highlight.start),
            end: offset(highlight.end),
            kind: highlight.kind.into(),
        })
        .collect()
}

impl From<core::FileDiff> for FileDiff {
    fn from(diff: core::FileDiff) -> Self {
        let content = match diff.content {
            core::DiffContent::Text {
                hunks,
                additions,
                deletions,
            } => DiffContent::Text {
                hunks: hunks
                    .into_iter()
                    .map(|hunk| Hunk {
                        header: hunk.header(),
                        old_start: hunk.old_start,
                        old_lines: hunk.old_lines,
                        new_start: hunk.new_start,
                        new_lines: hunk.new_lines,
                        lines: hunk
                            .lines
                            .into_iter()
                            .map(|line| DiffLine {
                                kind: match line.kind {
                                    core::LineKind::Context => LineKind::Context,
                                    core::LineKind::Added => LineKind::Added,
                                    core::LineKind::Removed => LineKind::Removed,
                                },
                                old_line: line.old_line,
                                new_line: line.new_line,
                                highlights: utf16_highlights(&line.text, line.highlights),
                                text: line.text,
                                missing_newline: line.missing_newline,
                            })
                            .collect(),
                    })
                    .collect(),
                additions,
                deletions,
            },
            core::DiffContent::Binary { old_size, new_size } => {
                DiffContent::Binary { old_size, new_size }
            }
            core::DiffContent::TooLarge { old_size, new_size } => {
                DiffContent::TooLarge { old_size, new_size }
            }
        };
        Self {
            path: diff.path.into(),
            old_path: diff.old_path.map(Into::into),
            old_mode: diff.old_mode,
            new_mode: diff.new_mode,
            language: diff.language,
            content,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn highlights_are_converted_to_utf16() {
        // "é" is 2 bytes and 1 UTF-16 unit; "☕" 3 and 1; "𝄞" 4 and 2.
        let text = "é ☕ 𝄞 x";
        let highlight = |text_part: &str| {
            let start = text.find(text_part).unwrap() as u32;
            core::Highlight {
                start,
                end: start + text_part.len() as u32,
                kind: core::HighlightKind::String,
            }
        };
        let converted = utf16_highlights(
            text,
            vec![
                highlight("é"),
                highlight("☕"),
                highlight("𝄞"),
                highlight("x"),
            ],
        );
        let ranges: Vec<_> = converted.iter().map(|h| (h.start, h.end)).collect();
        assert_eq!(ranges, [(0, 1), (2, 3), (4, 6), (7, 8)]);
        let units: Vec<u16> = text.encode_utf16().collect();
        assert_eq!(String::from_utf16(&units[4..6]).unwrap(), "𝄞");
    }
}
