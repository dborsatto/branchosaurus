# Architecture

Branchosaurus is one Rust engine with a thin native app on each platform. This document explains
how the pieces fit, and the rules that keep the apps thin and fast.

```
┌──────────────────────────────────────────────────────────────┐
│ branchosaurus-core (Rust)                                    │
│  gitoxide reads · git CLI writes · history index + graph     │
│  lanes · diff hunks · status                                 │
└───────┬─────────────────────────┬────────────────────────┬───┘
        │ UniFFI                  │ direct (Rust)          │ UniFFI
        │ (branchosaurus-ffi)     │                        │ (branchosaurus-ffi)
┌───────▼────────┐       ┌────────▼────────┐      ┌────────▼───────┐
│ macOS          │       │ Linux           │      │ Windows        │
│ SwiftUI+AppKit │       │ GTK 4+libadwaita│      │ WinUI 3 (C#)   │
└────────────────┘       └─────────────────┘      └────────────────┘
```

## The rule for where code goes

**If two apps would have to implement the same logic, it belongs in the core.** The core returns
data that is ready to display: graph lanes and line segments, diff hunks with line numbers, status
grouped into staged, unstaged and conflicted. Apps own presentation, navigation and selection.
Dates and numbers are formatted in the app, so they follow the system's locale settings.

## Core (`crates/branchosaurus-core`)

### Reading: gitoxide

Reads use [gitoxide](https://github.com/GitoxideLabs/gitoxide) (`gix`):

- **Repositories and refs:** discovery, HEAD, branches with their upstreams, remote branches, tags.
- **Commits:** decoding, and tree-to-tree diffs with rename detection.
- **Status:** HEAD against the index, and the index against the working tree, untracked files
  included.
- **File contents:** blob reads, and working-tree files read through git's filters (`autocrlf`,
  `ident`, LFS), exactly as `git add` would see them.
- **Line diffs:** `gix::diff::blob` (imara-diff), with git's slider heuristics, 3 lines of context,
  binary detection like git's, and a size limit.

[`Repository`](../crates/branchosaurus-core/src/repo.rs) wraps a `gix::ThreadSafeRepository`, so
it can be shared between threads. Each operation works on its own thread-local handle.

### Writing: the git CLI

Every write and network operation runs the user's own `git`. That way hooks, commit signing,
credential helpers, SSH configuration and every other git setting behave exactly as they do in a
terminal. [`GitCli`](../crates/branchosaurus-core/src/cli.rs) runs it:

- **Finding git:** apps launched from the Dock don't get your shell's `PATH`, so it looks in the
  usual install locations first. It recognises macOS's `/usr/bin/git` placeholder when the
  developer tools are missing, and requires git 2.34 or newer.
- **Running git:** git runs with `GIT_TERMINAL_PROMPT=0`, `LC_ALL=C` and `core.quotepath=false`.
  Inherited variables are cleared when they would redirect git (`GIT_DIR`, `GIT_INDEX_FILE`…) or
  prompt elsewhere (`GIT_ASKPASS`, `SSH_ASKPASS`).
- **Long operations:** fetch, pull and push stream git's stderr. Progress lines
  (`Receiving objects:  45% (…)`) become progress callbacks, and cancelling kills git. Error
  messages keep git's own wording, minus the progress noise.

The core holds gitoxide handles while git rewrites files under them. Refs and the index are
re-read on every call, but gitoxide reads configuration only once, when it opens a repository.
So `Repository` reopens its handle whenever `.git/config` changes; git commands that set
upstreams or add remotes change it.

#### Staging

- **Whole files:** `git add --all`, `git reset`, `git restore --worktree`. Paths go through
  `--pathspec-from-file` on stdin with `--literal-pathspecs`, so there is no command-line length
  limit and no globbing of names like `*.txt`. Discarded untracked files go to the system trash.
