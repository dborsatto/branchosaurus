// SPDX-License-Identifier: GPL-3.0-or-later

use gix::ObjectId;
use gix::bstr::ByteSlice;
use gix::diff::blob::ResourceKind;
use gix::diff::blob::platform::prepare_diff::Operation;
use gix::object::tree::diff::ChangeDetached;

use crate::diff::{
    BlobRef, ChangeKind, FileDiff, Side, diff_sides, entry_kind_to_mode, line_stats,
    mode_to_entry_kind,
};
use crate::error::Context;
use crate::{Error, RepoPath, Repository, Result};

/// Beyond this many changed files, per-file line counts are skipped to keep huge commits
/// (vendored dependencies, mass renames) responsive.
const MAX_FILES_WITH_STATS: usize = 1000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommitDetails {
    pub id: ObjectId,
    pub parents: Vec<ObjectId>,
    pub author: Signature,
    pub committer: Signature,
    pub summary: String,
    /// The full message, summary included.
    pub message: String,
    /// Changes against the first parent (or against nothing, for a root commit).
    pub changes: Vec<FileChange>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Signature {
    pub name: String,
    pub email: String,
    /// Seconds since the epoch.
    pub time: i64,
    /// Offset from UTC, in seconds.
    pub offset: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileChange {
    pub path: RepoPath,
    /// The source of a rename or copy.
    pub old_path: Option<RepoPath>,
    pub kind: ChangeKind,
    pub old: Option<BlobRef>,
    pub new: Option<BlobRef>,
    /// Lines added and removed; `None` for binary files and very large commits.
    pub stats: Option<LineStats>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LineStats {
    pub additions: u32,
    pub deletions: u32,
}

impl Repository {
    pub fn commit_details(&self, id: ObjectId) -> Result<CommitDetails> {
        let repo = self.local();
        let commit = repo
            .find_commit(id)
            .map_err(|_| Error::NotFound(format!("commit {id}")))?;
        let parents: Vec<ObjectId> = commit.parent_ids().map(|id| id.detach()).collect();
        let message = commit.message_raw_sloppy().to_str_lossy().into_owned();
        let summary = commit
            .message()
            .map(|message| message.summary().to_str_lossy().into_owned())
            .unwrap_or_default();
        let author = signature(commit.author().context("reading the commit author")?);
        let committer = signature(commit.committer().context("reading the committer")?);

        let tree = commit.tree().context("reading the commit's tree")?;
        let parent_tree = match parents.first() {
            Some(parent) => Some(
                repo.find_commit(*parent)
                    .context("reading the parent commit")?
                    .tree()
                    .context("reading the parent's tree")?,
            ),
            None => None,
        };
        let raw_changes = repo
            .diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), None)
            .context("comparing the commit with its parent")?;
        let mut changes: Vec<FileChange> =
            raw_changes.into_iter().filter_map(file_change).collect();
        changes.sort_by(|a, b| a.path.cmp(&b.path));
        if changes.len() <= MAX_FILES_WITH_STATS {
            add_line_stats(&repo, &mut changes)?;
        }

        Ok(CommitDetails {
            id,
            parents,
            author,
            committer,
            summary,
            message,
            changes,
        })
    }

    /// The line diff of one file changed by a commit.
    pub fn commit_file_diff(&self, change: &FileChange) -> Result<FileDiff> {
        let repo = self.local();
        let old_path = change.old_path.as_ref().unwrap_or(&change.path);
        diff_sides(
            &repo,
            None,
            change.path.clone(),
            change.old_path.clone(),
            object_side(change.old.as_ref(), old_path),
            object_side(change.new.as_ref(), &change.path),
        )
    }
}

fn object_side<'a>(blob: Option<&BlobRef>, path: &'a RepoPath) -> Side<'a> {
    match blob {
        Some(blob) => Side::Object {
            id: blob.id,
            kind: mode_to_entry_kind(blob.mode),
            path: path.as_bstr(),
        },
        None => Side::Missing,
    }
}

fn signature(signature: gix::actor::SignatureRef<'_>) -> Signature {
    let time = signature.time().unwrap_or_default();
    Signature {
        name: signature.name.to_str_lossy().into_owned(),
        email: signature.email.to_str_lossy().into_owned(),
        time: time.seconds,
        offset: time.offset,
    }
}

