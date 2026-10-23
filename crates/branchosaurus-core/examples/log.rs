// SPDX-License-Identifier: GPL-3.0-or-later

//! Prints a repository's history with its graph, and how long loading took.
//!
//! ```text
//! cargo run --release -p branchosaurus-core --example log -- <repo> [rows] [start]
//! ```

use std::time::Instant;

use branchosaurus_core::{CancelToken, History, Repository, render_ascii};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let path = args.next().unwrap_or_else(|| ".".into());
    let count: u32 = args.next().map_or(Ok(40), |arg| arg.parse())?;
    let start: u32 = args.next().map_or(Ok(0), |arg| arg.parse())?;

    let started = Instant::now();
    let repo = Repository::discover(&path)?;
    let history = History::load(&repo, &CancelToken::new(), &mut |_| {})?;
    let loaded = started.elapsed();

    let page_started = Instant::now();
    let rows = history.rows(start, count)?;
    let page = page_started.elapsed();

    print!("{}", render_ascii(&rows));
    let stats = history.stats();
    eprintln!(
        "\n{} commits ({} from commit-graph, {} decoded, {} missing parents), max {} lanes",
        stats.commits,
        stats.index.from_commit_graph,
        stats.index.decoded,
        stats.index.missing_parents,
        history.max_lanes()
    );
    eprintln!(
        "index {:?}, sort + layout {:?}, total load {:?}, page of {} rows at {} in {:?}",
        stats.index_time,
        stats.layout_time,
        loaded,
        rows.len(),
        start,
        page
    );
    Ok(())
}
