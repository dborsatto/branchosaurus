// SPDX-License-Identifier: GPL-3.0-or-later

//! Staging, committing and branches, checked against what git itself reports afterwards.

mod support;

use branchosaurus_core::{
    ChangeKind, CheckoutTarget, Error, Head, LineSelection, StatusEntry, UntrackedRemoval,
    parse_object_id,
};
use support::Fixture;

fn entry(path: &str, kind: ChangeKind) -> StatusEntry {
    StatusEntry {
        path: path.into(),
        old_path: None,
        kind,
    }
}

/// `git status --porcelain` lines, e.g. `M  staged.txt`.
fn porcelain(fixture: &Fixture) -> Vec<String> {
    fixture
        .git(&["status", "--porcelain", "--untracked-files=all"])
        .lines()
        .map(str::to_owned)
        .collect()
}

#[test]
fn stage_and_unstage_whole_files() {
    let fixture = Fixture::new();
    fixture.write("a.txt", "a\n");
    fixture.write("gone.txt", "bye\n");
    fixture.commit_all("initial");
    fixture.write("a.txt", "changed\n");
    fixture.remove("gone.txt");
    fixture.write("new * file.txt", "new\n");
    let repo = fixture.repo();

    repo.stage(&[
        entry("a.txt", ChangeKind::Modified),
        entry("gone.txt", ChangeKind::Deleted),
        entry("new * file.txt", ChangeKind::Untracked),
    ])
    .unwrap();
    assert_eq!(
        porcelain(&fixture),
        ["M  a.txt", "D  gone.txt", "A  \"new * file.txt\""]
    );

    repo.unstage(&[entry("a.txt", ChangeKind::Modified)])
        .unwrap();
    assert_eq!(
        porcelain(&fixture),
        [" M a.txt", "D  gone.txt", "A  \"new * file.txt\""]
    );

    repo.unstage_all().unwrap();
    repo.stage_all().unwrap();
    assert_eq!(porcelain(&fixture).len(), 3);
    assert!(
        porcelain(&fixture)
            .iter()
            .all(|line| !line.starts_with(' '))
    );
}

#[test]
fn unstage_works_before_the_first_commit() {
    let fixture = Fixture::new();
    fixture.write("first.txt", "1\n");
    fixture.write("second.txt", "2\n");
    let repo = fixture.repo();
    repo.stage_all().unwrap();
    repo.unstage(&[entry("first.txt", ChangeKind::Added)])
        .unwrap();
    assert_eq!(porcelain(&fixture), ["A  second.txt", "?? first.txt"]);
    repo.unstage_all().unwrap();
    assert_eq!(porcelain(&fixture), ["?? first.txt", "?? second.txt"]);
}

#[test]
fn unstaging_a_rename_restores_both_paths() {
    let fixture = Fixture::new();
    fixture.write(
        "old.txt",
        "content that makes the rename detectable\n".repeat(5),
    );
    fixture.commit_all("initial");
    fixture.git(&["mv", "old.txt", "new.txt"]);
    let repo = fixture.repo();
    let rename = StatusEntry {
        path: "new.txt".into(),
        old_path: Some("old.txt".into()),
        kind: ChangeKind::Renamed,
    };
    repo.unstage(&[rename]).unwrap();
    assert_eq!(porcelain(&fixture), [" D old.txt", "?? new.txt"]);
}

#[test]
fn discard_restores_tracked_and_removes_untracked() {
    let fixture = Fixture::new();
    fixture.write("tracked.txt", "original\n");
    fixture.commit_all("initial");
    fixture.write("tracked.txt", "edited\n");
    fixture.write("scratch.txt", "throwaway\n");
    let repo = fixture.repo();
    repo.discard(
        &[
            entry("tracked.txt", ChangeKind::Modified),
            entry("scratch.txt", ChangeKind::Untracked),
        ],
        UntrackedRemoval::Delete,
    )
    .unwrap();
    assert_eq!(fixture.read("tracked.txt"), "original\n");
    assert!(!fixture.path().join("scratch.txt").exists());
    assert!(porcelain(&fixture).is_empty());
}

