// SPDX-License-Identifier: GPL-3.0-or-later

//! The gitoxide status must agree with `git status --porcelain=v2`, which the core also uses as
//! its fallback.

mod support;

use branchosaurus_core::{ChangeKind, RepoPath, StatusBackend, StatusEntry, WorkingCopyStatus};
use support::Fixture;

fn both_backends(fixture: &Fixture) -> WorkingCopyStatus {
    let repo = fixture.repo();
    let gix = repo
        .status_with(StatusBackend::Gitoxide)
        .expect("gitoxide status");
    let cli = repo.status_with(StatusBackend::GitCli).expect("git status");
    assert_eq!(gix, cli, "gitoxide and git disagree");
    gix
}

fn entry(path: &str, kind: ChangeKind) -> StatusEntry {
    StatusEntry {
        path: path.into(),
        old_path: None,
        kind,
    }
}

#[test]
fn clean_working_copy() {
    let fixture = Fixture::new();
    fixture.write("a.txt", "a\n");
    fixture.commit_all("initial");
    assert_eq!(both_backends(&fixture), WorkingCopyStatus::default());
}

#[test]
fn staged_unstaged_and_untracked() {
    let fixture = Fixture::new();
    for name in [
        "modified.txt",
        "staged.txt",
        "both.txt",
        "deleted.txt",
        "staged-deleted.txt",
    ] {
        fixture.write(name, "original\n");
    }
    fixture.commit_all("initial");

    fixture.write("modified.txt", "changed\n");
    fixture.write("staged.txt", "changed\n");
    fixture.git(&["add", "staged.txt"]);
    fixture.write("both.txt", "staged change\n");
    fixture.git(&["add", "both.txt"]);
    fixture.write("both.txt", "staged change\nand an unstaged one\n");
    fixture.remove("deleted.txt");
    fixture.git(&["rm", "--quiet", "staged-deleted.txt"]);
    fixture.write("new-staged.txt", "new\n");
    fixture.git(&["add", "new-staged.txt"]);
    fixture.write("nested/dir/untracked.txt", "?\n");
    fixture.write("untracked with spaces.txt", "?\n");

    let status = both_backends(&fixture);
    assert_eq!(
        status.staged,
        [
            entry("both.txt", ChangeKind::Modified),
            entry("new-staged.txt", ChangeKind::Added),
            entry("staged-deleted.txt", ChangeKind::Deleted),
            entry("staged.txt", ChangeKind::Modified),
        ]
    );
    assert_eq!(
        status.unstaged,
        [
            entry("both.txt", ChangeKind::Modified),
            entry("deleted.txt", ChangeKind::Deleted),
            entry("modified.txt", ChangeKind::Modified),
            entry("nested/dir/untracked.txt", ChangeKind::Untracked),
            entry("untracked with spaces.txt", ChangeKind::Untracked),
        ]
    );
    assert!(status.conflicted.is_empty());
}

#[test]
fn ignored_files_are_not_listed() {
    let fixture = Fixture::new();
    fixture.write(".gitignore", "*.log\nbuild/\n");
    fixture.commit_all("initial");
    fixture.write("debug.log", "noise\n");
    fixture.write("build/output.bin", "noise\n");
    fixture.write("keep.txt", "signal\n");
    let status = both_backends(&fixture);
    assert_eq!(status.unstaged, [entry("keep.txt", ChangeKind::Untracked)]);
}

#[test]
fn staged_rename() {
    let fixture = Fixture::new();
    fixture.write(
        "old name.txt",
        "the same content on several lines\n".repeat(10),
    );
    fixture.commit_all("initial");
    fixture.git(&["mv", "old name.txt", "new name.txt"]);
    let status = both_backends(&fixture);
    assert_eq!(
        status.staged,
        [StatusEntry {
            path: "new name.txt".into(),
            old_path: Some("old name.txt".into()),
            kind: ChangeKind::Renamed,
        }]
    );
}

#[test]
fn merge_conflict() {
    let fixture = Fixture::new();
    fixture.write("shared.txt", "base\n");
    fixture.write("other.txt", "base\n");
    fixture.commit_all("base");
    fixture.git(&["switch", "--quiet", "-c", "theirs"]);
    fixture.write("shared.txt", "theirs\n");
    fixture.write("other.txt", "merged cleanly\n");
    fixture.commit_all("theirs");
    fixture.git(&["switch", "--quiet", "main"]);
    fixture.write("shared.txt", "ours\n");
    fixture.commit_all("ours");
    let merge = fixture.try_git(&["merge", "theirs"]);
    assert!(!merge.status.success(), "the merge must conflict");

    let status = both_backends(&fixture);
    assert_eq!(status.conflicted, [RepoPath::from("shared.txt")]);
    assert_eq!(status.staged, [entry("other.txt", ChangeKind::Modified)]);
    let info = fixture.repo().info().unwrap();
    assert_eq!(info.state, Some(branchosaurus_core::RepositoryState::Merge));
}

#[cfg(unix)]
#[test]
fn symlinks_and_executable_bit() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = Fixture::new();
    fixture.write("script.sh", "echo hi\n");
    fixture.write("target.txt", "target\n");
    fixture.write("link", "not a link yet\n");
    fixture.commit_all("initial");

    let script = fixture.path().join("script.sh");
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    fixture.remove("link");
    std::os::unix::fs::symlink("target.txt", fixture.path().join("link")).unwrap();

    let status = both_backends(&fixture);
    assert_eq!(
        status.unstaged,
        [
            entry("link", ChangeKind::TypeChanged),
            entry("script.sh", ChangeKind::Modified),
        ]
    );
}

/// macOS and Windows file systems reject such names, so only Linux can check this.
#[cfg(target_os = "linux")]
#[test]
fn non_utf8_path() {
    use std::os::unix::ffi::OsStrExt;
    let fixture = Fixture::new();
    fixture.commit("initial");
    let name = std::ffi::OsStr::from_bytes(b"caf\xe9.txt");
    std::fs::write(fixture.path().join(name), "latin-1 name\n").unwrap();
    let status = both_backends(&fixture);
    assert_eq!(status.unstaged.len(), 1);
    assert_eq!(status.unstaged[0].path.as_bytes(), b"caf\xe9.txt");
    assert_eq!(status.unstaged[0].path.display(), "caf\u{fffd}.txt");
}
