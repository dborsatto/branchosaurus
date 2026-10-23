// SPDX-License-Identifier: GPL-3.0-or-later

use gix::ObjectId;
use gix::bstr::ByteSlice;

use crate::error::Context;
use crate::repo::{Head, head};
use crate::{Repository, Result};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Refs {
    pub head: Option<Head>,
    pub local_branches: Vec<LocalBranch>,
    pub remote_branches: Vec<RemoteBranch>,
    pub tags: Vec<Tag>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalBranch {
    /// Short name, e.g. `feature/login`.
    pub name: String,
    pub full_name: String,
    pub target: ObjectId,
    /// Short name of the remote-tracking branch this branch pulls from, e.g. `origin/main`.
    pub upstream: Option<String>,
    pub is_head: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteBranch {
    pub remote: String,
    /// Short name including the remote, e.g. `origin/main`.
    pub name: String,
    pub full_name: String,
    pub target: ObjectId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tag {
    pub name: String,
    pub full_name: String,
    /// The object the tag points to after peeling annotated tags, usually a commit.
    pub target: ObjectId,
}

impl Repository {
    pub fn refs(&self) -> Result<Refs> {
        let repo = self.local();
        let head = head(&repo).ok();
        let head_branch = match &head {
            Some(Head::Branch { name, .. }) => Some(name.clone()),
            _ => None,
        };
        let platform = repo.references().context("reading references")?;

        let mut local_branches = Vec::new();
        for reference in platform.local_branches().context("reading branches")? {
            let mut reference = reference.context("reading a branch")?;
            let Ok(target) = reference.peel_to_id() else {
                continue;
            };
            let name = reference.name().shorten().to_str_lossy().into_owned();
            let upstream = repo
                .branch_remote_tracking_ref_name(reference.name(), gix::remote::Direction::Fetch)
                .and_then(Result::ok)
                .map(|name| name.as_ref().shorten().to_str_lossy().into_owned());
            local_branches.push(LocalBranch {
                is_head: head_branch.as_deref() == Some(name.as_str()),
                full_name: reference.name().as_bstr().to_str_lossy().into_owned(),
                name,
                target: target.detach(),
                upstream,
            });
        }

        let mut remote_branches = Vec::new();
        for reference in platform
            .remote_branches()
            .context("reading remote branches")?
        {
            let mut reference = reference.context("reading a remote branch")?;
            // `refs/remotes/origin/HEAD` only says which branch is the remote's default.
            if reference.name().as_bstr().ends_with(b"/HEAD") {
                continue;
            }
            let Ok(target) = reference.peel_to_id() else {
                continue;
            };
            let name = reference.name().shorten().to_str_lossy().into_owned();
            let remote = name.split('/').next().unwrap_or_default().to_owned();
            remote_branches.push(RemoteBranch {
                remote,
                full_name: reference.name().as_bstr().to_str_lossy().into_owned(),
                name,
                target: target.detach(),
            });
        }

        let mut tags = Vec::new();
        for reference in platform.tags().context("reading tags")? {
            let mut reference = reference.context("reading a tag")?;
            let Ok(target) = reference.peel_to_id() else {
                continue;
            };
            tags.push(Tag {
                name: reference.name().shorten().to_str_lossy().into_owned(),
                full_name: reference.name().as_bstr().to_str_lossy().into_owned(),
                target: target.detach(),
            });
        }

        local_branches.sort_by(|a, b| natural_cmp(&a.name, &b.name));
        remote_branches.sort_by(|a, b| natural_cmp(&a.name, &b.name));
        tags.sort_by(|a, b| natural_cmp(&a.name, &b.name));

        Ok(Refs {
            head,
            local_branches,
            remote_branches,
            tags,
        })
    }
}

/// Orders `v1.10` after `v1.9`, the way people expect branch and tag lists to be sorted.
fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let mut a = a.chars().peekable();
    let mut b = b.chars().peekable();
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return std::cmp::Ordering::Equal,
            (None, Some(_)) => return std::cmp::Ordering::Less,
            (Some(_), None) => return std::cmp::Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let take_number = |it: &mut std::iter::Peekable<std::str::Chars<'_>>| {
                    let mut digits = String::new();
                    while let Some(c) = it.peek().copied().filter(char::is_ascii_digit) {
                        digits.push(c);
                        it.next();
                    }
                    digits
                };
                let (x, y) = (take_number(&mut a), take_number(&mut b));
                let (xt, yt) = (x.trim_start_matches('0'), y.trim_start_matches('0'));
                let ordering = xt.len().cmp(&yt.len()).then_with(|| xt.cmp(yt));
                if ordering.is_ne() {
                    return ordering;
                }
            }
            (Some(x), Some(y)) => {
                let ordering = x
                    .to_lowercase()
                    .cmp(y.to_lowercase())
                    .then_with(|| x.cmp(&y));
                if ordering.is_ne() {
                    return ordering;
                }
                a.next();
                b.next();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::natural_cmp;

    #[test]
    fn sorts_numbers_naturally() {
        let mut names = vec!["v1.10", "v1.9", "main", "Feature", "v1.2", "develop"];
        names.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(
            names,
            ["develop", "Feature", "main", "v1.2", "v1.9", "v1.10"]
        );
    }
}
