// SPDX-License-Identifier: GPL-3.0-or-later

//! Staging, unstaging and discarding individual lines (a hunk is just all of its lines).
//!
//! Rather than building a patch for `git apply`, which is fragile with line endings, whitespace
//! settings and missing final newlines, the core computes the new file content itself from the
//! same diff that was shown, stores it with `git hash-object`, and points the index at it with
//! `git update-index`. Discarding writes the result back to the working tree through
//! `git cat-file --filters`, so line endings and other checkout filters are applied as git would.

use std::collections::HashSet;

use gix::diff::blob::InternedInput;
use gix::object::tree::EntryKind;

use crate::diff::{Side, Texts, entry_kind_to_mode, line_diff, load_texts};
use crate::stage::worktree_path;
use crate::status::{staged_sides, unstaged_sides};
use crate::{Error, Repository, Result, StatusEntry};

/// Changed lines of a diff, by the line numbers the diff shows (1-based).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LineSelection {
    /// Removed lines, by their number in the old version.
    pub removed: Vec<u32>,
    /// Added lines, by their number in the new version.
    pub added: Vec<u32>,
}

impl LineSelection {
    pub fn is_empty(&self) -> bool {
        self.removed.is_empty() && self.added.is_empty()
    }
}

impl Repository {
    /// Stages the selected lines of `entry`'s unstaged diff.
    pub fn stage_lines(&self, entry: &StatusEntry, selection: &LineSelection) -> Result<()> {
        let repo = self.local();
        let workdir = self.require_workdir()?;
        self.check_partial(entry)?;
        let (old, new) = unstaged_sides(&repo, workdir, entry)?;
        let mode = mode_of(new).or_else(|| mode_of(old)).unwrap_or(0o100644);
        let (base, target, algorithm) = texts(&repo, Some(workdir), old, new)?;
        let staged = apply_selection(algorithm, &base, &target, selection, false)?;
        self.write_index_entry(entry, mode, &staged)
    }

    /// Moves the selected lines of `entry`'s staged diff back out of the index. The working tree
    /// keeps them.
    pub fn unstage_lines(&self, entry: &StatusEntry, selection: &LineSelection) -> Result<()> {
        let repo = self.local();
        self.require_workdir()?;
        self.check_partial(entry)?;
        let (old, new) = staged_sides(&repo, entry)?;
        let ((Side::Object { kind, .. }, _) | (_, Side::Object { kind, .. })) = (new, old) else {
            return Ok(());
        };
        let (head, index, algorithm) = texts(&repo, None, old, new)?;
        let staged = apply_selection(algorithm, &head, &index, selection, true)?;
        if matches!(old, Side::Missing) && staged.is_empty() {
            // Every line of a newly added file was unstaged: it is no longer staged at all.
            return self.unstage(std::slice::from_ref(entry));
        }
        self.write_index_entry(entry, entry_kind_to_mode(kind), &staged)
    }

    /// Throws away the selected lines of `entry`'s unstaged diff from the working tree.
    pub fn discard_lines(&self, entry: &StatusEntry, selection: &LineSelection) -> Result<()> {
        let repo = self.local();
        let workdir = self.require_workdir()?;
        self.check_partial(entry)?;
        let (old, new) = unstaged_sides(&repo, workdir, entry)?;
        let (index, worktree, algorithm) = texts(&repo, Some(workdir), old, new)?;
        let kept = apply_selection(algorithm, &index, &worktree, selection, true)?;
        // `kept` is in git's form (e.g. LF line endings); let git convert it for the working tree.
        let blob = self.hash_object(&kept)?;
        let mut path_arg = std::ffi::OsString::from("--path=");
        path_arg.push(
            gix::path::from_bstr(entry.path.as_bstr())
                .map_err(|_| {
                    Error::Unsupported(format!("{} is not a valid path here", entry.path))
                })?
                .as_os_str(),
        );
        let checkout_form = self.run_git(
            [
                std::ffi::OsString::from("cat-file"),
                "--filters".into(),
                path_arg,
                blob.into(),
            ],
            None,
        )?;
        let file = worktree_path(workdir, &entry.path)?;
        std::fs::write(&file, checkout_form).map_err(|source| Error::Io {
            context: "writing the file",
            source,
        })
    }

    /// Refuses files whose lines can't be taken apart: links, submodules, and files with a
    /// filter (such as Git LFS), whose stored content isn't what the diff shows.
    fn check_partial(&self, entry: &StatusEntry) -> Result<()> {
        let output = self.run_git(
            [
                std::ffi::OsString::from("check-attr"),
                "-z".into(),
                "filter".into(),
                "--".into(),
                gix::path::from_bstr(entry.path.as_bstr())
                    .map(|path| path.into_owned().into_os_string())
                    .unwrap_or_default(),
            ],
            None,
        )?;
        // `<path>\0filter\0<value>\0`
        let value = output.split(|b| *b == 0).nth(2).unwrap_or_default();
        if !matches!(value, b"unspecified" | b"unset" | b"") {
            return Err(Error::Unsupported(format!(
                "{} uses a filter ({}); it can only be staged as a whole",
                entry.path,
                String::from_utf8_lossy(value)
            )));
        }
        Ok(())
    }