#[test]
fn commit_and_amend() {
    let fixture = Fixture::new();
    fixture.write("a.txt", "a\n");
    let repo = fixture.repo();
    repo.stage_all().unwrap();
    let first = repo.commit("First commit\n\nWith a body.", false).unwrap();
    assert_eq!(first.to_string(), fixture.rev_parse("HEAD"));
    assert_eq!(
        fixture.git(&["log", "-1", "--format=%B"]).trim(),
        "First commit\n\nWith a body."
    );

    let amended = repo.commit("First commit, reworded", true).unwrap();
    assert_ne!(amended, first);
    assert_eq!(fixture.git(&["rev-list", "--count", "HEAD"]).trim(), "1");
    assert_eq!(
        fixture.git(&["log", "-1", "--format=%s"]).trim(),
        "First commit, reworded"
    );

    // Nothing staged: git refuses, and says why.
    let error = repo.commit("Empty", false).unwrap_err();
    assert!(
        matches!(error, Error::GitCommand { ref stderr, .. } if stderr.contains("nothing")),
        "{error}"
    );
}

#[test]
fn commit_runs_hooks() {
    let fixture = Fixture::new();
    fixture.write("a.txt", "a\n");
    let hook = fixture.path().join(".git/hooks/commit-msg");
    std::fs::write(
        &hook,
        "#!/bin/sh\necho 'message rejected by hook' >&2\nexit 1\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let repo = fixture.repo();
    repo.stage_all().unwrap();
    let error = repo.commit("Will be rejected", false).unwrap_err();
    assert!(
        error.to_string().contains("message rejected by hook"),
        "{error}"
    );
}

#[test]
fn branches() {
    let fixture = Fixture::new();
    let first = fixture.commit("first");
    fixture.commit("second");
    let repo = fixture.repo();

    repo.create_branch("feature/x", None, false).unwrap();
    assert_eq!(fixture.rev_parse("feature/x"), fixture.rev_parse("HEAD"));
    assert_eq!(repo.info().unwrap().head.branch_name(), Some("main"));

    repo.create_branch("from-first", Some(parse_object_id(&first).unwrap()), true)
        .unwrap();
    assert_eq!(repo.info().unwrap().head.branch_name(), Some("from-first"));
    assert_eq!(fixture.rev_parse("HEAD"), first);

    repo.checkout(&CheckoutTarget::LocalBranch("main".into()))
        .unwrap();
    assert_eq!(repo.info().unwrap().head.branch_name(), Some("main"));

    repo.checkout(&CheckoutTarget::Commit(parse_object_id(&first).unwrap()))
        .unwrap();
    assert!(matches!(repo.info().unwrap().head, Head::Detached { .. }));

    assert!(matches!(
        repo.create_branch("bad name", None, false),
        Err(Error::Unsupported(_))
    ));
}

#[test]
fn checkout_refuses_to_overwrite_local_changes() {
    let fixture = Fixture::new();
    fixture.write("shared.txt", "main\n");
    fixture.commit_all("main");
    fixture.git(&["switch", "--quiet", "-c", "other"]);
    fixture.write("shared.txt", "other\n");
    fixture.commit_all("other");
    fixture.git(&["switch", "--quiet", "main"]);
    fixture.write("shared.txt", "uncommitted\n");
    let error = fixture
        .repo()
        .checkout(&CheckoutTarget::LocalBranch("other".into()))
        .unwrap_err();
    assert!(
        error.to_string().contains("would be overwritten"),
        "{error}"
    );
    assert_eq!(fixture.read("shared.txt"), "uncommitted\n");
}

/// A file with changes in three separate places, for partial staging.
fn three_changes(fixture: &Fixture) {
    let original: String = (1..=30).map(|n| format!("line {n}\n")).collect();
    fixture.write("story.txt", &original);
    fixture.commit_all("initial");
    let edited = original
        .replace("line 2\n", "line two\n")
        .replace("line 15\n", "line 15\nline 15½\n")
        .replace("line 29\n", "");
    fixture.write("story.txt", edited);
}

#[test]
fn stage_unstage_and_discard_lines() {
    let fixture = Fixture::new();
    three_changes(&fixture);
    let repo = fixture.repo();
    let story = entry("story.txt", ChangeKind::Modified);

    // Stage only the inserted line 16 ("line 15½").
    repo.stage_lines(
        &story,
        &LineSelection {
            removed: vec![],
            added: vec![16],
        },
    )
    .unwrap();
    assert_eq!(
        fixture.git(&["diff", "--cached", "--numstat"]).trim(),
        "1\t0\tstory.txt"
    );
    assert_eq!(
        fixture.git(&["diff", "--numstat"]).trim(),
        "1\t2\tstory.txt"
    );
    let staged = fixture.git(&["show", ":story.txt"]);
    assert!(staged.contains("line 15\nline 15½\nline 16\n"));
    assert!(staged.contains("line 2\n") && staged.contains("line 29\n"));

    // Stage the replacement of line 2 too, then take the insertion back out.
    repo.stage_lines(
        &story,
        &LineSelection {
            removed: vec![2],
            added: vec![2],
        },
    )
    .unwrap();
    assert_eq!(
        fixture.git(&["diff", "--cached", "--numstat"]).trim(),
        "2\t1\tstory.txt"
    );
    repo.unstage_lines(
        &story,
        &LineSelection {
            removed: vec![],
            added: vec![16],
        },
    )
    .unwrap();
    assert_eq!(
        fixture.git(&["diff", "--cached", "--numstat"]).trim(),
        "1\t1\tstory.txt"
    );

    // Discard the deletion of line 29 from the working tree; the other edits stay.
    let unstaged = repo.unstaged_diff(&story).unwrap();
    let branchosaurus_core::DiffContent::Text { hunks, .. } = unstaged.content else {
        panic!("text diff expected")
    };
    let removed_29 = hunks
        .iter()
        .flat_map(|hunk| &hunk.lines)
        .find(|line| line.text == "line 29")
        .and_then(|line| line.old_line)
        .unwrap();
    repo.discard_lines(
        &story,
        &LineSelection {
            removed: vec![removed_29],
            added: vec![],
        },
    )
    .unwrap();
    let worktree = fixture.read("story.txt");
    assert!(worktree.contains("line 28\nline 29\nline 30\n"));
    assert!(worktree.contains("line two\n") && worktree.contains("line 15½\n"));
}

#[test]
fn partial_staging_of_a_new_file() {
    let fixture = Fixture::new();
    fixture.commit("initial");
    fixture.write("notes.txt", "keep\nskip\nkeep too\n");
    let repo = fixture.repo();
    let notes = entry("notes.txt", ChangeKind::Untracked);
    repo.stage_lines(
        &notes,
        &LineSelection {
            removed: vec![],
            added: vec![1, 3],
        },
    )
    .unwrap();
    assert_eq!(fixture.git(&["show", ":notes.txt"]), "keep\nkeep too\n");
    assert_eq!(porcelain(&fixture), ["AM notes.txt"]);

    // Unstaging every line of a newly added file removes it from the index again.
    let staged = entry("notes.txt", ChangeKind::Added);
    repo.unstage_lines(
        &staged,
        &LineSelection {
            removed: vec![],
            added: vec![1, 2],
        },
    )
    .unwrap();
    assert_eq!(porcelain(&fixture), ["?? notes.txt"]);
}

#[test]
fn partial_staging_keeps_crlf_line_endings_intact() {
    let fixture = Fixture::new();
    fixture.git(&["config", "core.autocrlf", "true"]);
    fixture.write("windows.txt", "one\r\ntwo\r\nthree\r\n");
    fixture.commit_all("initial");
    fixture.write("windows.txt", "one\r\nTWO\r\ntwo\r\nthree\r\nfour\r\n");
    let repo = fixture.repo();
    let file = entry("windows.txt", ChangeKind::Modified);
    // Stage only the added last line; the index stores LF line endings.
    repo.stage_lines(
        &file,
        &LineSelection {
            removed: vec![],
            added: vec![5],
        },
    )
    .unwrap();
    assert_eq!(
        fixture.git(&["show", ":windows.txt"]),
        "one\ntwo\nthree\nfour\n"
    );
    // Discarding the other addition writes CRLF back to the working tree.
    repo.discard_lines(
        &file,
        &LineSelection {
            removed: vec![],
            added: vec![2],
        },
    )
    .unwrap();
    assert_eq!(
        fixture.read("windows.txt"),
        "one\r\ntwo\r\nthree\r\nfour\r\n"
    );
}

#[test]
fn refuses_stale_selections_and_binary_files() {
    let fixture = Fixture::new();
    fixture.write("a.txt", "a\n");
    fixture.write("blob.bin", [0u8, 1, 2]);
    fixture.commit_all("initial");
    fixture.write("a.txt", "a\nb\n");
    fixture.write("blob.bin", [0u8, 1, 2, 3]);
    let repo = fixture.repo();
    let stale = repo.stage_lines(
        &entry("a.txt", ChangeKind::Modified),
        &LineSelection {
            removed: vec![],
            added: vec![7],
        },
    );
    assert!(matches!(stale, Err(Error::OutOfDate(_))));
    let binary = repo.stage_lines(
        &entry("blob.bin", ChangeKind::Modified),
        &LineSelection {
            removed: vec![],
            added: vec![1],
        },
    );
    assert!(matches!(binary, Err(Error::Unsupported(_))));
}
