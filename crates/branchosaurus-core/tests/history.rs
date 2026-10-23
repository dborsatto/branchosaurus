// SPDX-License-Identifier: GPL-3.0-or-later

mod support;

use std::collections::HashMap;

use branchosaurus_core::{
    CancelToken, EdgeKind, Error, History, HistoryRow, RefLabelKind, render_ascii,
};
use support::Fixture;

fn load(fixture: &Fixture) -> History {
    History::load(&fixture.repo(), &CancelToken::new(), &mut |_| {}).expect("load history")
}

fn graph(fixture: &Fixture) -> String {
    let history = load(fixture);
    render_ascii(&history.rows(0, history.len()).unwrap())
}

/// Same commits and the same order as `git rev-list --date-order` over every ref we show.
fn assert_matches_git(fixture: &Fixture) {
    let history = load(fixture);
    let rows = history.rows(0, history.len()).unwrap();
    let ours: Vec<String> = rows.iter().map(|row| row.id.to_string()).collect();
    let git: Vec<String> = fixture
        .git(&[
            "rev-list",
            "--date-order",
            "--branches",
            "--remotes",
            "--tags",
            "HEAD",
        ])
        .lines()
        .map(str::to_owned)
        .collect();
    assert_eq!(ours, git);

    // Every commit is above all of its parents.
    let row_of: HashMap<&str, usize> = ours
        .iter()
        .enumerate()
        .map(|(i, id)| (id.as_str(), i))
        .collect();
    for (row, id) in ours.iter().enumerate() {
        let parents = fixture.git(&["rev-list", "--parents", "-n", "1", id]);
        for parent in parents.split_whitespace().skip(1) {
            assert!(
                row_of[parent] > row,
                "{id} must be above its parent {parent}"
            );
        }
    }
}

/// Every line that leaves a row's bottom enters the next row's top in the same lane.
fn assert_lines_connect(fixture: &Fixture) {
    let history = load(fixture);
    let rows = history.rows(0, history.len()).unwrap();
    for pair in rows.windows(2) {
        let mut leaving: Vec<u16> = pair[0]
            .graph
            .edges
            .iter()
            .filter(|edge| edge.kind != EdgeKind::IntoNode)
            .map(|edge| edge.to_lane)
            .collect();
        let mut entering: Vec<u16> = pair[1]
            .graph
            .edges
            .iter()
            .filter(|edge| edge.kind != EdgeKind::OutOfNode)
            .map(|edge| edge.from_lane)
            .collect();
        leaving.sort_unstable();
        leaving.dedup();
        entering.sort_unstable();
        assert_eq!(
            leaving, entering,
            "between {} and {}",
            pair[0].summary, pair[1].summary
        );
    }
}

#[test]
fn linear() {
    let fixture = Fixture::new();
    for n in 1..=3 {
        fixture.write("file.txt", format!("{n}\n"));
        fixture.commit_all(&format!("commit {n}"));
    }
    assert_matches_git(&fixture);
    assert_lines_connect(&fixture);
    insta::assert_snapshot!(graph(&fixture));
}

#[test]
fn branch_and_merge() {
    let fixture = Fixture::new();
    fixture.commit("base");
    fixture.git(&["switch", "--quiet", "-c", "feature"]);
    fixture.commit("feature 1");
    fixture.commit("feature 2");
    fixture.git(&["switch", "--quiet", "main"]);
    fixture.commit("main 1");
    fixture.git(&[
        "merge",
        "--quiet",
        "--no-ff",
        "-m",
        "merge feature",
        "feature",
    ]);
    fixture.commit("after merge");
    assert_matches_git(&fixture);
    assert_lines_connect(&fixture);
    insta::assert_snapshot!(graph(&fixture));
}

#[test]
fn octopus_merge() {
    let fixture = Fixture::new();
    fixture.commit("base");
    for name in ["a", "b", "c"] {
        fixture.git(&["switch", "--quiet", "-c", name, "main"]);
        fixture.write(&format!("{name}.txt"), name);
        fixture.commit_all(&format!("topic {name}"));
    }
    fixture.git(&["switch", "--quiet", "main"]);
    fixture.git(&["merge", "--quiet", "-m", "octopus", "a", "b", "c"]);
    assert_matches_git(&fixture);
    assert_lines_connect(&fixture);
    insta::assert_snapshot!(graph(&fixture));
}

#[test]
fn criss_cross_merge() {
    let fixture = Fixture::new();
    fixture.commit("base");
    fixture.git(&["branch", "other"]);
    fixture.commit("main 1");
    fixture.git(&["switch", "--quiet", "other"]);
    fixture.commit("other 1");
    fixture.git(&[
        "merge",
        "--quiet",
        "--no-ff",
        "-m",
        "merge main into other",
        "main",
    ]);
    fixture.git(&["switch", "--quiet", "main"]);
    fixture.git(&[
        "merge",
        "--quiet",
        "--no-ff",
        "-m",
        "merge other 1 into main",
        "other~1",
    ]);
    fixture.git(&["switch", "--quiet", "other"]);
    fixture.commit("other 2");
    fixture.git(&["switch", "--quiet", "main"]);
    fixture.commit("main 2");
    assert_matches_git(&fixture);
    assert_lines_connect(&fixture);
    insta::assert_snapshot!(graph(&fixture));
}