    fn hash_object(&self, content: &[u8]) -> Result<String> {
        let id = self.run_git(
            ["hash-object", "-w", "--no-filters", "--stdin"],
            Some(content),
        )?;
        Ok(String::from_utf8_lossy(&id).trim().to_owned())
    }

    fn write_index_entry(&self, entry: &StatusEntry, mode: u32, content: &[u8]) -> Result<()> {
        let blob = self.hash_object(content)?;
        // `<mode> <id>\t<path>\0`, which takes any path bytes.
        let mut record = format!("{mode:o} {blob}\t").into_bytes();
        record.extend_from_slice(entry.path.as_bytes());
        record.push(0);
        self.run_git(["update-index", "-z", "--index-info"], Some(&record))
            .map(drop)
    }
}

fn mode_of(side: Side<'_>) -> Option<u32> {
    match side {
        Side::Object { kind, .. } | Side::Worktree { kind, .. } => Some(entry_kind_to_mode(kind)),
        Side::Missing => None,
    }
}

fn texts(
    repo: &gix::Repository,
    workdir: Option<&std::path::Path>,
    old: Side<'_>,
    new: Side<'_>,
) -> Result<(Vec<u8>, Vec<u8>, gix::diff::blob::Algorithm)> {
    for side in [old, new] {
        if let Side::Object { kind, .. } | Side::Worktree { kind, .. } = side
            && matches!(kind, EntryKind::Link | EntryKind::Commit)
        {
            return Err(Error::Unsupported(
                "Symbolic links and submodules can only be staged as a whole".into(),
            ));
        }
    }
    match load_texts(repo, workdir, old, new)? {
        Texts::Text {
            old,
            new,
            algorithm,
        } => Ok((old, new, algorithm)),
        Texts::Binary { .. } => Err(Error::Unsupported(
            "Binary files can only be staged as a whole".into(),
        )),
        Texts::TooLarge { .. } => Err(Error::Unsupported(
            "This file is too large to stage by lines".into(),
        )),
    }
}

/// `old` with the selected changes of the `old` → `new` diff applied, or with `invert`, with
/// every change *except* the selected ones applied.
pub(crate) fn apply_selection(
    algorithm: gix::diff::blob::Algorithm,
    old: &[u8],
    new: &[u8],
    selection: &LineSelection,
    invert: bool,
) -> Result<Vec<u8>> {
    let input = InternedInput::new(old, new);
    let diff = line_diff(algorithm, &input);
    let hunks: Vec<_> = diff.hunks().collect();

    // Line numbers are 1-based; token indexes are 0-based.
    let removed: HashSet<u32> = selection
        .removed
        .iter()
        .map(|line| line.wrapping_sub(1))
        .collect();
    let added: HashSet<u32> = selection
        .added
        .iter()
        .map(|line| line.wrapping_sub(1))
        .collect();
    let is_removed = |line: &u32| hunks.iter().any(|hunk| hunk.before.contains(line));
    let is_added = |line: &u32| hunks.iter().any(|hunk| hunk.after.contains(line));
    if !removed.iter().all(is_removed) || !added.iter().all(is_added) {
        return Err(Error::OutOfDate(
            "The file changed since its diff was shown. Refresh and try again.".into(),
        ));
    }

    let line = |token| -> &[u8] { input.interner[token] };
    let mut out: Vec<u8> = Vec::with_capacity(old.len().max(new.len()));
    let push = |out: &mut Vec<u8>, text: &[u8]| {
        // A kept last line without a newline is no longer last: give it the newline it needs.
        if !out.is_empty() && !out.ends_with(b"\n") {
            out.extend_from_slice(if text.ends_with(b"\r\n") {
                b"\r\n"
            } else {
                b"\n"
            });
        }
        out.extend_from_slice(text);
    };
    let mut position = 0;
    for hunk in &hunks {
        for index in position..hunk.before.start {
            push(&mut out, line(input.before[index as usize]));
        }
        for index in hunk.before.clone() {
            let apply_removal = removed.contains(&index) != invert;
            if !apply_removal {
                push(&mut out, line(input.before[index as usize]));
            }
        }
        for index in hunk.after.clone() {
            let apply_addition = added.contains(&index) != invert;
            if apply_addition {
                push(&mut out, line(input.after[index as usize]));
            }
        }
        position = hunk.before.end;
    }
    for index in position..input.before.len() as u32 {
        push(&mut out, line(input.before[index as usize]));
    }
    Ok(out)
}

