// SPDX-License-Identifier: GPL-3.0-or-later

//! The Branchosaurus core: the git engine and UI-shaped data shared by every native shell.
//!
//! Reads go through [gitoxide](https://github.com/GitoxideLabs/gitoxide); writes and network
//! operations go through the user's `git` executable (see [`GitCli`]). Shells get data that is
//! ready to display, such as commit graph lanes and diff hunks, so no shell reimplements git
//! logic. `docs/ARCHITECTURE.md` explains the design.

mod askpass;
mod avatar;
mod branch;
mod cancel;
mod cli;
mod commit;
mod diff;
mod error;
mod highlight;
mod history;
mod partial;
mod path;
mod refs;
mod remote;
mod repo;
mod stage;
mod status;
mod watch;

pub use askpass::CredentialPrompter;
pub use avatar::{Avatar, avatar};
pub use branch::{CheckoutTarget, is_valid_branch_name};
pub use cancel::CancelToken;
pub use cli::{GitCli, GitVersion};
pub use commit::{CommitDetails, FileChange, LineStats, Signature};
pub use diff::{BlobRef, ChangeKind, DiffContent, DiffLine, FileDiff, Hunk, LineKind};
pub use error::{Error, Result};
pub use gix::ObjectId;
pub use highlight::{Highlight, HighlightKind, syntax_acknowledgements};
pub use history::{
    EdgeKind, GraphEdge, GraphRow, History, HistoryPhase, HistoryProgress, HistoryRow,
    HistoryStats, IndexStats, RefLabel, RefLabelKind, render_ascii,
};
pub use partial::LineSelection;
pub use path::RepoPath;
pub use refs::{LocalBranch, Refs, RemoteBranch, Tag};
pub use remote::{PullMode, PushTarget, RemoteOptions, RemoteProgress};
pub use repo::{Head, Repository, RepositoryInfo, RepositoryState, parse_object_id};
pub use stage::UntrackedRemoval;
pub use status::{StatusBackend, StatusEntry, WorkingCopyStatus};
pub use watch::{RepositoryWatcher, WatchEvent};
