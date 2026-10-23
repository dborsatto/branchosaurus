// SPDX-License-Identifier: GPL-3.0-or-later

//! A compact, in-memory index of every commit reachable from the repository's refs.
//!
//! Commits are numbered densely ("nodes") in discovery order. Parents are stored as node numbers
//! in one flat array, so a 1M-commit repository costs roughly 40 MB. When the repository has a
//! commit-graph file, parents and commit times come from it without decoding any commit object.

use std::collections::{BinaryHeap, HashMap, HashSet};

use gix::ObjectId;
use gix::prelude::FindExt;

use crate::error::Context;
use crate::{CancelToken, Result};

const NONE: u32 = u32::MAX;

#[derive(Debug, Default)]
pub(crate) struct CommitIndex {
    pub ids: Vec<ObjectId>,
    /// Committer time, seconds since the epoch.
    pub times: Vec<i64>,
    parent_ranges: Vec<(u32, u32)>,
    parents: Vec<u32>,
    pub stats: IndexStats,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct IndexStats {
    /// Commits whose parents came from the commit-graph file.
    pub from_commit_graph: u32,
    /// Commits that had to be decoded from the object database.
    pub decoded: u32,
    /// Parents that are referenced but not present (shallow or partial clones, corruption).
    pub missing_parents: u32,
}

impl CommitIndex {
    pub fn len(&self) -> usize {
        self.ids.len()
    }

    pub fn parents(&self, node: u32) -> &[u32] {
        let (start, len) = self.parent_ranges[node as usize];
        &self.parents[start as usize..(start + len) as usize]
    }

    /// Walks everything reachable from `tips`. `progress` is called with the number of commits
    /// indexed so far, every few thousand commits.
    pub fn build(
        repo: &gix::Repository,
        tips: &[ObjectId],
        cancel: &CancelToken,
        progress: &mut dyn FnMut(u32),
    ) -> Result<Self> {
        let graph = repo
            .commit_graph_if_enabled()
            .context("reading the commit-graph file")?;
        let shallow: HashSet<ObjectId> = repo
            .shallow_commits()
            .context("reading the list of shallow commits")?
            .map(|commits| commits.iter().copied().collect())
            .unwrap_or_default();

        let mut builder = Builder {
            index: CommitIndex::default(),
            graph_nodes: vec![NONE; graph.as_ref().map_or(0, |g| g.num_commits() as usize)],
            graph_positions: Vec::new(),
            other_nodes: HashMap::new(),
            pending: Vec::new(),
            graph: graph.as_ref(),
        };
        for tip in tips {
            builder.intern(tip);
        }

        let mut buf = Vec::new();
        let mut parent_ids = Vec::new();
        while let Some(node) = builder.pending.pop() {
            let node_idx = node as usize;
            let start = builder.index.parents.len() as u32;
            let graph_position = builder.graph_positions[node_idx];
            let id = builder.index.ids[node_idx];
            let is_shallow = !shallow.is_empty() && shallow.contains(&id);

            if let (Some(graph), true) = (builder.graph, graph_position != NONE) {
                let commit = graph.commit_at(gix::commitgraph::Position(graph_position));
                builder.index.times[node_idx] = commit.committer_timestamp() as i64;
                if !is_shallow {
                    for parent in commit.iter_parents() {
                        match parent {
                            Ok(position) => {
                                let parent = builder.intern_position(position.0);
                                builder.index.parents.push(parent);
                            }
                            Err(_) => builder.index.stats.missing_parents += 1,
                        }
                    }
                }
                builder.index.stats.from_commit_graph += 1;
            } else {
                parent_ids.clear();
                match repo.objects.find_commit(&id, &mut buf) {
                    Ok(commit) => {
                        builder.index.times[node_idx] =
                            commit.committer().map_or(0, |sig| sig.seconds());
                        if !is_shallow {
                            parent_ids.extend(commit.parents().map(|id| id.to_owned()));
                        }
                    }
                    // A tip that isn't a commit (a tag on a tree, say) or a missing object.
                    Err(_) => builder.index.stats.missing_parents += 1,
                }
                for parent in &parent_ids {
                    if repo.has_object(parent) {
                        let parent = builder.intern(parent);
                        builder.index.parents.push(parent);
                    } else {
                        builder.index.stats.missing_parents += 1;
                    }
                }
                builder.index.stats.decoded += 1;
            }
            let len = builder.index.parents.len() as u32 - start;
            builder.index.parent_ranges[node_idx] = (start, len);

            let done = builder.index.stats.from_commit_graph + builder.index.stats.decoded;
            if done.is_multiple_of(16_384) {
                cancel.check()?;
                progress(done);
            }
        }
        Ok(builder.index)
    }

