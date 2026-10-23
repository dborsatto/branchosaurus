// SPDX-License-Identifier: GPL-3.0-or-later

//! Whole-file staging, unstaging and discarding, and committing. Everything here runs the git
//! CLI, so hooks, signing and configuration behave as they do in a terminal.

use std::path::Path;

use gix::ObjectId;

use crate::diff::ChangeKind;
use crate::error::Context;
use crate::{Error, RepoPath, Repository, Result, StatusEntry};

/// What to do with untracked files when discarding them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UntrackedRemoval {
    /// Move them to the system trash, where they can be recovered.
    MoveToTrash,
    /// Delete them permanently.
    Delete,
}

impl Repository {
    /// Stages every change to these files: modifications, deletions and new files.
    pub fn stage(&self, entries: &[StatusEntry]) -> Result<()> {
        self.git_with_pathspecs(&["add", "--all"], entries.iter().map(|entry| &entry.path))
    }

    pub fn stage_all(&self) -> Result<()> {
        self.require_workdir()?;
        self.run_git(["add", "--all"], None).map(drop)
    }

    /// Removes these files' staged changes from the index, keeping the working tree as it is.
    pub fn unstage(&self, entries: &[StatusEntry]) -> Result<()> {
        // A staged rename is a deletion of the old path plus an addition of the new one.
        let paths = entries
            .iter()
            .flat_map(|entry| std::iter::once(&entry.path).chain(entry.old_path.as_ref()));
        if self.has_commits()? {
            self.git_with_pathspecs(&["reset", "--quiet"], paths)
        } else {
            self.git_with_pathspecs(&["rm", "--cached", "--quiet", "-r"], paths)
        }
    }

    pub fn unstage_all(&self) -> Result<()> {
        self.require_workdir()?;
        if self.has_commits()? {
            self.run_git(["reset", "--quiet"], None).map(drop)
        } else {
            self.run_git(
                ["rm", "--cached", "--quiet", "-r", "--ignore-unmatch", "."],
                None,
            )
            .map(drop)
        }
    }

    /// Throws away the unstaged changes to these files: tracked files go back to their staged
    /// version, untracked files are removed.
    pub fn discard(&self, entries: &[StatusEntry], untracked: UntrackedRemoval) -> Result<()> {
        let workdir = self.require_workdir()?;
        let (new_files, tracked): (Vec<&StatusEntry>, Vec<&StatusEntry>) = entries
            .iter()
            .partition(|entry| entry.kind == ChangeKind::Untracked);
        if !tracked.is_empty() {
            self.git_with_pathspecs(
                &["restore", "--worktree"],
                tracked.iter().map(|entry| &entry.path),
            )?;
        }
        let files: Vec<_> = new_files
            .iter()
            .map(|entry| worktree_path(workdir, &entry.path))
            .collect::<Result<_>>()?;
        if files.is_empty() {
            return Ok(());
        }
        match untracked {
            UntrackedRemoval::MoveToTrash => move_to_trash(&files),
            UntrackedRemoval::Delete => {
                for file in files {
                    std::fs::remove_file(&file).map_err(|source| Error::Io {
                        context: "deleting an untracked file",
                        source,
                    })?;
                }
                Ok(())
            }
        }
    }

    /// Commits what is staged, or with `amend` replaces the last commit. Returns the new commit.
    pub fn commit(&self, message: &str, amend: bool) -> Result<ObjectId> {
        self.require_workdir()?;
        let mut args = vec!["commit", "--file=-"];
        if amend {
            args.push("--amend");
        }
        self.run_git(args, Some(message.as_bytes()))?;
        let repo = self.local();
        Ok(repo.head_id().context("reading the new commit")?.detach())
    }

    pub(crate) fn has_commits(&self) -> Result<bool> {
        let repo = self.local();
        Ok(repo.head().context("reading HEAD")?.id().is_some())
    }

    /// Runs a git command on exactly these paths. Paths go through stdin, so there is no limit on
    /// how many there are, and `--literal-pathspecs` stops git from treating `*` or `:` in file
    /// names as patterns.
    fn git_with_pathspecs<'a>(
        &self,
        args: &[&str],
        paths: impl Iterator<Item = &'a RepoPath>,
    ) -> Result<()> {
        self.require_workdir()?;
        let mut input = Vec::new();
        for path in paths {
            input.extend_from_slice(path.as_bytes());
            input.push(0);
        }
        if input.is_empty() {
            return Ok(());
        }
        let mut full_args = vec!["--literal-pathspecs"];
        full_args.extend_from_slice(args);
        full_args.extend(["--pathspec-from-file=-", "--pathspec-file-nul"]);
        self.run_git(full_args, Some(&input)).map(drop)
    }
}

pub(crate) fn worktree_path(workdir: &Path, path: &RepoPath) -> Result<std::path::PathBuf> {
    gix::path::from_bstr(path.as_bstr())
        .map(|relative| workdir.join(relative))
        .map_err(|_| Error::NotFound(format!("file {path}")))
}

fn move_to_trash(files: &[std::path::PathBuf]) -> Result<()> {
    #[allow(unused_mut)]
    let mut context = trash::TrashContext::default();
    #[cfg(target_os = "macos")]
    {
        use trash::macos::{DeleteMethod, TrashContextExtMacos};
        // The default asks Finder through AppleScript, which needs an automation permission.
        context.set_delete_method(DeleteMethod::NsFileManager);
    }
    context.delete_all(files).map_err(|err| Error::Io {
        context: "moving files to the trash",
        source: std::io::Error::other(err.to_string()),
    })
}
