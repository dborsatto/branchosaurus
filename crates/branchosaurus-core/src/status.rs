// SPDX-License-Identifier: GPL-3.0-or-later

//! Working-copy status: what is staged, what is not, and what is in conflict.

use std::collections::BTreeSet;

use gix::bstr::{BStr, ByteSlice};
use gix::object::tree::EntryKind;
use gix::status::index_worktree::Item as WorktreeItem;
use gix::status::index_worktree::iter::Summary;

use crate::diff::{ChangeKind, FileDiff, Side, diff_sides};
use crate::error::Context;
use crate::{Error, RepoPath, Repository, Result};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WorkingCopyStatus {
    /// Differences between HEAD and the index.
    pub staged: Vec<StatusEntry>,
    /// Differences between the index and the working tree, untracked files included.
    pub unstaged: Vec<StatusEntry>,
    /// Paths with unresolved merge conflicts.
    pub conflicted: Vec<RepoPath>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct StatusEntry {
    pub path: RepoPath,
    /// The source of a staged rename or copy.
    pub old_path: Option<RepoPath>,
    pub kind: ChangeKind,
}

/// Which implementation produces the status.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatusBackend {
    /// Pick gitoxide unless the repository needs a feature only git implements.
    Auto,
    Gitoxide,
    GitCli,
}

impl Repository {
    pub fn status(&self) -> Result<WorkingCopyStatus> {
        self.status_with(StatusBackend::Auto)
    }

    pub fn status_with(&self, backend: StatusBackend) -> Result<WorkingCopyStatus> {
        self.require_workdir()?;
        let use_cli = match backend {
            StatusBackend::Auto => self.needs_cli_status(),
            StatusBackend::Gitoxide => false,
            StatusBackend::GitCli => true,
        };
        let mut status = if use_cli {
            self.cli_status()?
        } else {
            self.gix_status()?
        };
        status.staged.sort();
        status.unstaged.sort();
        status.conflicted.sort();
        Ok(status)
    }

    fn gix_status(&self) -> Result<WorkingCopyStatus> {
        let repo = self.local();
        let iter = repo
            .status(gix::progress::Discard)
            .context("preparing status")?
            .untracked_files(gix::status::UntrackedFiles::Files)
            .index_worktree_rewrites(None)
            .index_worktree_submodules(gix::status::Submodule::Given {
                ignore: gix::submodule::config::Ignore::Dirty,
                check_dirty: false,
            })
            .into_iter(None)
            .context("computing status")?;

        let mut status = WorkingCopyStatus::default();
        let mut conflicted = BTreeSet::new();
        for item in iter {
            match item.context("computing status")? {
                gix::status::Item::TreeIndex(change) => {
                    use gix::diff::index::ChangeRef;
                    let entry = match change {
                        ChangeRef::Addition { location, .. } => {
                            entry(&location, None, ChangeKind::Added)
                        }
                        ChangeRef::Deletion { location, .. } => {
                            entry(&location, None, ChangeKind::Deleted)
                        }
                        ChangeRef::Modification {
                            location,
                            previous_entry_mode,
                            entry_mode,
                            ..
                        } => {
                            let kind = if is_type_change(
                                previous_entry_mode
                                    .to_tree_entry_mode()
                                    .map(|mode| mode.kind()),
                                entry_mode.to_tree_entry_mode().map(|mode| mode.kind()),
                            ) {
                                ChangeKind::TypeChanged
                            } else {
                                ChangeKind::Modified
                            };
                            entry(&location, None, kind)
                        }
                        ChangeRef::Rewrite {
                            source_location,
                            location,
                            copy,
                            ..
                        } => entry(
                            &location,
                            Some(&source_location),
                            if copy {
                                ChangeKind::Copied
                            } else {
                                ChangeKind::Renamed
                            },
                        ),
                    };
                    status.staged.push(entry);
                }
                gix::status::Item::IndexWorktree(item) => {
                    let Some(summary) = item.summary() else {
                        continue;
                    };
                    let path = item.rela_path();
                    let kind = match summary {
                        Summary::Conflict => {
                            conflicted.insert(RepoPath::from(path));
                            continue;
                        }
                        Summary::Added => {
                            if matches!(item, WorktreeItem::DirectoryContents { .. }) {
                                ChangeKind::Untracked
                            } else {
                                ChangeKind::Added
                            }
                        }
                        Summary::IntentToAdd => ChangeKind::Added,
                        Summary::Removed => ChangeKind::Deleted,
                        Summary::Modified => ChangeKind::Modified,
                        Summary::TypeChange => ChangeKind::TypeChanged,
                        Summary::Renamed => ChangeKind::Renamed,
                        Summary::Copied => ChangeKind::Copied,
                    };
                    status.unstaged.push(entry(path, None, kind));
                }
            }
        }
        // Conflicted paths also show up as index changes; list them only once, as conflicts.
        status
            .staged
            .retain(|entry| !conflicted.contains(&entry.path));
        status.conflicted = conflicted.into_iter().collect();
        Ok(status)
    }

