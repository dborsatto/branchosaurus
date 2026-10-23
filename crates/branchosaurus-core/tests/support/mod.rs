// SPDX-License-Identifier: GPL-3.0-or-later

//! Throwaway repositories built with the real git CLI, isolated from the user's configuration so
//! that the core (opened in isolation too) and git see exactly the same settings.

#![allow(dead_code)]

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::process::Command;

use branchosaurus_core::{GitCli, Repository};
use tempfile::TempDir;

/// Commit dates start here and advance one minute per commit, so date order is unambiguous.
const EPOCH: u64 = 1_700_000_000;

pub struct Fixture {
    dir: TempDir,
    home: TempDir,
    clock: Cell<u64>,
}

impl Fixture {
    pub fn new() -> Self {
        let fixture = Self {
            dir: tempfile::tempdir().expect("temp dir"),
            home: tempfile::tempdir().expect("temp home"),
            clock: Cell::new(0),
        };
        std::fs::write(fixture.global_config(), "").expect("empty global config");
        fixture.git(&["init", "--quiet", "--initial-branch=main"]);
        fixture.git(&["config", "core.autocrlf", "false"]);
        fixture.git(&["config", "core.fsmonitor", "false"]);
        fixture.git(&["config", "commit.gpgsign", "false"]);
        fixture
    }

    pub fn path(&self) -> &Path {
        self.dir.path()
    }

    fn global_config(&self) -> PathBuf {
        self.home.path().join("gitconfig")
    }

    fn isolated_env(&self) -> Vec<(&'static str, String)> {
        vec![
            ("GIT_CONFIG_NOSYSTEM", "1".into()),
            (
                "GIT_CONFIG_GLOBAL",
                self.global_config().display().to_string(),
            ),
            ("HOME", self.home.path().display().to_string()),
            ("XDG_CONFIG_HOME", self.home.path().display().to_string()),
            ("GIT_AUTHOR_NAME", "Ada Lovelace".into()),
            ("GIT_AUTHOR_EMAIL", "ada@example.com".into()),
            ("GIT_COMMITTER_NAME", "Ada Lovelace".into()),
            ("GIT_COMMITTER_EMAIL", "ada@example.com".into()),
            ("LC_ALL", "C".into()),
        ]
    }

    /// Runs git in the fixture and returns its stdout.
    pub fn git(&self, args: &[&str]) -> String {
        String::from_utf8(self.git_bytes(args)).expect("utf-8 git output")
    }

    pub fn git_bytes(&self, args: &[&str]) -> Vec<u8> {
        let output = self.try_git(args);
        assert!(
            output.status.success(),
            "git {} failed:\n{}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
        output.stdout
    }

    /// Runs git in the fixture without requiring it to succeed.
    pub fn try_git(&self, args: &[&str]) -> std::process::Output {
        let tick = self.clock.get();
        self.clock.set(tick + 1);
        let date = format!("{} +0000", EPOCH + tick * 60);
        Command::new("git")
            .args(args)
            .current_dir(self.path())
            .envs(self.isolated_env())
            .env("GIT_AUTHOR_DATE", &date)
            .env("GIT_COMMITTER_DATE", &date)
            .output()
            .expect("run git")
    }

    pub fn write(&self, path: &str, contents: impl AsRef<[u8]>) {
        let path = self.path().join(path);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("create dirs");
        std::fs::write(path, contents).expect("write file");
    }

    pub fn remove(&self, path: &str) {
        std::fs::remove_file(self.path().join(path)).expect("remove file");
    }

    /// Stages everything and commits; returns the new commit's id.
    pub fn commit_all(&self, message: &str) -> String {
        self.git(&["add", "--all"]);
        self.commit(message)
    }

    /// Commits what is staged (or nothing); returns the new commit's id.
    pub fn commit(&self, message: &str) -> String {
        self.git(&["commit", "--quiet", "--allow-empty", "-m", message]);
        self.rev_parse("HEAD")
    }

    pub fn rev_parse(&self, rev: &str) -> String {
        self.git(&["rev-parse", rev]).trim().to_owned()
    }

    /// The repository as the core sees it, with git isolated the same way as the fixture.
    pub fn repo(&self) -> Repository {
        let mut git = GitCli::locate().expect("git is installed");
        for (key, value) in self.isolated_env() {
            git = git.with_env(key, value);
        }
        Repository::open_isolated(self.path())
            .expect("open fixture")
            .with_git(git)
            .with_askpass_program(env!("CARGO_BIN_EXE_branchosaurus-askpass"))
    }

    /// A bare copy of this repository in its own temporary directory, to use as a remote.
    pub fn bare_clone(&self) -> TempDir {
        let dir = tempfile::tempdir().expect("temp dir");
        let source = self.path().display().to_string();
        let target = dir.path().join("origin.git").display().to_string();
        self.git(&["clone", "--quiet", "--bare", &source, &target]);
        dir
    }

    pub fn read(&self, path: &str) -> String {
        std::fs::read_to_string(self.path().join(path)).expect("read file")
    }
}