fn file_change(change: ChangeDetached) -> Option<FileChange> {
    let blob = |id: ObjectId, mode: gix::object::tree::EntryMode| BlobRef {
        id,
        mode: entry_kind_to_mode(mode.kind()),
    };
    Some(match change {
        ChangeDetached::Addition {
            location,
            entry_mode,
            id,
            ..
        } => {
            if entry_mode.is_tree() {
                return None;
            }
            FileChange {
                path: location.into(),
                old_path: None,
                kind: ChangeKind::Added,
                old: None,
                new: Some(blob(id, entry_mode)),
                stats: None,
            }
        }
        ChangeDetached::Deletion {
            location,
            entry_mode,
            id,
            ..
        } => {
            if entry_mode.is_tree() {
                return None;
            }
            FileChange {
                path: location.into(),
                old_path: None,
                kind: ChangeKind::Deleted,
                old: Some(blob(id, entry_mode)),
                new: None,
                stats: None,
            }
        }
        ChangeDetached::Modification {
            location,
            previous_entry_mode,
            previous_id,
            entry_mode,
            id,
        } => {
            if entry_mode.is_tree() && previous_entry_mode.is_tree() {
                return None;
            }
            let kind = if previous_entry_mode.kind() == entry_mode.kind()
                || (!previous_entry_mode.is_link()
                    && !entry_mode.is_link()
                    && !previous_entry_mode.is_commit()
                    && !entry_mode.is_commit())
            {
                ChangeKind::Modified
            } else {
                ChangeKind::TypeChanged
            };
            FileChange {
                path: location.into(),
                old_path: None,
                kind,
                old: Some(blob(previous_id, previous_entry_mode)),
                new: Some(blob(id, entry_mode)),
                stats: None,
            }
        }
        ChangeDetached::Rewrite {
            source_location,
            source_entry_mode,
            source_id,
            entry_mode,
            id,
            location,
            copy,
            ..
        } => {
            if entry_mode.is_tree() {
                return None;
            }
            FileChange {
                path: location.into(),
                old_path: Some(source_location.into()),
                kind: if copy {
                    ChangeKind::Copied
                } else {
                    ChangeKind::Renamed
                },
                old: Some(blob(source_id, source_entry_mode)),
                new: Some(blob(id, entry_mode)),
                stats: None,
            }
        }
    })
}

fn add_line_stats(repo: &gix::Repository, changes: &mut [FileChange]) -> Result<()> {
    let mut cache = repo
        .diff_resource_cache_for_tree_diff()
        .context("preparing to diff")?;
    let null = repo.object_hash().null();
    for change in changes {
        let new_kind = change.new.map(|blob| mode_to_entry_kind(blob.mode));
        let old_kind = change.old.map(|blob| mode_to_entry_kind(blob.mode));
        if [new_kind, old_kind].contains(&Some(gix::object::tree::EntryKind::Commit)) {
            continue;
        }
        let old_path = change.old_path.as_ref().unwrap_or(&change.path);
        let fallback_kind = new_kind
            .or(old_kind)
            .unwrap_or(gix::object::tree::EntryKind::Blob);
        let old_set = cache.set_resource(
            change.old.map_or(null, |blob| blob.id),
            old_kind.unwrap_or(fallback_kind),
            old_path.as_bstr(),
            ResourceKind::OldOrSource,
            &repo.objects,
        );
        let new_set = cache.set_resource(
            change.new.map_or(null, |blob| blob.id),
            new_kind.unwrap_or(fallback_kind),
            change.path.as_bstr(),
            ResourceKind::NewOrDestination,
            &repo.objects,
        );
        if old_set.is_err() || new_set.is_err() {
            continue;
        }
        let Ok(outcome) = cache.prepare_diff() else {
            continue;
        };
        if let Operation::InternalDiff { algorithm } = outcome.operation {
            let (additions, deletions) = line_stats(
                algorithm,
                outcome.old.data.as_slice().unwrap_or_default(),
                outcome.new.data.as_slice().unwrap_or_default(),
            );
            change.stats = Some(LineStats {
                additions,
                deletions,
            });
        }
    }
    Ok(())
}
