// SPDX-License-Identifier: GPL-3.0-or-later

//! Line diffs of single files, between two blobs or between a blob and the working tree.

use std::path::Path;

use gix::ObjectId;
use gix::bstr::{BStr, ByteSlice};
use gix::diff::blob::pipeline::{Mode, WorktreeRoots};
use gix::diff::blob::platform::prepare_diff::Operation;
use gix::diff::blob::platform::resource::Data;
use gix::diff::blob::{Diff, InternedInput, ResourceKind};
use gix::object::tree::EntryKind;

use crate::error::Context;
use crate::highlight::{self, Highlight, Language, LineHighlighter};
use crate::{RepoPath, Result};

/// Lines of unchanged context around each change, as `git diff` shows by default.
const CONTEXT_LINES: u32 = 3;
/// Larger files are not diffed line by line; the UI offers to open them externally instead.
const MAX_DIFF_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ChangeKind {
    Added,
    Deleted,
    Modified,
    Renamed,
    Copied,
    TypeChanged,
    Untracked,
}

/// A blob (or submodule commit) as recorded in a tree or the index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlobRef {
    pub id: ObjectId,
    /// Git file mode, e.g. `0o100644`.
    pub mode: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileDiff {
    pub path: RepoPath,
    pub old_path: Option<RepoPath>,
    pub old_mode: Option<u32>,
    pub new_mode: Option<u32>,
    /// The language the lines are highlighted as, e.g. `Rust`. `None` when the language is
    /// unknown or the diff is too big to highlight quickly.
    pub language: Option<String>,
    pub content: DiffContent,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiffContent {
    /// A line diff. `hunks` is empty when only the mode changed.
    Text {
        hunks: Vec<Hunk>,
        additions: u32,
        deletions: u32,
    },
    Binary {
        old_size: Option<u64>,
        new_size: Option<u64>,
    },
    TooLarge {
        old_size: u64,
        new_size: u64,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hunk {
    /// Line numbers as printed in the `@@ -old_start,old_lines +new_start,new_lines @@` header.
    pub old_start: u32,
    pub old_lines: u32,
    pub new_start: u32,
    pub new_lines: u32,
    pub lines: Vec<DiffLine>,
}

impl Hunk {
    pub fn header(&self) -> String {
        format!(
            "@@ -{},{} +{},{} @@",
            self.old_start, self.old_lines, self.new_start, self.new_lines
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffLine {
    pub kind: LineKind,
    /// 1-based line number in the old version, for context and removed lines.
    pub old_line: Option<u32>,
    /// 1-based line number in the new version, for context and added lines.
    pub new_line: Option<u32>,
    /// The line without its line terminator. Invalid UTF-8 is replaced with U+FFFD.
    pub text: String,
    /// Syntax highlighting, in byte ranges of `text`. Empty when the language is unknown or the
    /// file is too big to highlight.
    pub highlights: Vec<Highlight>,
    /// The line is the last one in its file and has no trailing newline.
    pub missing_newline: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineKind {
    Context,
    Added,
    Removed,
}

/// One side of a file diff.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Side<'a> {
    Missing,
    Object {
        id: ObjectId,
        kind: EntryKind,
        path: &'a BStr,
    },
    /// The file in the working tree; `index_id` is the staged version, used for line-ending
    /// conversion.
    Worktree {
        path: &'a BStr,
        kind: EntryKind,
        index_id: Option<ObjectId>,
    },
}

impl Side<'_> {
    fn mode(&self) -> Option<u32> {
        match self {
            Side::Missing => None,
            Side::Object { kind, .. } | Side::Worktree { kind, .. } => {
                Some(entry_kind_to_mode(*kind))
            }
        }
    }
}

pub(crate) fn diff_sides(
    repo: &gix::Repository,
    workdir: Option<&Path>,
    path: RepoPath,
    old_path: Option<RepoPath>,
    old: Side<'_>,
    new: Side<'_>,
) -> Result<FileDiff> {
    let old_mode = old.mode();
    let new_mode = new.mode();
    let (content, language) = if is_submodule(&old) || is_submodule(&new) {
        (submodule_diff(&old, &new), None)
    } else {
        blob_diff(repo, workdir, &path, old, new)?
    };
    Ok(FileDiff {
        path,
        old_path,
        old_mode,
        new_mode,
        language: language.map(|language| language.name().to_owned()),
        content,
    })
}

fn is_submodule(side: &Side<'_>) -> bool {
    matches!(
        side,
        Side::Object {
            kind: EntryKind::Commit,
            ..
        } | Side::Worktree {
            kind: EntryKind::Commit,
            ..
        }
    )
}

/// Submodules are shown the way `git diff` shows them: as a one-line file with the commit id.
fn submodule_diff(old: &Side<'_>, new: &Side<'_>) -> DiffContent {
    let line = |side: &Side<'_>| match side {
        Side::Object { id, .. } => Some(format!("Subproject commit {id}")),
        Side::Worktree { index_id, .. } => index_id.map(|id| format!("Subproject commit {id}")),
        Side::Missing => None,
    };
    let (old, new) = (line(old), line(new));
    let mut lines = Vec::new();
    if let Some(text) = &old {
        lines.push(DiffLine {
            kind: LineKind::Removed,
            old_line: Some(1),
            new_line: None,
            text: text.clone(),
            highlights: Vec::new(),
            missing_newline: false,
        });
    }
    if let Some(text) = &new {
        lines.push(DiffLine {
            kind: LineKind::Added,
            old_line: None,
            new_line: Some(1),
            text: text.clone(),
            highlights: Vec::new(),
            missing_newline: false,
        });
    }
    let (deletions, additions) = (u32::from(old.is_some()), u32::from(new.is_some()));
    DiffContent::Text {
        hunks: vec![Hunk {
            old_start: deletions,
            old_lines: deletions,
            new_start: additions,
            new_lines: additions,
            lines,
        }],
        additions,
        deletions,
    }
}

/// Both sides of a file diff, read and converted the way git compares them.
pub(crate) enum Texts {
    Text {
        old: Vec<u8>,
        new: Vec<u8>,
        algorithm: gix::diff::blob::Algorithm,
    },
    Binary {
        old_size: Option<u64>,
        new_size: Option<u64>,
    },
    TooLarge {
        old_size: u64,
        new_size: u64,
    },
}

fn blob_diff(
    repo: &gix::Repository,
    workdir: Option<&Path>,
    path: &RepoPath,
    old: Side<'_>,
    new: Side<'_>,
) -> Result<(DiffContent, Option<Language>)> {
    Ok(match load_texts(repo, workdir, old, new)? {
        Texts::Text {
            old,
            new,
            algorithm,
        } => {
            let text = if new.is_empty() { &old } else { &new };
            let first_line = text.split(|byte| *byte == b'\n').next().unwrap_or_default();
            let language = highlight::detect(path, first_line);
            text_diff(algorithm, &old, &new, language)
        }
        Texts::Binary { old_size, new_size } => (DiffContent::Binary { old_size, new_size }, None),
        Texts::TooLarge { old_size, new_size } => {
            (DiffContent::TooLarge { old_size, new_size }, None)
        }
    })
}

/// Reads both sides, worktree files through git's clean filters (`autocrlf`, `ident`, LFS…),
/// and decides whether they can be diffed as text.
pub(crate) fn load_texts(
    repo: &gix::Repository,
    workdir: Option<&Path>,
    old: Side<'_>,
    new: Side<'_>,
) -> Result<Texts> {
    let roots = WorktreeRoots {
        old_root: None,
        new_root: match new {
            Side::Worktree { .. } => workdir.map(Path::to_owned),
            _ => None,
        },
    };
    let mut cache = repo
        .diff_resource_cache(Mode::ToGit, roots)
        .context("preparing to diff")?;
    // A missing side still needs a path and mode for attribute lookup: borrow the other side's.
    let fallback = match (old, new) {
        (Side::Missing, other) | (other, Side::Missing) => other,
        _ => old,
    };
    for (side, kind) in [
        (old, ResourceKind::OldOrSource),
        (new, ResourceKind::NewOrDestination),
    ] {
        let (id, entry_kind, path) = match side {
            Side::Object { id, kind, path } => (id, kind, path),
            Side::Worktree {
                path,
                kind,
                index_id,
            } => (
                index_id.unwrap_or_else(|| repo.object_hash().null()),
                kind,
                path,
            ),
            Side::Missing => match fallback {
                Side::Object { kind, path, .. } | Side::Worktree { kind, path, .. } => {
                    (repo.object_hash().null(), kind, path)
                }
                Side::Missing => {
                    return Ok(Texts::Text {
                        old: Vec::new(),
                        new: Vec::new(),
                        algorithm: gix::diff::blob::Algorithm::Myers,
                    });
                }
            },
        };
        cache
            .set_resource(id, entry_kind, path, kind, &repo.objects)
            .context("reading a file to diff")?;
    }
    let outcome = cache.prepare_diff().context("preparing to diff")?;
    let size = |data: &Data<'_>| match data {
        Data::Missing => None,
        Data::Buffer { buf, .. } => Some(buf.len() as u64),
        Data::Binary { size } => Some(*size),
    };
    let (old_data, new_data) = (outcome.old.data, outcome.new.data);
    let algorithm = match outcome.operation {
        Operation::InternalDiff { algorithm } => algorithm,
        Operation::SourceOrDestinationIsBinary | Operation::ExternalCommand { .. } => {
            return Ok(Texts::Binary {
                old_size: size(&old_data),
                new_size: size(&new_data),
            });
        }
    };
    let old_bytes = old_data.as_slice().unwrap_or_default();
    let new_bytes = new_data.as_slice().unwrap_or_default();
    if old_bytes.len() > MAX_DIFF_BYTES || new_bytes.len() > MAX_DIFF_BYTES {
        return Ok(Texts::TooLarge {
            old_size: old_bytes.len() as u64,
            new_size: new_bytes.len() as u64,
        });
    }
    Ok(Texts::Text {
        old: old_bytes.to_vec(),
        new: new_bytes.to_vec(),
        algorithm,
    })
}

/// The line diff shown to people. Partial staging must use exactly this one, so the lines it is
/// asked to stage are the lines that were shown.
pub(crate) fn line_diff(
    algorithm: gix::diff::blob::Algorithm,
    input: &InternedInput<&[u8]>,
) -> Diff {
    let mut diff = Diff::compute(algorithm, input);
    diff.postprocess_lines(input);
    diff
}

/// Diffs two texts line by line. Lines keep their terminators while diffing, so a change to the
/// final newline counts as a change, as in git. Also returns the language the lines were
/// highlighted as, if any.
pub(crate) fn text_diff(
    algorithm: gix::diff::blob::Algorithm,
    old: &[u8],
    new: &[u8],
    language: Option<Language>,
) -> (DiffContent, Option<Language>) {
    let input = InternedInput::new(old, new);
    let diff = line_diff(algorithm, &input);
    let mut hunks = build_hunks(&input, &diff, CONTEXT_LINES);
    let language =
        language.filter(|language| highlight_hunks(*language, &input, &diff, &mut hunks));
    let content = DiffContent::Text {
        hunks,
        additions: diff.count_additions(),
        deletions: diff.count_removals(),
    };
    (content, language)
}

/// Counts changed lines without building hunks, for the per-file statistics of a commit.
pub(crate) fn line_stats(
    algorithm: gix::diff::blob::Algorithm,
    old: &[u8],
    new: &[u8],
) -> (u32, u32) {
    let input = InternedInput::new(old, new);
    let diff = Diff::compute(algorithm, &input);
    (diff.count_additions(), diff.count_removals())
}

/// A line's text without its terminator, and whether it had none.
fn split_line(bytes: &[u8]) -> (&[u8], bool) {
    match bytes.strip_suffix(b"\n") {
        Some(text) => (text.strip_suffix(b"\r").unwrap_or(text), false),
        None => (bytes, true),
    }
}

/// Fills in the highlights of the hunks' lines.
///
/// Both versions are parsed from their first line, so constructs that start above a hunk (a
/// comment, a string) are recognised inside it. Lines the versions share are parsed once while the
/// two parsers agree, and parsing stops after the last line shown, so the cost is about one pass
/// over the new version down to its last hunk. Diffs that need more than
/// [`highlight::MAX_HIGHLIGHT_BYTES`] of parsing (counting lines the parsers read while they
/// disagree) are left plain. Returns whether the lines were highlighted.
fn highlight_hunks(
    language: Language,
    input: &InternedInput<&[u8]>,
    diff: &Diff,
    hunks: &mut [Hunk],
) -> bool {
    // Which lines of each version are shown: removed lines from the old one, the rest from the
    // new one.
    let mut shown_old = vec![false; input.before.len()];
    let mut shown_new = vec![false; input.after.len()];
    for line in hunks.iter().flat_map(|hunk| &hunk.lines) {
        match (line.kind, line.old_line, line.new_line) {
            (LineKind::Removed, Some(old), _) => shown_old[old as usize - 1] = true,
            (_, _, Some(new)) => shown_new[new as usize - 1] = true,
            _ => {}
        }
    }
    let old_end = shown_old
        .iter()
        .rposition(|shown| *shown)
        .map_or(0, |last| last + 1) as u32;
    let new_end = shown_new
        .iter()
        .rposition(|shown| *shown)
        .map_or(0, |last| last + 1) as u32;
    // The new version down to its last shown line, and the removed lines, must be parsed; give up
    // before starting when that's already too much.
    let line_len = |token: &gix::diff::blob::Token| input.interner[*token].len();
    let needed = input.after[..new_end as usize]
        .iter()
        .map(line_len)
        .sum::<usize>()
        + (input.before.iter().zip(&shown_old))
            .filter(|(_, shown)| **shown)
            .map(|(token, _)| line_len(token))
            .sum::<usize>();
    if needed > highlight::MAX_HIGHLIGHT_BYTES {
        return false;
    }
    let mut old_highlights = vec![Vec::new(); input.before.len()];
    let mut new_highlights = vec![Vec::new(); input.after.len()];

    let walked = (|| {
        let mut budget = highlight::MAX_HIGHLIGHT_BYTES;
        let mut read = |highlighter: &mut LineHighlighter, token, emit: bool| {
            let text = split_line(input.interner[token]).0;
            budget = budget.checked_sub(text.len() + 1)?;
            highlighter.line(&text.to_str_lossy(), emit)
        };
        let mut new = LineHighlighter::new(language);
        // The old version's parser, while it differs from the new one's.
        let mut old: Option<LineHighlighter> = None;
        let (mut o, mut n) = (0u32, 0u32);
        // A final empty change covers the unchanged lines after the last real one.
        let end = gix::diff::blob::Hunk {
            before: input.before.len() as u32..input.before.len() as u32,
            after: input.after.len() as u32..input.after.len() as u32,
        };
        for change in diff.hunks().chain(std::iter::once(end)) {
            while o < change.before.start {
                if o >= old_end && n >= new_end {
                    return Some(());
                }
                new_highlights[n as usize] =
                    read(&mut new, input.after[n as usize], shown_new[n as usize])?;
                if let Some(diverged) = &mut old {
                    read(diverged, input.before[o as usize], false)?;
                    if *diverged == new {
                        old = None;
                    }
                }
                o += 1;
                n += 1;
            }
            if o >= old_end && n >= new_end {
                return Some(());
            }
            let mut diverged = old.take().unwrap_or_else(|| new.clone());
            for pos in change.before.clone() {
                old_highlights[pos as usize] = read(
                    &mut diverged,
                    input.before[pos as usize],
                    shown_old[pos as usize],
                )?;
            }
            for pos in change.after.clone() {
                new_highlights[pos as usize] =
                    read(&mut new, input.after[pos as usize], shown_new[pos as usize])?;
            }
            old = (diverged != new).then_some(diverged);
            (o, n) = (change.before.end, change.after.end);
        }
        Some(())
    })();
    if walked.is_none() {
        return false;
    }
    for line in hunks.iter_mut().flat_map(|hunk| &mut hunk.lines) {
        line.highlights = match (line.kind, line.old_line, line.new_line) {
            (LineKind::Removed, Some(old), _) => {
                std::mem::take(&mut old_highlights[old as usize - 1])
            }
            (_, _, Some(new)) => std::mem::take(&mut new_highlights[new as usize - 1]),
            _ => Vec::new(),
        };
    }
    true
}

fn build_hunks(input: &InternedInput<&[u8]>, diff: &Diff, context: u32) -> Vec<Hunk> {
    let changes: Vec<_> = diff.hunks().collect();
    let old_len = input.before.len() as u32;
    // Removed lines come from the old version; added and context lines from the new one.
    let line = |kind: LineKind, old_line: Option<u32>, new_line: Option<u32>| {
        let token = match (kind, old_line, new_line) {
            (LineKind::Removed, Some(old), _) => input.before[old as usize - 1],
            (_, _, Some(new)) => input.after[new as usize - 1],
            _ => unreachable!("every diff line has the line number of the side it shows"),
        };
        let (text, missing_newline) = split_line(input.interner[token]);
        DiffLine {
            kind,
            old_line,
            new_line,
            text: text.to_str_lossy().into_owned(),
            highlights: Vec::new(),
            missing_newline,
        }
    };

    let mut hunks = Vec::new();
    let mut first = 0;
    while first < changes.len() {
        // Changes closer than twice the context share one hunk.
        let mut last = first;
        while last + 1 < changes.len()
            && changes[last + 1].before.start - changes[last].before.end <= 2 * context
        {
            last += 1;
        }
        let old_start = changes[first].before.start.saturating_sub(context);
        let old_end = (changes[last].before.end + context).min(old_len);
        let new_start = changes[first].after.start - (changes[first].before.start - old_start);

        let mut lines = Vec::new();
        let (mut old_pos, mut new_pos) = (old_start, new_start);
        for change in &changes[first..=last] {
            while old_pos < change.before.start {
                lines.push(line(
                    LineKind::Context,
                    Some(old_pos + 1),
                    Some(new_pos + 1),
                ));
                old_pos += 1;
                new_pos += 1;
            }
            for pos in change.before.clone() {
                lines.push(line(LineKind::Removed, Some(pos + 1), None));
            }
            for pos in change.after.clone() {
                lines.push(line(LineKind::Added, None, Some(pos + 1)));
            }
            old_pos = change.before.end;
            new_pos = change.after.end;
        }
        while old_pos < old_end {
            lines.push(line(
                LineKind::Context,
                Some(old_pos + 1),
                Some(new_pos + 1),
            ));
            old_pos += 1;
            new_pos += 1;
        }

        let old_lines = old_end - old_start;
        let new_lines = new_pos - new_start;
        hunks.push(Hunk {
            // Git prints the line before an empty range, and 1-based starts otherwise.
            old_start: if old_lines == 0 {
                old_start
            } else {
                old_start + 1
            },
            old_lines,
            new_start: if new_lines == 0 {
                new_start
            } else {
                new_start + 1
            },
            new_lines,
            lines,
        });
        first = last + 1;
    }
    hunks
}

pub(crate) fn entry_kind_to_mode(kind: EntryKind) -> u32 {
    match kind {
        EntryKind::Tree => 0o040000,
        EntryKind::Blob => 0o100644,
        EntryKind::BlobExecutable => 0o100755,
        EntryKind::Link => 0o120000,
        EntryKind::Commit => 0o160000,
    }
}

pub(crate) fn mode_to_entry_kind(mode: u32) -> EntryKind {
    match mode {
        0o040000 => EntryKind::Tree,
        0o100755 => EntryKind::BlobExecutable,
        0o120000 => EntryKind::Link,
        0o160000 => EntryKind::Commit,
        _ => EntryKind::Blob,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gix::diff::blob::Algorithm;

    fn hunks(old: &str, new: &str) -> Vec<Hunk> {
        match text_diff(Algorithm::Myers, old.as_bytes(), new.as_bytes(), None).0 {
            DiffContent::Text { hunks, .. } => hunks,
            other => panic!("expected text, got {other:?}"),
        }
    }

    #[test]
    fn single_change_has_context_on_both_sides() {
        let old = "1\n2\n3\n4\n5\n6\n7\n8\n9\n";
        let new = "1\n2\n3\n4\nfive\n6\n7\n8\n9\n";
        let hunks = hunks(old, new);
        assert_eq!(hunks.len(), 1);
        assert_eq!(hunks[0].header(), "@@ -2,7 +2,7 @@");
        let kinds: Vec<_> = hunks[0].lines.iter().map(|line| line.kind).collect();
        use LineKind::*;
        assert_eq!(
            kinds,
            [
                Context, Context, Context, Removed, Added, Context, Context, Context
            ]
        );
    }

    #[test]
    fn distant_changes_get_separate_hunks() {
        let old: String = (1..=20).map(|n| format!("{n}\n")).collect();
        let new: String = (1..=20)
            .map(|n| match n {
                2 => "two\n".to_owned(),
                19 => "nineteen\n".to_owned(),
                n => format!("{n}\n"),
            })
            .collect();
        let hunks = hunks(&old, &new);
        assert_eq!(hunks.len(), 2);
        assert_eq!(hunks[0].header(), "@@ -1,5 +1,5 @@");
        assert_eq!(hunks[1].header(), "@@ -16,5 +16,5 @@");
    }

    #[test]
    fn new_file_and_missing_newline() {
        let hunks = hunks("", "a\nb");
        assert_eq!(hunks[0].header(), "@@ -0,0 +1,2 @@");
        assert!(hunks[0].lines[1].missing_newline);
        assert_eq!(hunks[0].lines[1].text, "b");
    }
}
