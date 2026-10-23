// SPDX-License-Identifier: GPL-3.0-or-later

mod support;

use branchosaurus_core::{Head, Repository};
use support::Fixture;

#[test]
fn branches_remotes_tags_and_upstream() {
    let upstream = Fixture::new();
    upstream.commit("upstream work");
    upstream.git(&["branch", "release/v1.10"]);
    upstream.git(&["branch", "release/v1.9"]);

    let fixture = Fixture::new();
    let remote = upstream.path().display().to_string();
    fixture.git(&["remote", "add", "origin", &remote]);
    fixture.git(&["fetch", "--quiet", "origin"]);
    fixture.git(&["remote", "set-head", "origin", "main"]);
    fixture.git(&["switch", "--quiet", "-c", "main", "--track", "origin/main"]);
    fixture.git(&["branch", "local-only"]);
    fixture.git(&["tag", "-a", "-m", "annotated", "v1.0"]);

    let refs = fixture.repo().refs().unwrap();
    let locals: Vec<_> = refs
        .local_branches
        .iter()
        .map(|b| (b.name.as_str(), b.upstream.as_deref(), b.is_head))
        .collect();
    assert_eq!(
        locals,
        [
            ("local-only", None, false),
            ("main", Some("origin/main"), true)
        ]
    );
    let remotes: Vec<_> = refs
        .remote_branches
        .iter()
        .map(|b| b.name.as_str())
        .collect();
    assert_eq!(
        remotes,
        ["origin/main", "origin/release/v1.9", "origin/release/v1.10"]
    );
    assert!(refs.remote_branches.iter().all(|b| b.remote == "origin"));
    assert_eq!(refs.tags.len(), 1);
    // Annotated tags are peeled to the commit they tag.
    assert_eq!(refs.tags[0].target.to_string(), fixture.rev_parse("HEAD"));
    assert!(matches!(refs.head, Some(Head::Branch { ref name, .. }) if name == "main"));
}

#[test]
fn unborn_branch() {
    let fixture = Fixture::new();
    let info = fixture.repo().info().unwrap();
    assert_eq!(
        info.head,
        Head::Branch {
            name: "main".into(),
            target: None
        }
    );
    assert!(fixture.repo().refs().unwrap().local_branches.is_empty());
}

#[test]
fn discover_finds_the_repository_from_a_subdirectory() {
    let fixture = Fixture::new();
    fixture.write("deep/inside/file.txt", "x");
    let repo = Repository::discover(fixture.path().join("deep/inside")).unwrap();
    assert_eq!(
        repo.workdir().unwrap().canonicalize().unwrap(),
        fixture.path().canonicalize().unwrap()
    );
    assert!(Repository::discover(std::env::temp_dir().join("surely/not/a/repo")).is_err());
}
