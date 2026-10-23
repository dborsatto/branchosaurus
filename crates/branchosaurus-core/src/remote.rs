// SPDX-License-Identifier: GPL-3.0-or-later

//! Fetch, pull and push, through the git CLI so credential helpers, SSH configuration and hooks
//! all apply.

use std::sync::Arc;

use gix::bstr::ByteSlice;

use crate::askpass::CredentialPrompter;
use crate::cli::Streaming;
use crate::error::Context;
use crate::{CancelToken, Error, Repository, Result};

/// One step of a remote operation, as git reports it (`Receiving objects:  45% (123/456)`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteProgress {
    /// What git is doing, e.g. `Receiving objects`.
    pub phase: String,
    pub percent: Option<u8>,
    /// The whole line git printed.
    pub message: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PullMode {
    /// Whatever the repository's configuration says (`pull.rebase`, `pull.ff`).
    #[default]
    Default,
    FastForwardOnly,
    Merge,
    Rebase,
}

/// How a remote operation reports progress, asks for credentials and can be stopped.
pub struct RemoteOptions<'a> {
    pub progress: &'a mut dyn FnMut(&RemoteProgress),
    /// Asked when git needs a username, password or passphrase that no credential helper has.
    /// Without one, such operations fail instead of prompting.
    pub credentials: Option<Arc<dyn CredentialPrompter>>,
    pub cancel: &'a CancelToken,
}

impl std::fmt::Debug for RemoteOptions<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemoteOptions")
            .field("credentials", &self.credentials.is_some())
            .finish_non_exhaustive()
    }
}

/// Where a push went.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PushTarget {
    pub remote: String,
    /// The branch on the remote, e.g. `refs/heads/main`.
    pub remote_ref: String,
    /// The push also made it the branch's upstream.
    pub set_upstream: bool,
}

impl Repository {
    /// Names of the configured remotes.
    pub fn remotes(&self) -> Result<Vec<String>> {
        let repo = self.local();
        Ok(repo
            .remote_names()
            .into_iter()
            .map(|name| name.to_str_lossy().into_owned())
            .collect())
    }

    /// Fetches `remote`, or every remote when `None`, pruning branches deleted on the remote.
    pub fn fetch(&self, remote: Option<&str>, options: RemoteOptions<'_>) -> Result<()> {
        let mut args = vec!["fetch", "--progress", "--prune"];
        match remote {
            Some(remote) => args.push(remote),
            None => args.push("--all"),
        }
        self.run_remote(&args, options)
    }

    /// Pulls the current branch's upstream into it.
    pub fn pull(&self, mode: PullMode, options: RemoteOptions<'_>) -> Result<()> {
        self.require_workdir()?;
        let mut args = vec!["pull", "--progress"];
        match mode {
            PullMode::Default => {}
            PullMode::FastForwardOnly => args.push("--ff-only"),
            PullMode::Merge => args.push("--no-rebase"),
            PullMode::Rebase => args.push("--rebase"),
        }
        self.run_remote(&args, options)
    }

    /// Pushes the current branch to its upstream. A branch without an upstream is pushed to its
    /// push remote (`branch.<name>.pushRemote`, `remote.pushDefault`, the only remote, or
    /// `origin`) under the same name, and that becomes its upstream. `force` uses
    /// `--force-with-lease`, which refuses to overwrite commits you haven't fetched.
    pub fn push(&self, force: bool, options: RemoteOptions<'_>) -> Result<PushTarget> {
        let target = self.push_target()?;
        let refspec = format!("{}:{}", self.current_branch_ref()?, target.remote_ref);
        let mut args = vec!["push", "--progress"];
        if force {
            args.push("--force-with-lease");
        }
        if target.set_upstream {
            args.push("--set-upstream");
        }
        args.push(&target.remote);
        args.push(&refspec);
        self.run_remote(&args, options)?;
        Ok(target)
    }

    /// Where [`push`](Self::push) would push the current branch.
    pub fn push_target(&self) -> Result<PushTarget> {
        let repo = self.local();
        let branch_ref = self.current_branch_ref()?;
        let branch = branch_ref.trim_start_matches("refs/heads/").to_owned();
        let config = repo.config_snapshot();
        let setting = |key: &str| {
            config
                .string(format!("branch.{branch}.{key}").as_str())
                .map(|value| value.to_str_lossy().into_owned())
        };
        if let (Some(remote), Some(merge)) = (setting("remote"), setting("merge"))
            && remote != "."
        {
            return Ok(PushTarget {
                remote,
                remote_ref: merge,
                set_upstream: false,
            });
        }
        let remote = setting("pushRemote")
            .or_else(|| {
                repo.remote_default_name(gix::remote::Direction::Push)
                    .map(|name| name.to_str_lossy().into_owned())
            })
            .or_else(|| {
                let names = repo.remote_names();
                names
                    .iter()
                    .find(|name| name.as_bytes() == b"origin")
                    .or_else(|| names.iter().next())
                    .map(|name| name.to_str_lossy().into_owned())
            })
            .ok_or_else(|| Error::Unsupported("This repository has no remote to push to".into()))?;
        Ok(PushTarget {
            remote,
            remote_ref: branch_ref,
            set_upstream: true,
        })
    }