    fn cli_status(&self) -> Result<WorkingCopyStatus> {
        let output = self.git()?.run(
            self.require_workdir()?,
            [
                "--no-optional-locks",
                "status",
                "--porcelain=v2",
                "-z",
                "--untracked-files=all",
                "--ignore-submodules=dirty",
            ],
        )?;
        parse_porcelain_v2(&output)
    }

    /// The diff of what is staged for `entry`: HEAD against the index.
    pub fn staged_diff(&self, entry: &StatusEntry) -> Result<FileDiff> {
        let repo = self.local();
        let (old, new) = staged_sides(&repo, entry)?;
        diff_sides(
            &repo,
            None,
            entry.path.clone(),
            entry.old_path.clone(),
            old,
            new,
        )
    }

    /// The diff of what is not staged for `entry`: the index against the working tree.
    pub fn unstaged_diff(&self, entry: &StatusEntry) -> Result<FileDiff> {
        let repo = self.local();
        let workdir = self.require_workdir()?;
        let (old, new) = unstaged_sides(&repo, workdir, entry)?;
        diff_sides(&repo, Some(workdir), entry.path.clone(), None, old, new)
    }
}

/// The HEAD and index versions of a staged change.
pub(crate) fn staged_sides<'a>(
    repo: &gix::Repository,
    entry: &'a StatusEntry,
) -> Result<(Side<'a>, Side<'a>)> {
    let index = repo.index_or_empty().context("reading the index")?;
    let head_tree = repo.head_tree().ok();
    let old_path = entry.old_path.as_ref().unwrap_or(&entry.path);
    let old = match &head_tree {
        Some(tree) => tree_side(tree, old_path)?,
        None => Side::Missing,
    };
    let new = match index.entry_by_path(entry.path.as_bstr()) {
        Some(index_entry) => Side::Object {
            id: index_entry.id,
            kind: index_kind(index_entry.mode),
            path: entry.path.as_bstr(),
        },
        None => Side::Missing,
    };
    Ok((old, new))
}

/// The index and working-tree versions of an unstaged change.
pub(crate) fn unstaged_sides<'a>(
    repo: &gix::Repository,
    workdir: &std::path::Path,
    entry: &'a StatusEntry,
) -> Result<(Side<'a>, Side<'a>)> {
    let index = repo.index_or_empty().context("reading the index")?;
    let index_entry = index.entry_by_path(entry.path.as_bstr());
    let old = match index_entry {
        Some(index_entry) if entry.kind != ChangeKind::Untracked => Side::Object {
            id: index_entry.id,
            kind: index_kind(index_entry.mode),
            path: entry.path.as_bstr(),
        },
        _ => Side::Missing,
    };
    let file = crate::stage::worktree_path(workdir, &entry.path)?;
    let new = match std::fs::symlink_metadata(&file) {
        Ok(metadata) => Side::Worktree {
            path: entry.path.as_bstr(),
            kind: worktree_kind(&metadata, index_entry.map(|entry| index_kind(entry.mode))),
            index_id: index_entry.map(|entry| entry.id),
        },
        Err(_) => Side::Missing,
    };
    Ok((old, new))
}

fn entry(path: &BStr, old_path: Option<&BStr>, kind: ChangeKind) -> StatusEntry {
    StatusEntry {
        path: path.into(),
        old_path: old_path.map(Into::into),
        kind,
    }
}

fn is_type_change(old: Option<EntryKind>, new: Option<EntryKind>) -> bool {
    let class = |kind: Option<EntryKind>| match kind {
        Some(EntryKind::Blob | EntryKind::BlobExecutable) => 0,
        Some(EntryKind::Link) => 1,
        Some(EntryKind::Commit) => 2,
        _ => 3,
    };
    class(old) != class(new)
}

fn index_kind(mode: gix::index::entry::Mode) -> EntryKind {
    mode.to_tree_entry_mode()
        .map_or(EntryKind::Blob, |mode| mode.kind())
}

#[cfg_attr(unix, allow(unused_variables))]
fn worktree_kind(metadata: &std::fs::Metadata, index: Option<EntryKind>) -> EntryKind {
    if metadata.file_type().is_symlink() {
        return EntryKind::Link;
    }
    if metadata.is_dir() {
        // A directory where a submodule is checked out.
        return EntryKind::Commit;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 != 0 {
            return EntryKind::BlobExecutable;
        }
        EntryKind::Blob
    }
    #[cfg(not(unix))]
    {
        // Windows has no executable bit; keep what the index says.
        index.unwrap_or(EntryKind::Blob)
    }
}

