// SPDX-License-Identifier: GPL-3.0-or-later

//! Commit details and file diffs, checked against `git show --numstat` and `git diff`, and by
//! rebuilding the new file from the old one plus the hunks.

mod support;

use branchosaurus_core::{
    ChangeKind, DiffContent, DiffLine, FileDiff, HighlightKind, LineKind, StatusEntry,
    parse_object_id,
};
use support::Fixture;

/// Applies the diff's hunks to `old` and returns the resulting lines.
fn apply(old: &str, diff: &FileDiff) -> Vec<String> {
    let DiffContent::Text { hunks, .. } = &diff.content else {
        panic!("expected a text diff, got {:?}", diff.content);
    };
    let old_lines: Vec<&str> = old.lines().collect();
    let mut out = Vec::new();
    let mut next_old = 0usize;
    for hunk in hunks {
        let hunk_start = if hunk.old_lines == 0 {
            hunk.old_start as usize
        } else {
            hunk.old_start as usize - 1
        };
        out.extend(
            old_lines[next_old..hunk_start]
                .iter()
                .map(|line| line.to_string()),
        );
        next_old = hunk_start;
        for line in &hunk.lines {
            match line.kind {
                LineKind::Context => {
                    assert_eq!(
                        old_lines[next_old], line.text,
                        "context must match the old file"
                    );
                    out.push(line.text.clone());
                    next_old += 1;
                }
                LineKind::Removed => {
                    assert_eq!(old_lines[next_old], line.text, "removed lines must exist");
                    next_old += 1;
                }
                LineKind::Added => out.push(line.text.clone()),
            }
        }
    }
    out.extend(old_lines[next_old..].iter().map(|line| line.to_string()));
    out
}

fn numstat(fixture: &Fixture, args: &[&str]) -> Vec<(u32, u32, String)> {
    fixture
        .git(args)
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| {
            let mut parts = line.splitn(3, '\t');
            let added = parts.next().unwrap().parse().unwrap();
            let removed = parts.next().unwrap().parse().unwrap();
            (added, removed, parts.next().unwrap().to_owned())
        })
        .collect()
}

#[test]
fn commit_details_match_git_show() {
    let fixture = Fixture::new();
    let old_text: String = (1..=30).map(|n| format!("line {n}\n")).collect();
    fixture.write("src/main.rs", &old_text);
    fixture.write("README.md", "# Title\n");
    fixture.write("old.txt", "will be deleted\n");
    let parent = fixture.commit_all("initial");

    let new_text = old_text
        .replace("line 3\n", "line three\n")
        .replace("line 20\n", "")
        .replace("line 30\n", "line 30\nline 31\nline 32\n");
    fixture.write("src/main.rs", &new_text);
    fixture.write("docs/new.md", "new file\n");
    fixture.remove("old.txt");
    let id = fixture.commit_all("Change things\n\nWith a body.");

    let repo = fixture.repo();
    let details = repo.commit_details(parse_object_id(&id).unwrap()).unwrap();
    assert_eq!(details.summary, "Change things");
    assert_eq!(details.message, "Change things\n\nWith a body.\n");
    assert_eq!(details.parents, [parse_object_id(&parent).unwrap()]);
    assert_eq!(details.author.name, "Ada Lovelace");

    let ours: Vec<(u32, u32, String)> = details
        .changes
        .iter()
        .map(|change| {
            let stats = change.stats.expect("text files have stats");
            (stats.additions, stats.deletions, change.path.to_string())
        })
        .collect();
    let mut git = numstat(&fixture, &["show", "--numstat", "--format=", &id]);
    git.sort_by(|a, b| a.2.cmp(&b.2));
    assert_eq!(ours, git);

    let kinds: Vec<_> = details.changes.iter().map(|change| change.kind).collect();
    assert_eq!(
        kinds,
        [ChangeKind::Added, ChangeKind::Deleted, ChangeKind::Modified]
    );

    let main = details
        .changes
        .iter()
        .find(|c| c.path.to_string() == "src/main.rs")
        .unwrap();
    let diff = repo.commit_file_diff(main).unwrap();
    assert_eq!(
        apply(&old_text, &diff),
        new_text.lines().collect::<Vec<_>>()
    );
    let DiffContent::Text { hunks, .. } = &diff.content else {
        unreachable!()
    };
    let headers: Vec<String> = hunks.iter().map(|hunk| hunk.header()).collect();
    let git_headers: Vec<String> = fixture
        .git(&["show", "--format=", &id, "--", "src/main.rs"])
        .lines()
        .filter(|line| line.starts_with("@@"))
        .map(|line| line.split(" @@").next().unwrap().to_owned() + " @@")
        .collect();
    assert_eq!(headers, git_headers);
}

