# Branchosaurus

An open source Git client that looks native and runs fast on macOS, Linux and Windows. No
Electron, no web views: each platform gets a real native app, and all of them share one Rust
engine.

> **Status:** early development. The macOS app browses history and diffs; stages files, hunks
> and lines; commits; switches and creates branches; and fetches, pulls and pushes. Linux and
> Windows apps come next. See the [roadmap](docs/ROADMAP.md).

## Why another Git client?

- **Native on every platform.** SwiftUI and AppKit on macOS, GTK 4 and libadwaita on Linux,
  WinUI 3 on Windows. System controls, fonts, colors, dark mode and accessibility.
- **Fast on huge repositories.** The full history of rust-lang/rust (350,000 commits) loads in
  about 130 ms when git's commit-graph file is present. The list only ever draws the rows on
  screen.
- **Behaves like git.** Reads go through [gitoxide](https://github.com/GitoxideLabs/gitoxide).
  Writes and network operations run your own `git`, so hooks, commit signing, credential helpers
  and SSH configuration work exactly as they do in the terminal.

## How it's built

```
crates/branchosaurus-core    Rust: the git engine and UI-ready data (graph lanes, diff hunks, status)
crates/branchosaurus-ffi     Rust: UniFFI bindings for the Swift and C# apps
xtask                        Rust: build tasks (bindings, app bundles)
apps/macos                   SwiftUI + AppKit app (SwiftPM package)
apps/linux                   GTK 4 + libadwaita app (planned)
apps/windows                 WinUI 3 app (planned)
```

Each app is a thin layer over the same core: if two apps would need the same logic, it belongs in
the core. [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) has the details.

## Building from source

Builds go through `make`, so they all use the Rust version pinned in `rust-toolchain.toml`. Linux
builds, lints and tests run in Docker. macOS builds run natively, because Apple's SDK only exists
on macOS, using the same Rust version through [rustup](https://rustup.rs).

You need:
- Docker, for `make check`;
- rustup and git 2.34 or newer, for macOS builds;
- on macOS, Xcode or only the Command Line Tools (`xcode-select --install`).

Lint and test the core, in Docker:

```bash
make check
```

Build and open the macOS app:

```bash
make macos-app
```

```bash
open target/macos/Branchosaurus.app
```

To open a repository straight away:

```bash
open -a target/macos/Branchosaurus.app ~/code/some-repo
```

`make help` lists everything else. [CONTRIBUTING.md](CONTRIBUTING.md) covers the development
workflow.

## License

Branchosaurus is free software, licensed under the [GNU General Public License, version 3 or
later](LICENSE).
