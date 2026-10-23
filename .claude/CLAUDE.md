# Branchosaurus: notes for Claude

A native, cross-platform Git client. It has a shared Rust core (`crates/branchosaurus-core`),
UniFFI bindings (`crates/branchosaurus-ffi`), and a thin native app per platform: SwiftUI +
AppKit in `apps/macos` today, GTK 4 and WinUI 3 later. Read these instead of guessing:
- [`docs/ARCHITECTURE.md`](../docs/ARCHITECTURE.md): how the pieces fit, and why.
- [`docs/ROADMAP.md`](../docs/ROADMAP.md): what's done, what's next.
- [`CONTRIBUTING.md`](../CONTRIBUTING.md): the workflow.

## Commands

Always go through `make`, which uses the Rust version pinned in `rust-toolchain.toml`.

| What | Command |
| --- | --- |
| Lint and test, in Docker (what CI's Linux job runs) | `make check`, or `make lint` / `make test` |
| Format Rust code | `make fmt` |
| Build the macOS app | `make macos-app` → `target/macos/Branchosaurus.app` |
| Swift tests | `make macos-test` |
| Regenerate the Swift bindings after changing `branchosaurus-ffi` | `make bindings` |
| Which Rust macOS builds use | `make which-rust` |

Never run bare `cargo` on the host: Homebrew's comes first on `PATH`. For a one-off command, call
the pinned toolchain's own binaries:

```sh
T=$(rustup run "$(sed -n 's/^channel *= *"\(.*\)"/\1/p' rust-toolchain.toml)" rustc --print sysroot)/bin
PATH="$T:$PATH" RUSTC="$T/rustc" "$T/cargo" test -p branchosaurus-core --test write
```

`rustup run … cargo` alone is not enough here: Homebrew's rustup leaves `PATH` alone, so cargo
picks up Homebrew's `rustc`.

## Where code goes

- If two apps would need the same logic, it belongs in the core. Apps only present.
- Reads use gitoxide (`gix`). Writes and network operations run the user's `git` through
  `GitCli` (`cli.rs`), so hooks, signing and credential helpers apply.
- Paths are bytes (`RepoPath`); convert to strings only for display.
- Core calls block. Apps call them off the UI thread, except `History::rows`, which is cheap by
  design.
- FFI types mirror core types one to one, in `branchosaurus-ffi/src/lib.rs`. The generated
  bindings are not committed.

## Tests

- **Read features:** compare with the git CLI on throwaway repositories, using
  `crates/branchosaurus-core/tests/support` (isolated from the user's git configuration, with
  fixed dates).
- **Write features:** check the result with git itself (`git status --porcelain`,
  `git diff --cached --numstat`, `git show :path`).
- **Partial staging** (`partial.rs`) has property tests; keep them passing.
- **Graph snapshots** (`tests/snapshots`): look at changed layouts before accepting them
  (`INSTA_UPDATE=always`), and delete leftover `.snap.new` files.
- **Swift tests** only check data crossing the FFI. Their `Fixture` exports an isolated git
  environment, because the core inherits the test process's environment.

## Checking the UI

The debug app saves PNGs of its own windows. Never screenshot the whole screen: it captures
whatever else the user has open.

```sh
BRANCHOSAURUS_SNAPSHOT=/tmp/shot BRANCHOSAURUS_SNAPSHOT_QUIT=1 \
  target/macos/Branchosaurus.app/Contents/MacOS/Branchosaurus \
  -ApplePersistenceIgnoreState YES -repository /path/to/scratch/clone
```

- **Always pass `-ApplePersistenceIgnoreState YES`:** without it macOS restores the user's last
  windows, which can be their own repositories, instead of opening `-repository`.
- **More options:** `BRANCHOSAURUS_SNAPSHOT_DELAY` (seconds), `BRANCHOSAURUS_APPEARANCE=dark`,
  `BRANCHOSAURUS_SIDEBAR=changes`.
- **Which repositories:** use scratch clones (git/git, or rust-lang/rust with `--filter=tree:0`
  for history benchmarks), not the user's own repositories.
- **Benchmarks:** `make bench REPO=…` times history loading. Repositories with and without a
  commit-graph file differ about tenfold.

## Gotchas found the hard way

- **Launch arguments:** a bare path on the app's command line is taken by AppKit as a document to
  open. Use `-repository <path>`, or `open -a target/macos/Branchosaurus.app <folder>`.
- **Command Line Tools only (no Xcode):** `xtask` builds with `--build-system native` and the
  macOS 26 SDK, because SwiftUI in SDK 27 needs Xcode's macro plugin. Don't use APIs newer than
  macOS 26, and keep the deployment target at macOS 14.
- **Swift naming:** don't name free functions after SwiftUI modifiers. A function called
  `background` broke every view.
- **Custom views:** since macOS 14, custom `NSView`s draw outside their bounds unless you set
  `clipsToBounds = true`.
- **Linking:** the Rust static library must sit alone in `target/macos/lib`. Next to the `.dylib`,
  the linker picks the dylib.
- **Stale config:** gitoxide caches repository configuration when it opens a repository.
  `Repository::local()` reopens the handle when `.git/config` changes; refs and the index are
  re-read anyway.
- **Shell on macOS:** BSD `grep -Z` is not NUL output (use `--null`); zsh doesn't word-split
  `$variables`; `sed -i ''`.
- **Errors on stdout:** some git failures explain themselves on stdout (`git commit` with nothing
  staged); `GitCli` reports stdout when stderr is empty.