#[test]
fn root_commit_and_renames() {
    let fixture = Fixture::new();
    let body = "a line that makes the file recognisable\n".repeat(20);
    fixture.write("before.txt", &body);
    let root = fixture.commit_all("root");
    let repo = fixture.repo();
    let details = repo
        .commit_details(parse_object_id(&root).unwrap())
        .unwrap();
    assert!(details.parents.is_empty());
    assert_eq!(details.changes.len(), 1);
    assert_eq!(details.changes[0].kind, ChangeKind::Added);

    fixture.git(&["mv", "before.txt", "after.txt"]);
    let renamed = fixture.commit("rename");
    let details = repo
        .commit_details(parse_object_id(&renamed).unwrap())
        .unwrap();
    assert_eq!(details.changes.len(), 1);
    let change = &details.changes[0];
    assert_eq!(change.kind, ChangeKind::Renamed);
    assert_eq!(change.path.to_string(), "after.txt");
    assert_eq!(change.old_path.as_ref().unwrap().to_string(), "before.txt");
    let diff = repo.commit_file_diff(change).unwrap();
    assert!(matches!(&diff.content, DiffContent::Text { hunks, .. } if hunks.is_empty()));
}

#[test]
fn binary_files_are_not_diffed_as_text() {
    let fixture = Fixture::new();
    fixture.write("image.bin", [0u8, 1, 2, 3, 0, 255]);
    let id = fixture.commit_all("binary");
    let repo = fixture.repo();
    let details = repo.commit_details(parse_object_id(&id).unwrap()).unwrap();
    assert_eq!(details.changes[0].stats, None);
    let diff = repo.commit_file_diff(&details.changes[0]).unwrap();
    assert_eq!(
        diff.content,
        DiffContent::Binary {
            old_size: None,
            new_size: Some(6)
        }
    );
}

#[test]
fn staged_and_unstaged_diffs_of_the_same_file() {
    let fixture = Fixture::new();
    fixture.write("notes.txt", "one\ntwo\nthree\n");
    fixture.commit_all("initial");
    fixture.write("notes.txt", "one\n2\nthree\n");
    fixture.git(&["add", "notes.txt"]);
    fixture.write("notes.txt", "one\n2\nthree\nfour");

    let repo = fixture.repo();
    let entry = StatusEntry {
        path: "notes.txt".into(),
        old_path: None,
        kind: ChangeKind::Modified,
    };
    let staged = repo.staged_diff(&entry).unwrap();
    assert_eq!(apply("one\ntwo\nthree\n", &staged), ["one", "2", "three"]);
    let unstaged = repo.unstaged_diff(&entry).unwrap();
    assert_eq!(
        apply("one\n2\nthree\n", &unstaged),
        ["one", "2", "three", "four"]
    );
    let DiffContent::Text {
        hunks,
        additions,
        deletions,
    } = &unstaged.content
    else {
        unreachable!()
    };
    assert_eq!((*additions, *deletions), (1, 0));
    assert!(hunks[0].lines.last().unwrap().missing_newline);

    let git = numstat(&fixture, &["diff", "--numstat"]);
    assert_eq!(git, [(1, 0, "notes.txt".to_owned())]);
}