- **Hunks and lines:** see [`partial.rs`](../crates/branchosaurus-core/src/partial.rs). Building
  a patch for `git apply` breaks on line endings, whitespace settings and missing final newlines.
  Instead, the core recomputes the same diff the app showed and builds the new file content
  itself, applying only the selected lines. It stores the result with
  `git hash-object -w --no-filters`, and points the index at it with `git update-index --index-info`.
  - **Discarding lines** writes the result back to the working tree through
    `git cat-file --filters`, so CRLF conversion and other checkout filters apply.
  - **Outdated selections:** lines are identified by their numbers in the diff. If the file
    changed and the selection no longer matches the diff, the operation fails with `OutOfDate`
    rather than guessing.
  - **Whole files only:** binary files, symbolic links, submodules and files with a `filter`
    attribute (Git LFS).
  - **Tests:** property tests check that selecting everything gives the new version, nothing gives
    the old one, and that line counts add up. Integration tests compare the result with
    `git diff --cached`.

#### Credentials

git and ssh ask for passwords and passphrases through an *askpass* program. For each remote
operation the core does the following:
1. It listens on a Unix socket in a private (`0700`) temporary directory.
2. It points `GIT_ASKPASS` and `SSH_ASKPASS` (with `SSH_ASKPASS_REQUIRE=force`) at the tiny
   [`branchosaurus-askpass`](../crates/branchosaurus-core/src/bin/askpass.rs) helper, which is
   bundled next to the app's executable.
3. The helper relays each prompt over the socket to a `CredentialPrompter` supplied by the app,
   and prints the answer back to git.

Credential helpers such as the macOS keychain run first, so people are only asked when nothing is
stored. An end-to-end test serves an HTTP endpoint that demands Basic authentication, and checks
both the prompts and the credentials git sends.

#### Watching

[`watch.rs`](../crates/branchosaurus-core/src/watch.rs) watches the working tree and `.git` with
`notify` (FSEvents, inotify, ReadDirectoryChangesW):
- **What it reports:** a working-copy, index or refs change, once things have been quiet for
  250 ms (or every 2 s during a continuous stream).
- **What it ignores:** writes to objects and logs, lock files, and gitignored files.
- **How the app uses it:** to refresh the status, and the history when refs change.

#### Status fallback

Status uses the CLI as a fallback. If the repository uses something gitoxide doesn't implement
(`core.fsmonitor`, a sparse or split index), `status` runs `git status --porcelain=v2 -z`
instead. The tests compare both implementations on every fixture.

### History: our own commit index

Showing history means sorting every reachable commit so each one comes before its parents, then
working out the graph lanes. gitoxide's built-in walk has no fast topological order, so the core
builds its own index ([`history/index.rs`](../crates/branchosaurus-core/src/history/index.rs)):

1. **Walk.** Start from every branch, remote branch, tag and HEAD. Commits are numbered densely,
   and their parents and committer times are stored as flat arrays, so a million commits take a
   few tens of megabytes.
   - When git's **commit-graph file** exists, parents and times come straight from it without
     decoding any commit.
   - Commits newer than the file, or all of them when there is no file, are decoded.
   - Shallow clones and missing objects are handled.
2. **Sort.** Kahn's algorithm with a priority queue on commit time: children before parents,
   otherwise newest first. This is `git log --date-order`, and the tests check that the order
   matches git's exactly.
3. **Lay out lanes.** See [`history/graph.rs`](../crates/branchosaurus-core/src/history/graph.rs):
   - Each lane waits for one commit. A commit takes the first lane waiting for it, sends its first
     parent straight down, and opens or joins lanes for its other parents.
   - A row's layout depends only on the rows above it. So instead of storing every row (wide
     histories pass dozens of lines through each one), the lane state is saved every 1,024 rows,
     and a page of rows is recomputed from the nearest checkpoint.
4. **Fill in details.** Subject, author and date are decoded only for the rows a page asks for,
   and cached.

The result is an immutable `History` snapshot. `History::rows(start, count)` replays at most
about a thousand lanes and decodes only the commits asked for, so the UI thread can call it
directly. That is what a virtualized table like `NSTableView` needs.

#### Measured performance

Measured on an Apple Silicon Mac in October 2026 with
`cargo run --release -p branchosaurus-core --example log`, loading the whole history and then
one page of 50 rows:

| Repository | Commits | commit-graph file | Full load | Peak memory | Page of 50 rows |
| --- | ---: | --- | ---: | ---: | ---: |
| git/git | 85,767 | yes | 31–35 ms | 34 MB | 0.35–0.45 ms |
| git/git | 85,767 | no | 0.47–0.55 s | 62 MB | 0.35–0.45 ms |
| rust-lang/rust | 349,696 | yes | 130–145 ms | 54 MB | 0.4–0.6 ms |
| rust-lang/rust | 349,696 | no | 1.8 s | 196 MB | 0.4–0.6 ms |

All figures are with the repository already in the operating system's file cache. Reading it from
disk for the first time costs more: about 110 ms for git/git, 240 ms for rust-lang/rust, and up to
about 4 ms for a page of rows.

Git writes the commit-graph file during `git gc` and `git maintenance`, so most working
repositories have one. A fresh clone doesn't, and decoding every commit is about ten times slower.
The app can offer to run `git commit-graph write` later.

### Branch labels and avatars

Two pieces of the commit list are decided in the core, so every app shows them the same way:
- **Branch labels:** a local branch and remote branches of the same name on the same commit
  (`main`, `origin/main`, `upstream/main`) are one `RefLabel` with `local` and `remotes`. The
  checked-out branch comes first.
- **Avatars:** [`avatar.rs`](../crates/branchosaurus-core/src/avatar.rs) gives an author's
  initials, a palette index derived from their email, and where their picture may be. That is
  GitHub for its no-reply addresses (`12345+name@users.noreply.github.com`), and Gravatar, by the
  SHA-256 of the email, otherwise. Apps download pictures with their own HTTP stack and cache, and
  let people turn downloading off, because Gravatar then learns which email hashes the app asks
  about.

### Syntax highlighting

