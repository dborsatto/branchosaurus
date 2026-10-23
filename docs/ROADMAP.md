# Roadmap

The first public release, **v0.1**, is a Git client you can use every day for the common work:
browse history, review changes, stage, commit, switch branches, fetch, pull and push, on macOS,
Linux and Windows.

## M0 — Foundation (done)

- Cargo workspace, GPL-3.0 license, CI on macOS, Linux and Windows.
- Core, read side:
  - open and discover repositories; HEAD and repository state;
  - branches, remote branches, tags;
  - full history with graph lanes, paged;
  - commit details with per-file line counts;
  - working-copy status;
  - diffs of commits, staged changes and unstaged changes.
- Tests that compare the core with the git CLI, and snapshot tests of graph layouts.
- UniFFI bindings for Swift.
- macOS app, read-only: welcome window, sidebar, history with graph, commit details, diffs,
  working-copy changes, light and dark mode.

## M1 — macOS write path → v0.1 on macOS (in progress)

Done:
- **Refresh:** the working copy and `.git` are watched (`notify`), debounced, ignoring object
  writes, lock files and gitignored files.
- **Staging:** stage, unstage and discard whole files, hunks and selected lines.
  - The core builds the new index content itself (`git hash-object` + `git update-index`)
    instead of going through `git apply`.
  - Property tests and comparisons with `git diff --cached` cover it.
  - Binary, LFS, symlink and submodule changes are staged whole.
  - Discarded untracked files go to the Trash.
- **Committing:** commit and amend with `git commit -F -`, so hooks and signing work.
- **Branches:** check out local and remote branches and commits; create branches from HEAD or
  any commit.
- **Remotes:** fetch, pull (as configured, fast-forward only, merge or rebase) and push (setting
  the upstream for new branches; force push with lease), with progress, cancellation and
  credential prompts through a bundled askpass helper.
- **Opening repositories:** folders can be opened from Finder and the Dock.
- **Large repositories:** the app offers to write the commit-graph file when it is missing.
- **Commit graph:** in the style of GitKraken: branch pills on the left, author avatars as
  commits, rounded lines, and commits grouped by age instead of dates and hashes.
- **Syntax highlighting:** diffs are highlighted by the core, for every app, in the system's
  colors on macOS.
- **Builds:** one pinned Rust version for every build (`rust-toolchain.toml`, Docker for Linux,
  the Makefile for macOS).
- **App icon:** a stylized brachiosaurus whose spots form a small commit graph, drawn by
  `apps/macos/Scripts/make-icon.swift`.

Still to do for v0.1 on macOS:
- **Release:** signed, notarized and stapled builds (needs an Apple Developer ID).
- **Conflicts:** surface merges and rebases stopped by conflicts in the UI, with Continue and
  Abort.
- **Keyboard:** keyboard shortcuts for moving between files and staging them.

## M2 — Linux app

- GTK 4 + libadwaita app in `apps/linux`, using the core directly.
- `GtkColumnView` over list models backed by `History::rows`, with the graph drawn in a custom
  widget.
- The same features as macOS v0.1.
- Flatpak packaging, with a plan for running the host's git and SSH from inside the sandbox.

## M3 — Windows app → v0.1 on every platform

- WinUI 3 app (C#, Windows App SDK) in `apps/windows`, using bindings from uniffi-bindgen-cs.
  That generator needs to support the project's UniFFI version (0.32) first; see
  [NordSecurity/uniffi-bindgen-cs#176](https://github.com/NordSecurity/uniffi-bindgen-cs/pull/176).
- Batch git invocations (`--pathspec-from-file`), because starting processes is slow on Windows.
  Support long paths.
- MSIX packaging and winget.

## After v0.1

- **Integrating changes:** stash, merge, rebase, interactive rebase, cherry-pick, revert.
- **Conflicts:** a conflict resolution view.
- **History tools:** blame, file history, search.
- **Refs:** tag management.
- **Repository structure:** worktrees and submodules.
- **Graph:** fewer lanes for very wide histories, and a first-parent view.