/// Every changed line of the `old` → `new` diff, for selecting all of them.
#[cfg(test)]
pub(crate) fn select_all(old: &[u8], new: &[u8]) -> LineSelection {
    let input = InternedInput::new(old, new);
    let diff = line_diff(gix::diff::blob::Algorithm::Myers, &input);
    let mut selection = LineSelection::default();
    for hunk in diff.hunks() {
        selection.removed.extend(hunk.before.map(|index| index + 1));
        selection.added.extend(hunk.after.map(|index| index + 1));
    }
    selection
}

#[cfg(test)]
mod tests {
    use super::*;
    use gix::diff::blob::Algorithm::Myers;
    use proptest::prelude::*;

    fn apply(old: &str, new: &str, removed: &[u32], added: &[u32], invert: bool) -> String {
        let selection = LineSelection {
            removed: removed.to_vec(),
            added: added.to_vec(),
        };
        String::from_utf8(
            apply_selection(Myers, old.as_bytes(), new.as_bytes(), &selection, invert).unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn stages_one_of_two_additions() {
        let old = "a\nb\n";
        let new = "a\nx\nb\ny\n";
        assert_eq!(apply(old, new, &[], &[2], false), "a\nx\nb\n");
        assert_eq!(apply(old, new, &[], &[4], false), "a\nb\ny\n");
    }

    #[test]
    fn stages_part_of_a_replacement() {
        let old = "one\ntwo\nthree\n";
        let new = "one\nTWO\nthree\n";
        // Only the removal: the old line goes, the new one is not added yet.
        assert_eq!(apply(old, new, &[2], &[], false), "one\nthree\n");
        // Only the addition: both versions are present.
        assert_eq!(apply(old, new, &[], &[2], false), "one\ntwo\nTWO\nthree\n");
    }

    #[test]
    fn inverted_selection_keeps_everything_else() {
        let old = "a\nb\n";
        let new = "a\nx\nb\ny\n";
        assert_eq!(apply(old, new, &[], &[2], true), "a\nb\ny\n");
    }

    #[test]
    fn adds_a_missing_final_newline_when_lines_follow() {
        // The old file ends without a newline; staging only the new last line must not glue it
        // onto the old last line.
        assert_eq!(apply("a", "a\nb\n", &[], &[2], false), "a\nb\n");
    }

    #[test]
    fn rejects_lines_that_are_not_changes() {
        let selection = LineSelection {
            removed: vec![],
            added: vec![1],
        };
        let result = apply_selection(Myers, b"a\nb\n", b"a\nb\nc\n", &selection, false);
        assert!(matches!(result, Err(Error::OutOfDate(_))));
    }

    fn lines() -> impl Strategy<Value = String> {
        prop::collection::vec(
            prop::sample::select(vec!["a", "b", "c", "d", "{", "}", ""]),
            0..24,
        )
        .prop_map(|lines| lines.iter().map(|line| format!("{line}\n")).collect())
    }

    proptest! {
        #[test]
        fn all_or_nothing(old in lines(), new in lines()) {
            let all = select_all(old.as_bytes(), new.as_bytes());
            let none = LineSelection::default();
            prop_assert_eq!(apply_selection(Myers, old.as_bytes(), new.as_bytes(), &all, false).unwrap(), new.as_bytes());
            prop_assert_eq!(apply_selection(Myers, old.as_bytes(), new.as_bytes(), &none, false).unwrap(), old.as_bytes());
            prop_assert_eq!(apply_selection(Myers, old.as_bytes(), new.as_bytes(), &all, true).unwrap(), old.as_bytes());
            prop_assert_eq!(apply_selection(Myers, old.as_bytes(), new.as_bytes(), &none, true).unwrap(), new.as_bytes());
        }

        #[test]
        fn line_counts_add_up(old in lines(), new in lines(), picks in prop::collection::vec(any::<bool>(), 64)) {
            let all = select_all(old.as_bytes(), new.as_bytes());
            let mut pick = picks.iter().cycle();
            let selection = LineSelection {
                removed: all.removed.iter().copied().filter(|_| *pick.next().unwrap()).collect(),
                added: all.added.iter().copied().filter(|_| *pick.next().unwrap()).collect(),
            };
            let staged = apply_selection(Myers, old.as_bytes(), new.as_bytes(), &selection, false).unwrap();
            let count = |text: &[u8]| text.iter().filter(|b| **b == b'\n').count();
            prop_assert_eq!(
                count(&staged),
                count(old.as_bytes()) - selection.removed.len() + selection.added.len()
            );
            // Staging the selection, then everything else, ends at the new version.
            let rest = select_all(&staged, new.as_bytes());
            prop_assert_eq!(apply_selection(Myers, &staged, new.as_bytes(), &rest, false).unwrap(), new.as_bytes());
            // Unstaging the selection from the new version leaves exactly what wasn't selected.
            let unstaged = apply_selection(Myers, old.as_bytes(), new.as_bytes(), &selection, true).unwrap();
            prop_assert_eq!(
                count(&unstaged),
                count(new.as_bytes()) - selection.added.len() + selection.removed.len()
            );
        }
    }
}
