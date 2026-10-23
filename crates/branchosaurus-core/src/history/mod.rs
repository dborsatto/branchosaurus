// SPDX-License-Identifier: GPL-3.0-or-later

//! The commit list with its graph, as an immutable snapshot that shells page through.

mod graph;
mod index;

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use gix::ObjectId;
use gix::bstr::ByteSlice;

pub use graph::{EdgeKind, GraphEdge, GraphRow};
pub use index::IndexStats;

use crate::refs::Refs;
use crate::repo::Head;
use crate::{CancelToken, Repository, Result};
use graph::GraphLayout;
use index::CommitIndex;

/// How many decoded commits [`History`] keeps around for repeated `rows` calls.
const METADATA_CACHE_CAPACITY: usize = 8192;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistoryPhase {
    /// Reading commits; `commits` is how many so far.
    Indexing,
    /// Sorting and laying out the graph.
    Layout,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HistoryProgress {
    pub phase: HistoryPhase,
    pub commits: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HistoryStats {
    pub commits: u32,
    pub index: IndexStats,
    pub index_time: Duration,
    pub layout_time: Duration,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryRow {
    pub id: ObjectId,
    pub summary: String,
    pub author_name: String,
    pub author_email: String,
    /// Seconds since the epoch.
    pub author_time: i64,
    /// When the commit was made, in seconds since the epoch. History is sorted by this.
    pub commit_time: i64,
    /// The checked-out branch (or detached HEAD) first, then other local branches, branches only
    /// on remotes, and tags.
    pub refs: Vec<RefLabel>,
    pub graph: GraphRow,
}

/// A name on a commit. A local branch and remote branches of the same name on the same commit
/// (`main`, `origin/main`) are one label, as apps show them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RefLabel {
    pub kind: RefLabelKind,
    /// The branch name without a remote (`main`), the tag name, or `HEAD`.
    pub name: String,
    /// Branches: a local branch of this name points here.
    pub local: bool,
    /// Branches: the remotes whose branch of this name points here, e.g. `origin`.
    pub remotes: Vec<String>,
    /// The branch HEAD points to, or the detached HEAD itself.
    pub is_head: bool,
}

impl RefLabel {
    /// The label as git spells it: `main`, or `origin/main` for a branch only on a remote.
    pub fn display_name(&self) -> String {
        match (self.local, self.remotes.as_slice()) {
            (false, [remote, ..]) => format!("{remote}/{}", self.name),
            _ => self.name.clone(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefLabelKind {
    /// A detached HEAD.
    Head,
    Branch,
    Tag,
}

#[derive(Clone, Debug)]
struct CommitMeta {
    summary: String,
    author_name: String,
    author_email: String,
    author_time: i64,
}

/// Every commit reachable from the repository's branches, remote branches, tags and HEAD, in
/// display order.
///
/// Building one reads the whole history once. Afterwards [`rows`](Self::rows) is cheap and safe to
/// call from a UI thread: it replays at most ~1000 graph rows and decodes only the requested
/// commits.
pub struct History {
    index: CommitIndex,
    order: Vec<u32>,
    row_of_node: Vec<u32>,
    layout: GraphLayout,
    labels: HashMap<u32, Vec<RefLabel>>,
    stats: HistoryStats,
    repo: Mutex<gix::Repository>,
    metadata: Mutex<MetadataCache>,
    /// Nodes sorted by object id, built on the first lookup by id.
    nodes_by_id: OnceLock<Vec<u32>>,
}

impl std::fmt::Debug for History {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("History")
            .field("stats", &self.stats)
            .finish_non_exhaustive()
    }
}

impl History {
    pub fn load(
        repository: &Repository,
        cancel: &CancelToken,
        progress: &mut dyn FnMut(HistoryProgress),
    ) -> Result<Self> {
        let started = Instant::now();
        let repo = repository.local();
        let refs = repository.refs()?;
        let tips = tips_with_labels(&repo, &refs);
        let tip_ids: Vec<ObjectId> = tips.iter().map(|(id, _)| *id).collect();

        let index = CommitIndex::build(&repo, &tip_ids, cancel, &mut |commits| {
            progress(HistoryProgress {
                phase: HistoryPhase::Indexing,
                commits,
            })
        })?;
        let index_time = started.elapsed();
        progress(HistoryProgress {
            phase: HistoryPhase::Layout,
            commits: index.len() as u32,
        });

        let layout_started = Instant::now();
        let order = index.date_order(cancel)?;
        cancel.check()?;
        let layout = GraphLayout::compute(&index, &order);
        let mut row_of_node = vec![0u32; order.len()];
        for (row, &node) in order.iter().enumerate() {
            row_of_node[node as usize] = row as u32;
        }
        // Tips are interned first, so tip `i` is node `i`.
        let labels = tips
            .into_iter()
            .enumerate()
            .map(|(node, (_, labels))| (node as u32, labels))
            .collect();

        let stats = HistoryStats {
            commits: index.len() as u32,
            index: index.stats,
            index_time,
            layout_time: layout_started.elapsed(),
        };
        Ok(Self {
            index,
            order,
            row_of_node,
            layout,
            labels,
            stats,
            repo: Mutex::new(repo),
            metadata: Mutex::new(MetadataCache::default()),
            nodes_by_id: OnceLock::new(),
        })
    }

    pub fn len(&self) -> u32 {
        self.order.len() as u32
    }

    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    pub fn stats(&self) -> HistoryStats {
        self.stats
    }

    /// The widest the graph gets anywhere, in lanes.
    pub fn max_lanes(&self) -> u16 {
        self.layout.max_lanes
    }

    pub fn row_of(&self, id: &gix::oid) -> Option<u32> {
        let ids = &self.index.ids;
        let nodes_by_id = self.nodes_by_id.get_or_init(|| {
            let mut nodes: Vec<u32> = (0..ids.len() as u32).collect();
            nodes.sort_unstable_by(|a, b| ids[*a as usize].cmp(&ids[*b as usize]));
            nodes
        });
        let found = nodes_by_id
            .binary_search_by(|node| ids[*node as usize].as_ref().cmp(id))
            .ok()?;
        Some(self.row_of_node[nodes_by_id[found] as usize])
    }

    pub fn id_at(&self, row: u32) -> Option<ObjectId> {
        let node = *self.order.get(row as usize)?;
        Some(self.index.ids[node as usize])
    }

    /// Rows `start..start + count`, clamped to the history's length.
    pub fn rows(&self, start: u32, count: u32) -> Result<Vec<HistoryRow>> {
        let start = (start as usize).min(self.order.len());
        let end = start.saturating_add(count as usize).min(self.order.len());
        let graph_rows = self.layout.rows(&self.index, &self.order, start, end);
        let nodes = &self.order[start..end];
        let metadata = self.metadata(nodes)?;
        Ok(nodes
            .iter()
            .zip(graph_rows)
            .zip(metadata)
            .map(|((&node, graph), meta)| HistoryRow {
                id: self.index.ids[node as usize],
                summary: meta.summary.clone(),
                author_name: meta.author_name.clone(),
                author_email: meta.author_email.clone(),
                author_time: meta.author_time,
                commit_time: self.index.times[node as usize],
                refs: self.labels.get(&node).cloned().unwrap_or_default(),
                graph,
            })
            .collect())
    }

    fn metadata(&self, nodes: &[u32]) -> Result<Vec<Arc<CommitMeta>>> {
        let mut cache = self.metadata.lock().expect("metadata cache poisoned");
        let missing: Vec<u32> = nodes
            .iter()
            .copied()
            .filter(|node| !cache.entries.contains_key(node))
            .collect();
        if !missing.is_empty() {
            let repo = self.repo.lock().expect("repository poisoned");
            for node in missing {
                let meta = decode_meta(&repo, &self.index.ids[node as usize]);
                cache.insert(node, Arc::new(meta));
            }
        }
        Ok(nodes
            .iter()
            .map(|node| cache.entries[node].clone())
            .collect())
    }
}

fn decode_meta(repo: &gix::Repository, id: &gix::oid) -> CommitMeta {
    let decoded = repo.find_commit(id).ok().and_then(|commit| {
        let summary = commit
            .message()
            .ok()
            .map(|message| message.summary().to_str_lossy().into_owned())
            .unwrap_or_default();
        let author = commit.author().ok()?;
        Some(CommitMeta {
            summary,
            author_name: author.name.to_str_lossy().into_owned(),
            author_email: author.email.to_str_lossy().into_owned(),
            author_time: author.seconds(),
        })
    });
    decoded.unwrap_or_else(|| CommitMeta {
        summary: String::new(),
        author_name: String::new(),
        author_email: String::new(),
        author_time: 0,
    })
}

#[derive(Default)]
struct MetadataCache {
    entries: HashMap<u32, Arc<CommitMeta>>,
    insertion_order: VecDeque<u32>,
}

impl MetadataCache {
    fn insert(&mut self, node: u32, meta: Arc<CommitMeta>) {
        if self.entries.len() >= METADATA_CACHE_CAPACITY
            && let Some(oldest) = self.insertion_order.pop_front()
        {
            self.entries.remove(&oldest);
        }
        self.entries.insert(node, meta);
        self.insertion_order.push_back(node);
    }
}

/// The commits to start walking from, the checked-out one first, with the ref labels to show on
/// each.
fn tips_with_labels(repo: &gix::Repository, refs: &Refs) -> Vec<(ObjectId, Vec<RefLabel>)> {
    let mut tips: Vec<(ObjectId, Vec<RefLabel>)> = Vec::new();
    let mut add = |id: ObjectId, label: RefLabel| {
        if let Some((_, labels)) = tips.iter_mut().find(|(tip, _)| *tip == id) {
            // A remote branch joins the label of the local branch of the same name.
            if let Some(same) = labels.iter_mut().find(|existing| {
                existing.kind == RefLabelKind::Branch
                    && label.kind == RefLabelKind::Branch
                    && existing.name == label.name
            }) {
                same.local |= label.local;
                same.remotes.extend(label.remotes);
            } else {
                labels.push(label);
            }
        } else if repo
            .find_header(id)
            .is_ok_and(|header| header.kind() == gix::object::Kind::Commit)
        {
            tips.push((id, vec![label]));
        }
    };
    let label = |kind, name: &str, is_head| RefLabel {
        kind,
        name: name.to_owned(),
        local: false,
        remotes: Vec::new(),
        is_head,
    };

    if let Some(Head::Detached { target }) = &refs.head {
        add(*target, label(RefLabelKind::Head, "HEAD", true));
    }
    // The checked-out branch first among its commit's labels.
    let mut local_branches: Vec<_> = refs.local_branches.iter().collect();
    local_branches.sort_by_key(|branch| !branch.is_head);
    for branch in local_branches {
        add(
            branch.target,
            RefLabel {
                local: true,
                ..label(RefLabelKind::Branch, &branch.name, branch.is_head)
            },
        );
    }
    for branch in &refs.remote_branches {
        let name = branch
            .name
            .strip_prefix(&format!("{}/", branch.remote))
            .unwrap_or(&branch.name);
        add(
            branch.target,
            RefLabel {
                remotes: vec![branch.remote.clone()],
                ..label(RefLabelKind::Branch, name, false)
            },
        );
    }
    for tag in &refs.tags {
        add(tag.target, label(RefLabelKind::Tag, &tag.name, false));
    }
    // The checked-out commit first, so its line takes the leftmost lane.
    if let Some(head) = refs.head.as_ref().and_then(Head::target)
        && let Some(position) = tips.iter().position(|(id, _)| *id == head)
    {
        tips[..=position].rotate_right(1);
    }
    tips
}

/// Renders rows as text, one line per row: lanes (`*` commit, `|` line passing by), the summary,
/// ref labels, and the lines that change lane in that row: `in 2>0` is a line from lane 2 joining
/// the commit in lane 0, `out 0>1` a line from the commit to a parent in lane 1. Used by tests and
/// the `log` example.
#[doc(hidden)]
pub fn render_ascii(rows: &[HistoryRow]) -> String {
    let mut out = String::new();
    for row in rows {
        let graph = &row.graph;
        let mut cells = vec![' '; graph.lane_count as usize];
        for edge in &graph.edges {
            if edge.kind == EdgeKind::Through {
                cells[edge.from_lane as usize] = '|';
            }
        }
        cells[graph.node_lane as usize] = '*';
        let lanes: String = cells.iter().flat_map(|c| [*c, ' ']).collect();
        let bends: Vec<String> = graph
            .edges
            .iter()
            .filter(|edge| edge.from_lane != edge.to_lane)
            .map(|edge| match edge.kind {
                EdgeKind::IntoNode => format!("in {}>{}", edge.from_lane, edge.to_lane),
                _ => format!("out {}>{}", edge.from_lane, edge.to_lane),
            })
            .collect();
        let refs: Vec<String> = row.refs.iter().map(RefLabel::display_name).collect();
        out.push_str(lanes.trim_end());
        out.push_str(&" ".repeat((12usize).saturating_sub(lanes.trim_end().chars().count())));
        out.push_str(&row.summary);
        if !refs.is_empty() {
            out.push_str(&format!(" ({})", refs.join(", ")));
        }
        if !bends.is_empty() {
            out.push_str(&format!("  [{}]", bends.join(", ")));
        }
        out.push('\n');
    }
    out
}