#[test]
fn multiple_roots_and_tags() {
    let fixture = Fixture::new();
    fixture.commit("main root");
    fixture.git(&["tag", "-a", "-m", "first release", "v1.0"]);
    fixture.git(&["switch", "--quiet", "--orphan", "pages"]);
    fixture.commit("pages root");
    fixture.git(&["switch", "--quiet", "main"]);
    fixture.commit("main 2");
    fixture.git(&["tag", "light"]);
    assert_matches_git(&fixture);
    assert_lines_connect(&fixture);
    insta::assert_snapshot!(graph(&fixture));

    let history = load(&fixture);
    let first = &history.rows(0, 1).unwrap()[0];
    let labels: Vec<_> = first
        .refs
        .iter()
        .map(|label| (label.kind, label.name.as_str(), label.is_head))
        .collect();
    assert_eq!(
        labels,
        [
            (RefLabelKind::Branch, "main", true),
            (RefLabelKind::Tag, "light", false)
        ]
    );
}

/// `main` and `origin/main` on one commit are one label, and the checked-out branch comes first.
#[test]
fn local_and_remote_branches_share_a_label() {
    let fixture = Fixture::new();
    let base = fixture.commit("base");
    fixture.commit("tip");
    fixture.git(&["update-ref", "refs/remotes/origin/main", "HEAD"]);
    fixture.git(&["update-ref", "refs/remotes/upstream/main", "HEAD"]);
    fixture.git(&["update-ref", "refs/remotes/origin/old", &base]);
    fixture.git(&["branch", "aaa"]);

    let history = load(&fixture);
    let rows = history.rows(0, 2).unwrap();
    let labels = |row: &HistoryRow| -> Vec<(String, bool, Vec<String>, bool)> {
        row.refs
            .iter()
            .map(|label| {
                (
                    label.display_name(),
                    label.local,
                    label.remotes.clone(),
                    label.is_head,
                )
            })
            .collect()
    };
    assert_eq!(
        labels(&rows[0]),
        [
            (
                "main".into(),
                true,
                vec!["origin".into(), "upstream".into()],
                true
            ),
            ("aaa".into(), true, vec![], false),
        ]
    );
    assert_eq!(
        labels(&rows[1]),
        [("origin/old".into(), false, vec!["origin".into()], false)]
    );

    let commit_time: i64 = fixture
        .git(&["log", "-1", "--format=%ct"])
        .trim()
        .parse()
        .unwrap();
    assert_eq!(rows[0].commit_time, commit_time);
}

#[test]
fn detached_head_gets_its_own_label() {
    let fixture = Fixture::new();
    let first = fixture.commit("one");
    fixture.commit("two");
    fixture.git(&["switch", "--quiet", "--detach", &first]);
    let history = load(&fixture);
    let row = history
        .row_of(&branchosaurus_core::parse_object_id(&first).unwrap())
        .unwrap();
    let labels = &history.rows(row, 1).unwrap()[0].refs;
    assert!(
        labels
            .iter()
            .any(|label| label.kind == RefLabelKind::Head && label.is_head)
    );
}

#[test]
fn commit_graph_and_decoding_agree() {
    let fixture = Fixture::new();
    fixture.commit("base");
    fixture.git(&["switch", "--quiet", "-c", "feature"]);
    fixture.commit("feature");
    fixture.git(&["switch", "--quiet", "main"]);
    fixture.git(&["merge", "--quiet", "--no-ff", "-m", "merge", "feature"]);
    let layout = |history: &History, skip: u32| -> Vec<(String, u16)> {
        let rows = history.rows(skip, history.len()).unwrap();
        rows.iter()
            .map(|row| (row.id.to_string(), row.graph.node_lane))
            .collect()
    };
    let decoded = load(&fixture);
    assert_eq!(decoded.stats().index.from_commit_graph, 0);

    fixture.git(&["commit-graph", "write", "--reachable"]);
    // Commits newer than the commit-graph file are decoded.
    fixture.commit("after the graph was written");
    let mixed = load(&fixture);
    assert_eq!(mixed.stats().index.from_commit_graph, 3);
    assert_eq!(mixed.stats().index.decoded, 1);
    assert_eq!(layout(&mixed, 1), layout(&decoded, 0));
    assert_matches_git(&fixture);
}

#[test]
fn paging_matches_a_single_page() {
    let fixture = Fixture::new();
    fixture.commit("base");
    for n in 0..40 {
        let branch = format!("topic-{n}");
        fixture.git(&["switch", "--quiet", "-c", &branch, "main"]);
        fixture.commit(&format!("topic {n}"));
        fixture.git(&["switch", "--quiet", "main"]);
        fixture.git(&[
            "merge",
            "--quiet",
            "--no-ff",
            "-m",
            &format!("merge {n}"),
            &branch,
        ]);
    }
    let history = load(&fixture);
    let all = history.rows(0, history.len()).unwrap();
    let mut paged = Vec::new();
    let mut start = 0;
    while start < history.len() {
        paged.extend(history.rows(start, 7).unwrap());
        start += 7;
    }
    assert_eq!(all, paged);
    assert!(history.rows(history.len() + 10, 5).unwrap().is_empty());
}

#[test]
fn empty_repository_has_no_rows() {
    let fixture = Fixture::new();
    let history = load(&fixture);
    assert!(history.is_empty());
    assert!(history.rows(0, 10).unwrap().is_empty());
}

#[test]
fn cancelled_load_stops() {
    let fixture = Fixture::new();
    fixture.commit("base");
    let cancel = CancelToken::new();
    cancel.cancel();
    let result = History::load(&fixture.repo(), &cancel, &mut |_| {});
    assert!(matches!(result, Err(Error::Cancelled)));
}