fn tree_side<'a>(tree: &gix::Tree<'_>, path: &'a RepoPath) -> Result<Side<'a>> {
    let relative = gix::path::from_bstr(path.as_bstr())
        .map_err(|_| Error::NotFound(format!("file {path}")))?;
    let found = tree
        .clone()
        .peel_to_entry_by_path(relative)
        .context("looking up a file in HEAD")?;
    Ok(match found {
        Some(entry) => Side::Object {
            id: entry.object_id(),
            kind: entry.mode().kind(),
            path: path.as_bstr(),
        },
        None => Side::Missing,
    })
}

/// Parses `git status --porcelain=v2 -z` output.
pub(crate) fn parse_porcelain_v2(output: &[u8]) -> Result<WorkingCopyStatus> {
    let malformed = || Error::GitCommand {
        command: "status --porcelain=v2".into(),
        stderr: "unexpected output".into(),
    };
    let mut status = WorkingCopyStatus::default();
    let mut records = output
        .split(|b| *b == 0)
        .filter(|record| !record.is_empty());
    while let Some(record) = records.next() {
        let (kind, rest) = record.split_at(1);
        match kind {
            b"1" | b"2" => {
                // 1 XY sub mH mI mW hH hI path
                // 2 XY sub mH mI mW hH hI Xscore path \0 origPath
                let fields = if kind == b"1" { 8 } else { 9 };
                let mut parts = rest[1..].splitn(fields, |b| *b == b' ');
                let xy = parts.next().ok_or_else(malformed)?;
                let mode_head = parts.nth(1).ok_or_else(malformed)?;
                let mode_index = parts.next().ok_or_else(malformed)?;
                let mode_worktree = parts.next().ok_or_else(malformed)?;
                let path: RepoPath = parts.last().ok_or_else(malformed)?.as_bstr().into();
                let old_path: Option<RepoPath> = if kind == b"2" {
                    Some(records.next().ok_or_else(malformed)?.as_bstr().into())
                } else {
                    None
                };
                if let Some(kind) = porcelain_kind(xy[0], mode_head, mode_index) {
                    status.staged.push(StatusEntry {
                        path: path.clone(),
                        old_path: old_path.clone(),
                        kind,
                    });
                }
                if let Some(kind) = porcelain_kind(xy[1], mode_index, mode_worktree) {
                    status.unstaged.push(StatusEntry {
                        path,
                        old_path: None,
                        kind,
                    });
                }
            }
            b"u" => {
                let path = rest[1..]
                    .splitn(10, |b| *b == b' ')
                    .last()
                    .ok_or_else(malformed)?;
                status.conflicted.push(path.as_bstr().into());
            }
            b"?" => status.unstaged.push(StatusEntry {
                path: rest[1..].as_bstr().into(),
                old_path: None,
                kind: ChangeKind::Untracked,
            }),
            _ => {}
        }
    }
    Ok(status)
}

fn porcelain_kind(code: u8, old_mode: &[u8], new_mode: &[u8]) -> Option<ChangeKind> {
    Some(match code {
        b'M' => ChangeKind::Modified,
        b'T' => {
            let kind = |mode: &[u8]| {
                u32::from_str_radix(mode.to_str().ok()?, 8)
                    .ok()
                    .map(crate::diff::mode_to_entry_kind)
            };
            if is_type_change(kind(old_mode), kind(new_mode)) {
                ChangeKind::TypeChanged
            } else {
                ChangeKind::Modified
            }
        }
        b'A' => ChangeKind::Added,
        b'D' => ChangeKind::Deleted,
        b'R' => ChangeKind::Renamed,
        b'C' => ChangeKind::Copied,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_porcelain_records() {
        let output = b"1 M. N... 100644 100644 100644 aaaa bbbb staged.txt\0\
1 .M N... 100644 100644 100644 aaaa aaaa unstaged file.txt\0\
2 R. N... 100644 100644 100644 aaaa aaaa R100 new.txt\0old.txt\0\
u UU N... 100644 100644 100644 100644 aaaa bbbb cccc conflict.txt\0\
? untracked.txt\0";
        let status = parse_porcelain_v2(output).unwrap();
        let staged: Vec<_> = status
            .staged
            .iter()
            .map(|e| (e.path.to_string(), e.kind))
            .collect();
        assert_eq!(
            staged,
            [
                ("staged.txt".into(), ChangeKind::Modified),
                ("new.txt".into(), ChangeKind::Renamed)
            ]
        );
        assert_eq!(status.staged[1].old_path, Some("old.txt".into()));
        let unstaged: Vec<_> = status
            .unstaged
            .iter()
            .map(|e| (e.path.to_string(), e.kind))
            .collect();
        assert_eq!(
            unstaged,
            [
                ("unstaged file.txt".into(), ChangeKind::Modified),
                ("untracked.txt".into(), ChangeKind::Untracked)
            ]
        );
        assert_eq!(status.conflicted, [RepoPath::from("conflict.txt")]);
    }
}