#[test]
fn untracked_and_deleted_files() {
    let fixture = Fixture::new();
    fixture.write("gone.txt", "bye\n");
    fixture.commit_all("initial");
    fixture.remove("gone.txt");
    fixture.write("fresh.txt", "hello\nworld\n");
    let repo = fixture.repo();

    let untracked = StatusEntry {
        path: "fresh.txt".into(),
        old_path: None,
        kind: ChangeKind::Untracked,
    };
    let diff = repo.unstaged_diff(&untracked).unwrap();
    assert_eq!(apply("", &diff), ["hello", "world"]);

    let deleted = StatusEntry {
        path: "gone.txt".into(),
        old_path: None,
        kind: ChangeKind::Deleted,
    };
    let diff = repo.unstaged_diff(&deleted).unwrap();
    assert_eq!(apply("bye\n", &diff), Vec::<String>::new());
}

/// With `core.autocrlf=true` a CRLF working tree file whose content is unchanged has no diff,
/// because the worktree side goes through the same clean filter as `git add`.
#[test]
fn line_endings_are_normalised_like_git() {
    let fixture = Fixture::new();
    fixture.git(&["config", "core.autocrlf", "true"]);
    fixture.write("windows.txt", "first\r\nsecond\r\n");
    fixture.commit_all("crlf");
    fixture.write("windows.txt", "first\r\nsecond\r\nthird\r\n");
    let repo = fixture.repo();
    let entry = StatusEntry {
        path: "windows.txt".into(),
        old_path: None,
        kind: ChangeKind::Modified,
    };
    let diff = repo.unstaged_diff(&entry).unwrap();
    let DiffContent::Text {
        additions,
        deletions,
        ..
    } = diff.content
    else {
        panic!("expected text")
    };
    assert_eq!((additions, deletions), (1, 0));
    assert_eq!(
        numstat(&fixture, &["diff", "--numstat"]),
        [(1, 0, "windows.txt".to_owned())]
    );
}

/// The highlighted pieces of a line, as text and kind.
fn pieces(line: &DiffLine) -> Vec<(&str, HighlightKind)> {
    line.highlights
        .iter()
        .map(|h| (&line.text[h.start as usize..h.end as usize], h.kind))
        .collect()
}

#[test]
fn lines_are_syntax_highlighted() {
    let fixture = Fixture::new();
    let old = "/*\n * A long comment\n * that starts\n * well above\n * the change.\n */\nfn old_name() {}\nlet x = 1;\n";
    let new = "/*\n * A long comment\n * that starts\n * well above\n * the changed line.\n */\nfn new_name() {}\nlet x = \"naïve ☕\";\n";
    fixture.write("src/lib.rs", old);
    fixture.commit_all("initial");
    fixture.write("src/lib.rs", new);
    let entry = StatusEntry {
        path: "src/lib.rs".into(),
        old_path: None,
        kind: ChangeKind::Modified,
    };
    let diff = fixture.repo().unstaged_diff(&entry).unwrap();
    assert_eq!(diff.language.as_deref(), Some("Rust"));
    let DiffContent::Text { hunks, .. } = &diff.content else {
        panic!("expected text")
    };
    let lines: Vec<&DiffLine> = hunks.iter().flat_map(|hunk| &hunk.lines).collect();
    let find = |kind: LineKind, text: &str| {
        *lines
            .iter()
            .find(|line| line.kind == kind && line.text.contains(text))
            .unwrap_or_else(|| panic!("no {kind:?} line containing {text:?}"))
    };

    // The comment opens above the hunk's context, and is still a comment inside it.
    let comment = find(LineKind::Removed, "the change.");
    assert_eq!(
        pieces(comment),
        [(" * the change.", HighlightKind::Comment)]
    );
    let comment = find(LineKind::Added, "the changed line.");
    assert_eq!(
        pieces(comment),
        [(" * the changed line.", HighlightKind::Comment)]
    );

    let removed = pieces(find(LineKind::Removed, "old_name"));
    assert!(
        removed.contains(&("fn", HighlightKind::Keyword)),
        "{removed:?}"
    );
    assert!(
        removed.contains(&("old_name", HighlightKind::Function)),
        "{removed:?}"
    );
    let added = pieces(find(LineKind::Added, "new_name"));
    assert!(
        added.contains(&("new_name", HighlightKind::Function)),
        "{added:?}"
    );
    let string = pieces(find(LineKind::Added, "naïve"));
    assert!(
        string.contains(&("\"naïve ☕\"", HighlightKind::String)),
        "{string:?}"
    );
    let number = pieces(find(LineKind::Removed, "let x = 1"));
    assert!(number.contains(&("1", HighlightKind::Number)), "{number:?}");

    for line in &lines {
        for highlight in &line.highlights {
            assert!(highlight.start < highlight.end);
            assert!(line.text.is_char_boundary(highlight.start as usize));
            assert!(line.text.is_char_boundary(highlight.end as usize));
        }
    }
}