    fn current_branch_ref(&self) -> Result<String> {
        let repo = self.local();
        let head = repo.head_name().context("reading HEAD")?.ok_or_else(|| {
            Error::Unsupported("Check out a branch first: HEAD is detached".into())
        })?;
        Ok(head.as_bstr().to_str_lossy().into_owned())
    }

    /// Writes git's commit-graph file, which makes loading history several times faster.
    pub fn write_commit_graph(&self, cancel: &CancelToken) -> Result<()> {
        let dir = self.workdir().unwrap_or_else(|| self.git_dir());
        self.git()?
            .run_streaming(
                dir,
                ["commit-graph", "write", "--reachable"],
                Streaming {
                    env: &[],
                    stdin: None,
                    on_stderr_line: &mut |_| {},
                    cancel,
                },
            )
            .map(drop)
    }

    fn run_remote(&self, args: &[&str], options: RemoteOptions<'_>) -> Result<()> {
        let dir = self.workdir().unwrap_or_else(|| self.git_dir());
        #[cfg(unix)]
        let server = match (&options.credentials, self.askpass_helper()) {
            (Some(prompter), Some(_)) => {
                Some(crate::askpass::AskpassServer::start(prompter.clone())?)
            }
            _ => None,
        };
        #[cfg(unix)]
        let env = match (&server, self.askpass_helper()) {
            (Some(server), Some(helper)) => server.env(helper),
            _ => Vec::new(),
        };
        #[cfg(not(unix))]
        let env = Vec::new();
        let progress = options.progress;
        self.git()?
            .run_streaming(
                dir,
                args,
                Streaming {
                    env: &env,
                    stdin: None,
                    on_stderr_line: &mut |line| {
                        if let Some(update) = parse_progress(line) {
                            progress(&update);
                        }
                    },
                    cancel: options.cancel,
                },
            )
            .map(drop)
    }
}

/// Parses git's progress lines: `Receiving objects:  45% (123/456), 1.2 MiB | 3 MiB/s`,
/// `remote: Counting objects: 100% (10/10), done.`, `remote: Enumerating objects: 5, done.`
pub(crate) fn parse_progress(line: &str) -> Option<RemoteProgress> {
    let message = line.trim();
    let body = message
        .strip_prefix("remote:")
        .unwrap_or(message)
        .trim_start();
    let (phase, rest) = body.split_once(':')?;
    let rest = rest.trim_start();
    let is_phase = !phase.is_empty() && phase.chars().all(|c| c.is_ascii_alphabetic() || c == ' ');
    if !is_phase || !rest.starts_with(|c: char| c.is_ascii_digit()) {
        return None;
    }
    let percent = rest
        .split_once('%')
        .and_then(|(number, _)| number.trim().parse::<u8>().ok())
        .filter(|percent| *percent <= 100);
    Some(RemoteProgress {
        phase: phase.trim().to_owned(),
        percent,
        message: message.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::parse_progress;

    #[test]
    fn parses_git_progress() {
        let receiving =
            parse_progress("Receiving objects:  45% (123/456), 1.20 MiB | 2.00 MiB/s").unwrap();
        assert_eq!(receiving.phase, "Receiving objects");
        assert_eq!(receiving.percent, Some(45));

        let counting = parse_progress("remote: Counting objects: 100% (10/10), done.").unwrap();
        assert_eq!(counting.phase, "Counting objects");
        assert_eq!(counting.percent, Some(100));

        let enumerating = parse_progress("remote: Enumerating objects: 5, done.").unwrap();
        assert_eq!(enumerating.phase, "Enumerating objects");
        assert_eq!(enumerating.percent, None);

        assert_eq!(parse_progress("From github.com:ada/engine"), None);
        assert_eq!(
            parse_progress(" * [new branch]      main -> origin/main"),
            None
        );
        assert_eq!(parse_progress("fatal: could not read Username"), None);
    }
}