Diff lines come with highlights from the core
([`highlight.rs`](../crates/branchosaurus-core/src/highlight.rs)). Each app maps them to its own
colors:
- **Grammars:** [syntect](https://github.com/trishume/syntect) with the Sublime Text syntaxes
  that [bat](https://github.com/sharkdp/bat) collects, packaged by
  [two-face](https://codeberg.org/CosmicHarper/two-face).
- **Choosing a language:** by file name (`Makefile`, `Dockerfile`), then extension, then the
  first line (`#!/usr/bin/env python3`).
- **What the apps get:** each highlight is a range of the line plus a `HighlightKind` (keyword,
  string, comment, number, type, function…). This is the kind of the innermost TextMate scope
  that has one, so punctuation takes the color of what it belongs to. There are no theme colors,
  so each platform uses its own palette and appearances.
- **Ranges:** the core uses UTF-8 byte ranges; the FFI converts them to UTF-16, the unit of
  `NSString` and C# strings.

A comment or string can open far above a hunk, so both versions of the file are parsed from
their first line:
- **Shared lines:** lines the two versions share are parsed once, while the two parsers' states
  are equal.
- **Stopping early:** parsing stops after the last line shown.
- **Size limit:** diffs that would need more than 256 KiB of parsing are shown plain. Grammars
  parse about 1.5 MB/s, so this keeps highlighting under about 200 ms.
- **Measured cost:** git's `sequencer.c` (7,090 lines, 200 KB) changed at its last line takes
  about 135 ms to diff and highlight. Without the shared parsing it took twice as long.

The grammars' licenses ship with the apps. `cargo run -p branchosaurus-core --example
acknowledgements` prints them, and `cargo xtask macos-app` writes them to the bundle's
`Resources/Acknowledgements.md`.

### Errors, cancellation, paths

- **Errors:** every public function returns `branchosaurus_core::Error`, whose messages are
  written to be shown to people.
- **Cancellation:** long operations take a `CancelToken`, which the history walk checks
  regularly.
- **Paths:** paths are `RepoPath`, raw bytes, because git paths need not be UTF-8. `display()`
  renders them for people.

## Bindings (`crates/branchosaurus-ffi`)

[UniFFI](https://mozilla.github.io/uniffi-rs/) generates Swift (and later C#) bindings from Rust
code:

- **Types:** the FFI crate mirrors the core's types one to one. Object ids become hex strings,
  paths become bytes plus a display string, durations become milliseconds. The core itself has no
  UniFFI annotations, so the GTK app uses it directly.
- **Blocking calls:** calls block, and apps make them off the UI thread. There are no `async`
  exports, because UniFFI's async support is incomplete for both Swift 6 and C#. Progress is
  reported through callback interfaces (`HistoryObserver`), and cancellation through a
  `CancelToken` object.
- **Paging:** `History` is an object, not a list, so a history never crosses the boundary all at
  once. The app pages through it, and the macOS app caches pages of 256 rows.
- **Windows bindings:** the project uses the latest UniFFI (0.32).
  [uniffi-bindgen-cs](https://github.com/NordSecurity/uniffi-bindgen-cs), which the Windows app
  will use, still targets 0.31; its upgrade is
  [NordSecurity/uniffi-bindgen-cs#176](https://github.com/NordSecurity/uniffi-bindgen-cs/pull/176).
  Check where that stands before starting the Windows app.

## macOS app (`apps/macos`)

The app is a SwiftPM package, buildable with Xcode or just the Command Line Tools. It has three
targets:

- **`BranchosaurusFFI`:** a C target holding the UniFFI header and module map, which links the
  Rust static library from `target/macos/lib`.
- **`BranchosaurusCore`:** the generated Swift bindings, plus small extensions.
- **`Branchosaurus`:** the app itself.

All of them compile in Swift 6 language mode, with strict concurrency checking.

`cargo xtask macos-app` generates the bindings, builds, and assembles and ad-hoc signs
`target/macos/Branchosaurus.app`.

The app uses SwiftUI for structure: one window per repository, a `NavigationSplitView` sidebar,
and the commit details. The two lists that can be enormous, the commit list and the diff, are
`NSTableView`s with custom-drawn cells, because they only ever create views for the rows on
screen. Everything uses system components and colors, so the app follows the current macOS
design, dark mode, accent color and accessibility settings without extra work.

The Changes view stages whole files, hunks and selected lines. It discards changes after
confirmation, and commits or amends with a message editor (⌘↩). Fetch, pull and push run with a
progress HUD and a Cancel button. Credential prompts are an `NSAlert` with a secure text field,
shown from the askpass relay. The app also does the following:
- **Branches:** checkout and branch creation from the sidebar and from the commit list's context
  menu.
- **Refreshing:** refreshes whenever the watcher reports a change.
- **Opening folders:** opens folders dropped on its icon or chosen with Open With.
- **Commit list:** a graph in the style of GitKraken.
  - **Branches and tags:** pills on the left, in their lane's color, joined to their commit by a
    line. A check marks the checked-out branch; a laptop and a cloud say whether a branch is
    local, on a remote, or both. "+N" counts more labels on the same commit.
  - **Commits:** drawn as their author's avatar, ringed in the lane's color; merges are small
    rings. Pictures come through `AvatarStore` (a disk cache, a few downloads at a time, the most
    recently scrolled-to first), with initials until they arrive or when turned off in Settings.
  - **Lines:** they change lanes with rounded corners, and a tinted band leads from each commit
    to its message.
  - **Wide graphs:** the graph column fits up to 12 lanes by default and can be resized. Past
    its width, lines fade out (they are drawn on their own layer and erased with a gradient), and
    commits in lanes that don't fit are pinned to the last lane that does, with a shadow and a
    "›", so every row still shows its commit.
  - **Dates:** there is no date or hash column. Separator lines group commits by age ("4 hours
    ago", "yesterday", "3 days ago"), by commit time, which the list is sorted by, so groups
    don't interleave. Ages use `RelativeDateTimeFormatter`, so they follow the system language,
    and they move on every minute. The author and exact date are in the avatar's tooltip, and
    everything else in the commit details.
- **Diffs:** colors highlighted code with system colors (`systemPink` keywords, `systemRed`
  strings, secondary-label comments…), so it adapts to dark mode and Increase Contrast. Tabs are
  drawn at four-column tab stops.

## Linux and Windows apps

These are planned; see the [roadmap](ROADMAP.md).

- **Linux:** a GTK 4 + libadwaita app in Rust (`gtk4-rs`) that uses the core directly.
  `GtkColumnView` over list models backed by `History::rows`.
- **Windows:** a WinUI 3 app in C# (Windows App SDK), using bindings from uniffi-bindgen-cs, with
  the same paging model as macOS.
