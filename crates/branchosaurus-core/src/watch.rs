// SPDX-License-Identifier: GPL-3.0-or-later

//! Notices when the working tree, the index or the refs change, so the UI can refresh itself.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};

use notify::{RecommendedWatcher, RecursiveMode, Watcher};

use crate::error::Context;
use crate::{Error, Repository, Result};

/// Changes are reported once things have been quiet this long…
const QUIET_PERIOD: Duration = Duration::from_millis(250);
/// …or after this long, during a continuous stream of changes (a build writing files, say).
const MAX_DELAY: Duration = Duration::from_secs(2);

/// What changed since the last report.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WatchEvent {
    /// Files in the working tree (ignored files excluded).
    pub working_copy: bool,
    /// The staging area.
    pub index: bool,
    /// Branches, tags, remote branches, HEAD, or an operation in progress (merge, rebase…).
    pub refs: bool,
}

impl WatchEvent {
    fn is_empty(&self) -> bool {
        !(self.working_copy || self.index || self.refs)
    }

    fn merge(&mut self, other: WatchEvent) {
        self.working_copy |= other.working_copy;
        self.index |= other.index;
        self.refs |= other.refs;
    }
}

/// Watches a repository until dropped.
pub struct RepositoryWatcher {
    _watcher: RecommendedWatcher,
}

impl std::fmt::Debug for RepositoryWatcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RepositoryWatcher").finish_non_exhaustive()
    }
}

impl Repository {
    /// Calls `on_change` (from a background thread) whenever the repository changes, a short
    /// while after the changes settle.
    pub fn watch(
        &self,
        on_change: impl Fn(WatchEvent) + Send + 'static,
    ) -> Result<RepositoryWatcher> {
        let canonical = |path: &Path| path.canonicalize().unwrap_or_else(|_| path.to_owned());
        let git_dir = canonical(self.git_dir());
        let workdir = self.workdir().map(canonical);

        let (sender, receiver) = mpsc::channel::<Vec<PathBuf>>();
        let mut watcher =
            notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                if let Ok(event) = event
                    && !event.kind.is_access()
                {
                    let _ = sender.send(event.paths);
                }
            })
            .map_err(|err| Error::Io {
                context: "watching the repository",
                source: std::io::Error::other(err.to_string()),
            })?;
        let watch = |watcher: &mut RecommendedWatcher, path: &Path| {
            watcher
                .watch(path, RecursiveMode::Recursive)
                .map_err(|err| Error::Io {
                    context: "watching the repository",
                    source: std::io::Error::other(err.to_string()),
                })
        };
        if let Some(workdir) = &workdir {
            watch(&mut watcher, workdir)?;
        }
        if workdir
            .as_ref()
            .is_none_or(|workdir| !git_dir.starts_with(workdir))
        {
            watch(&mut watcher, &git_dir)?;
        }

        let repository = self.clone();
        std::thread::spawn(move || {
            let classifier = Classifier {
                repository,
                git_dir,
                workdir,
            };
            debounce(&receiver, &classifier, &on_change);
        });
        Ok(RepositoryWatcher { _watcher: watcher })
    }
}

/// Collects changes until they settle, then reports them. Ends when the watcher is dropped.
fn debounce(
    receiver: &mpsc::Receiver<Vec<PathBuf>>,
    classifier: &Classifier,
    on_change: &dyn Fn(WatchEvent),
) {
    loop {
        // Wait for something to happen.
        let Ok(paths) = receiver.recv() else { return };
        let started = Instant::now();
        let mut pending = paths;
        loop {
            match receiver.recv_timeout(QUIET_PERIOD) {
                Ok(paths) => {
                    pending.extend(paths);
                    if started.elapsed() >= MAX_DELAY {
                        break;
                    }
                }
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
        let event = classifier.classify(&pending);
        if !event.is_empty() {
            on_change(event);
        }
    }
}

struct Classifier {
    repository: Repository,
    git_dir: PathBuf,
    workdir: Option<PathBuf>,
}

impl Classifier {
    fn classify(&self, paths: &[PathBuf]) -> WatchEvent {
        let mut event = WatchEvent::default();
        let mut worktree_paths = Vec::new();
        for path in paths {
            if let Ok(inside_git_dir) = path.strip_prefix(&self.git_dir) {
                event.merge(classify_git_dir_path(inside_git_dir));
            } else if let Some(relative) = self
                .workdir
                .as_ref()
                .and_then(|workdir| path.strip_prefix(workdir).ok())
            {
                worktree_paths.push(relative.to_owned());
            }
        }
        if !worktree_paths.is_empty() {
            event.working_copy = self.any_not_ignored(&worktree_paths).unwrap_or(true);
        }
        event
    }

    /// Whether any of these paths matters, i.e. isn't excluded by `.gitignore` and friends.
    fn any_not_ignored(&self, paths: &[PathBuf]) -> Result<bool> {
        let repo = self.repository.local();
        let index = repo.index_or_empty().context("reading the index")?;
        let mut excludes = repo
            .excludes(&index, None, Default::default())
            .context("reading ignore rules")?;
        for path in paths {
            let is_dir = self
                .workdir
                .as_ref()
                .is_some_and(|workdir| workdir.join(path).is_dir());
            let mode = is_dir.then_some(gix::index::entry::Mode::DIR);
            let platform = excludes
                .at_path(path, mode)
                .context("checking ignore rules")?;
            if !platform.is_excluded() {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

/// What a change inside `.git` means. Object and log writes, and lock files, are noise.
fn classify_git_dir_path(path: &Path) -> WatchEvent {
    let mut event = WatchEvent::default();
    let Some(first) = path.components().next() else {
        return event;
    };
    let first = first.as_os_str().to_string_lossy();
    let is_lock = path
        .extension()
        .is_some_and(|extension| extension == "lock");
    match first.as_ref() {
        _ if is_lock => {}
        "objects" | "logs" | "hooks" | "info" | "lfs" | "modules" => {}
        "index" => event.index = true,
        _ => event.refs = true,
    }
    event
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_git_dir_changes() {
        let refs = |path: &str| classify_git_dir_path(Path::new(path));
        assert!(refs("HEAD").refs);
        assert!(refs("refs/heads/main").refs);
        assert!(refs("packed-refs").refs);
        assert!(refs("MERGE_HEAD").refs);
        assert!(refs("index").index);
        assert!(refs("index.lock").is_empty());
        assert!(refs("refs/heads/main.lock").is_empty());
        assert!(refs("objects/ab/cdef").is_empty());
        assert!(refs("logs/HEAD").is_empty());
    }
}