#[test]
fn unknown_languages_are_not_highlighted() {
    let fixture = Fixture::new();
    fixture.write("notes.txt", "fn not_code() {}\n");
    fixture.commit_all("initial");
    fixture.write("notes.txt", "fn still_not_code() {}\n");
    let entry = StatusEntry {
        path: "notes.txt".into(),
        old_path: None,
        kind: ChangeKind::Modified,
    };
    let diff = fixture.repo().unstaged_diff(&entry).unwrap();
    assert_eq!(diff.language, None);
    let DiffContent::Text { hunks, .. } = &diff.content else {
        panic!("expected text")
    };
    assert!(
        hunks
            .iter()
            .flat_map(|hunk| &hunk.lines)
            .all(|line| line.highlights.is_empty())
    );
}

/// Parsing stops after the last line shown, so a change at the top of a big file is highlighted;
/// the same file changed at the bottom is too much to parse, and is shown plain.
#[test]
fn big_files_are_highlighted_only_down_to_their_last_hunk() {
    let fixture = Fixture::new();
    let body: String = (0..20_000)
        .map(|i| format!("const C{i}: u32 = {i};\n"))
        .collect();
    assert!(body.len() > 256 * 1024);
    fixture.write("top.rs", &body);
    fixture.write("bottom.rs", &body);
    fixture.commit_all("initial");
    fixture.write("top.rs", format!("// changed\n{body}"));
    fixture.write("bottom.rs", format!("{body}// changed\n"));
    let repo = fixture.repo();
    let diff = |path: &str| {
        let entry = StatusEntry {
            path: path.into(),
            old_path: None,
            kind: ChangeKind::Modified,
        };
        repo.unstaged_diff(&entry).unwrap()
    };
    let highlighted = |diff: &FileDiff| {
        let DiffContent::Text { hunks, .. } = &diff.content else {
            panic!("expected text")
        };
        hunks
            .iter()
            .flat_map(|hunk| &hunk.lines)
            .any(|line| !line.highlights.is_empty())
    };

    let top = diff("top.rs");
    assert_eq!(top.language.as_deref(), Some("Rust"));
    assert!(highlighted(&top));
    let bottom = diff("bottom.rs");
    assert_eq!(bottom.language, None);
    assert!(!highlighted(&bottom));
}

/// Removing a comment's opener far above a change: the old version's lines are still read as a
/// comment, the new version's as code.
#[test]
fn each_version_is_highlighted_with_its_own_state() {
    let fixture = Fixture::new();
    let filler: String = (0..10).map(|i| format!("line {i}\n")).collect();
    fixture.write("x.c", format!("/*\n{filler}int old_value;\n*/\n"));
    fixture.commit_all("initial");
    fixture.write("x.c", format!("{filler}int new_value;\n"));
    let entry = StatusEntry {
        path: "x.c".into(),
        old_path: None,
        kind: ChangeKind::Modified,
    };
    let diff = fixture.repo().unstaged_diff(&entry).unwrap();
    let DiffContent::Text { hunks, .. } = &diff.content else {
        panic!("expected text")
    };
    assert_eq!(hunks.len(), 2, "{hunks:?}");
    let lines: Vec<&DiffLine> = hunks.iter().flat_map(|hunk| &hunk.lines).collect();
    let line = |text: &str| *lines.iter().find(|line| line.text == text).unwrap();
    assert_eq!(
        pieces(line("int old_value;")),
        [("int old_value;", HighlightKind::Comment)]
    );
    let added = pieces(line("int new_value;"));
    assert!(
        added.contains(&("int", HighlightKind::Keyword)),
        "{added:?}"
    );
}
