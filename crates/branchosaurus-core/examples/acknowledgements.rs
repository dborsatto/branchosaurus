// SPDX-License-Identifier: GPL-3.0-or-later

//! Prints the licenses of the bundled syntax definitions as Markdown. `cargo xtask macos-app`
//! ships the output in the app bundle.
//!
//! ```text
//! cargo run -p branchosaurus-core --example acknowledgements > Acknowledgements.md
//! ```

fn main() {
    print!("{}", branchosaurus_core::syntax_acknowledgements());
}
