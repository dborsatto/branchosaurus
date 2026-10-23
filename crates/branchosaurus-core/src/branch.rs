// SPDX-License-Identifier: GPL-3.0-or-later

//! Switching and creating branches.

use gix::ObjectId;

use crate::{Error, Repository, Result};

/// What to check out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CheckoutTarget {
    /// A local branch, by short name (`feature/login`).
    LocalBranch(String),
    /// A remote-tracking branch, by short name (`origin/feature/login`). Creates a local branch
    /// with the same name that tracks it.
    RemoteBranch(String),
    /// A commit, leaving HEAD detached.
    Commit(ObjectId),
}

/// Whether `name` is a valid name for a new branch.
pub fn is_valid_branch_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('-')
        && name != "HEAD"
        && gix::refs::FullName::try_from(format!("refs/heads/{name}")).is_ok()
}

impl Repository {
    pub fn checkout(&self, target: &CheckoutTarget) -> Result<()> {
        self.require_workdir()?;
        match target {
            CheckoutTarget::LocalBranch(name) => {
                self.run_git(["switch", "--no-guess", name.as_str()], None)
            }
            CheckoutTarget::RemoteBranch(name) => {
                self.run_git(["switch", "--track", name.as_str()], None)
            }
            CheckoutTarget::Commit(id) => {
                self.run_git(["switch", "--detach", &id.to_string()], None)
            }
        }
        .map(drop)
    }

    /// Creates a branch at `start` (HEAD if `None`), and switches to it if `checkout` is set.
    pub fn create_branch(&self, name: &str, start: Option<ObjectId>, checkout: bool) -> Result<()> {
        if !is_valid_branch_name(name) {
            return Err(Error::Unsupported(format!(
                "“{name}” is not a valid branch name"
            )));
        }
        let start = start.map(|id| id.to_string());
        let mut args = if checkout {
            self.require_workdir()?;
            vec!["switch", "--create", name]
        } else {
            vec!["branch", "--", name]
        };
        if let Some(start) = &start {
            args.push(start);
        }
        self.run_git(args, None).map(drop)
    }
}

#[cfg(test)]
mod tests {
    use super::is_valid_branch_name;

    #[test]
    fn validates_branch_names() {
        for valid in ["main", "feature/login", "fix-123", "release/v1.2"] {
            assert!(is_valid_branch_name(valid), "{valid}");
        }
        for invalid in [
            "",
            "-x",
            "HEAD",
            "a..b",
            "with space",
            "end.lock",
            "trailing/",
            "a~b",
        ] {
            assert!(!is_valid_branch_name(invalid), "{invalid}");
        }
    }
}