    /// Orders nodes so that every commit comes before its parents, and otherwise newest first
    /// by committer date. This is `git log --date-order`.
    pub fn date_order(&self, cancel: &CancelToken) -> Result<Vec<u32>> {
        let len = self.len();
        let mut children = vec![0u32; len];
        for &parent in &self.parents {
            children[parent as usize] += 1;
        }
        // Ties are broken by discovery order, so tips listed first come first.
        let mut ready: BinaryHeap<(i64, std::cmp::Reverse<u32>)> = (0..len as u32)
            .filter(|&node| children[node as usize] == 0)
            .map(|node| (self.times[node as usize], std::cmp::Reverse(node)))
            .collect();
        let mut order = Vec::with_capacity(len);
        while let Some((_, std::cmp::Reverse(node))) = ready.pop() {
            order.push(node);
            for &parent in self.parents(node) {
                let remaining = &mut children[parent as usize];
                *remaining -= 1;
                if *remaining == 0 {
                    ready.push((self.times[parent as usize], std::cmp::Reverse(parent)));
                }
            }
            if order.len().is_multiple_of(65_536) {
                cancel.check()?;
            }
        }
        if order.len() < len {
            // Only possible with a cyclic history (replace refs or grafts); show the rest anyway.
            let mut seen = vec![false; len];
            for &node in &order {
                seen[node as usize] = true;
            }
            order.extend((0..len as u32).filter(|&node| !seen[node as usize]));
        }
        Ok(order)
    }
}

struct Builder<'g> {
    index: CommitIndex,
    /// commit-graph position → node, or `NONE`.
    graph_nodes: Vec<u32>,
    /// node → commit-graph position, or `NONE`.
    graph_positions: Vec<u32>,
    /// Nodes for commits that are not in the commit-graph file.
    other_nodes: HashMap<ObjectId, u32>,
    pending: Vec<u32>,
    graph: Option<&'g gix::commitgraph::Graph>,
}

impl Builder<'_> {
    fn intern(&mut self, id: &gix::oid) -> u32 {
        if let Some(position) = self.graph.and_then(|graph| graph.lookup(id)) {
            return self.intern_position(position.0);
        }
        if let Some(&node) = self.other_nodes.get(id) {
            return node;
        }
        let node = self.push(id.to_owned(), NONE);
        self.other_nodes.insert(id.to_owned(), node);
        node
    }

    fn intern_position(&mut self, position: u32) -> u32 {
        let existing = self.graph_nodes[position as usize];
        if existing != NONE {
            return existing;
        }
        let graph = self.graph.expect("positions only come from the graph");
        let id = graph.id_at(gix::commitgraph::Position(position)).to_owned();
        let node = self.push(id, position);
        self.graph_nodes[position as usize] = node;
        node
    }

    fn push(&mut self, id: ObjectId, graph_position: u32) -> u32 {
        let node = self.index.ids.len() as u32;
        self.index.ids.push(id);
        self.index.times.push(0);
        self.index.parent_ranges.push((0, 0));
        self.graph_positions.push(graph_position);
        self.pending.push(node);
        node
    }
}
