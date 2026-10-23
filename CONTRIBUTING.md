# Contributing to Branchosaurus

Thanks for helping. This file covers how to build, test and change the code. Read
[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) first: it explains where each kind of code belongs.

## Setup

- Docker (with Compose), for Linux builds, lints and tests.
- [rustup](https://rustup.rs) and git 2.34 or newer, for macOS builds.
- macOS: Xcode, or only the Command Line Tools.

## One Rust version everywhere

`rust-toolchain.toml` pins the exact Rust version, and every build reads it:

- **Docker:** `docker/Dockerfile` is built from the official `rust:<version>` image (the Makefile
  passes the version in).
- **macOS:** builds run `rustup run <version> cargo`. They never use whatever `cargo` comes first
  on your `PATH`; Homebrew's Rust, for instance, builds apps that only start on the macOS version
  it was installed on.
- **CI:** the Linux job runs `make check` in the same image; the macOS job uses the Makefile; the
  Windows job installs the pinned version with rustup.

To upgrade Rust, change `rust-toolchain.toml` and nothing else.

## Everyday commands

| What | Command |
| --- | --- |
| Lints and tests, as CI's Linux job runs them | `make check` (or `make lint`, `make test`) |
| Formatting | `make fmt` |
| A shell in the Linux build environment | `make shell` |
| Build the macOS app | `make macos-app` |
| Universal release build | `make macos-release` |
| Swift tests | `make macos-test` |
| Regenerate Swift bindings | `make bindings` |
| History benchmark | `make bench REPO=path/to/repo` |
| Redraw the app icon after editing its script | `cd apps/macos && swift Scripts/make-icon.swift` |
| Review graph snapshot changes | `cargo insta review` (needs `cargo install cargo-insta`) |

The Docker targets keep their cargo registry and build output in a Docker volume, so Linux
artifacts never mix with the macOS ones in `target/`. `make clean` removes both.

The Swift bindings in `apps/macos/Sources/BranchosaurusFFI/include` and
`apps/macos/Sources/BranchosaurusCore/Generated` are generated and not committed. Every macOS
target regenerates them, so run one after changing `branchosaurus-ffi`.

### macOS with only the Command Line Tools

The build (`cargo xtask`, which the Makefile runs) notices when there is no Xcode and adapts:

- SwiftPM's default build engine needs Xcode, so it builds with `--build-system native`.
- SwiftUI in SDKs newer than macOS 26 needs a macro plugin that only Xcode ships, so it builds
  against the newest macOS 26 SDK the Command Line Tools include. Set `SDKROOT` to override.

If you use `swift build` directly, pass the same options.

### Checking the UI from scripts

Debug builds can save PNGs of their own windows, which helps check the UI without clicking around
and without screen-recording permissions:

```bash
BRANCHOSAURUS_SNAPSHOT=/tmp/shot BRANCHOSAURUS_SNAPSHOT_QUIT=1 \
  target/macos/Branchosaurus.app/Contents/MacOS/Branchosaurus \
  -ApplePersistenceIgnoreState YES -repository ~/code/repo
```

`-ApplePersistenceIgnoreState YES` stops macOS from reopening the windows of your last session,
which would take the place of `-repository`.

`BRANCHOSAURUS_APPEARANCE=dark` forces dark mode and `BRANCHOSAURUS_SIDEBAR=changes` opens the
Changes view. `apps/macos/Sources/Branchosaurus/Support/DebugSnapshot.swift` lists every option.

## Tests

- **Core behaviour is tested against git itself.** `crates/branchosaurus-core/tests` builds
  throwaway repositories with the git CLI (isolated from your own git configuration) and checks
  that the core agrees with `git rev-list`, `git status --porcelain=v2` and `git diff`. New read
  features should come with a test like that.
- **Graph layouts are snapshot tests.** Each topology in `tests/history.rs` renders as ASCII in
  `tests/snapshots`. Review changed snapshots by eye before accepting them.
- **The Swift tests** only check that data crosses the FFI boundary correctly; behaviour belongs
  in the Rust tests.

## Code conventions

- Every source file starts with `SPDX-License-Identifier: GPL-3.0-or-later`.
- The core holds the logic, the shells hold presentation. If you are about to write the same
  thing in Swift and C#, put it in the core instead.
- Core calls block. Shells call them off the UI thread, except `History::rows`, which is designed
  to be cheap enough for the UI thread.
- Paths are bytes in the core (`RepoPath`), because git paths need not be UTF-8. Convert to
  strings only for display.

## License

By contributing you agree that your contributions are licensed under the GPL, version 3 or later.
