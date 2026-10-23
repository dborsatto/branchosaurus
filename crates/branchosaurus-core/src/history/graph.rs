// SPDX-License-Identifier: GPL-3.0-or-later

//! Lane layout for the commit graph.
//!
//! Rows are laid out top to bottom. Each lane waits for one commit; a commit takes the first lane
//! waiting for it, sends its first parent straight down that lane, and opens or joins lanes for
//! its other parents. The layout of a row depends only on the rows above it, so instead of
//! storing every row (wide histories pass dozens of lines through each row) we store the lane
//! state every [`CHECKPOINT_INTERVAL`] rows and replay from the nearest checkpoint on demand.

use super::index::CommitIndex;

pub(crate) const CHECKPOINT_INTERVAL: usize = 1024;

/// What the shell needs to draw the graph cell of one row.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GraphRow {
    /// Lane the commit's dot is drawn in.
    pub node_lane: u16,
    pub node_color: u16,
    /// Number of lanes the row spans; the cell is at least this many lanes wide.
    pub lane_count: u16,
    pub edges: Vec<GraphEdge>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GraphEdge {
    pub from_lane: u16,
    pub to_lane: u16,
    /// Index into the shell's palette (wraps around).
    pub color: u16,
    pub kind: EdgeKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EdgeKind {
    /// From the top of the row in `from_lane` down to the bottom in `to_lane` (always equal),
    /// passing by the commit.
    Through,
    /// From the top of the row in `from_lane` into the commit's dot in `to_lane`.
    IntoNode,
    /// From the commit's dot in `from_lane` to the bottom of the row in `to_lane`, towards a parent.
    OutOfNode,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Lane {
    /// Node this lane is waiting for.
    expects: u32,
    color: u16,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct LaneState {
    lanes: Vec<Option<Lane>>,
    next_color: u16,
}

impl LaneState {
    /// Advances past the row for `node`, optionally describing it in `out`. Returns the row's
    /// width in lanes.
    pub fn step(&mut self, node: u32, parents: &[u32], mut out: Option<&mut GraphRow>) -> u16 {
        let lanes_above = self.lanes.len();
        let existing_lane = self
            .lanes
            .iter()
            .position(|lane| lane.is_some_and(|lane| lane.expects == node));
        let node_lane = match existing_lane {
            Some(lane) => lane,
            // A branch tip: nothing above is waiting for it.
            None => {
                let lane = self.free_lane(0);
                let color = self.new_color();
                self.lanes[lane] = Some(Lane {
                    expects: node,
                    color,
                });
                lane
            }
        };
        let node_color = self.lanes[node_lane].expect("node lane is occupied").color;

        for (index, lane) in self.lanes.iter_mut().enumerate() {
            let Some(current) = *lane else { continue };
            if current.expects == node {
                if existing_lane.is_some() {
                    push_edge(
                        &mut out,
                        index,
                        node_lane,
                        current.color,
                        EdgeKind::IntoNode,
                    );
                }
                if index != node_lane {
                    // Several lanes converge on this commit; only the node lane continues.
                    *lane = None;
                }
            } else {
                push_edge(&mut out, index, index, current.color, EdgeKind::Through);
            }
        }

        match parents.split_first() {
            None => self.lanes[node_lane] = None,
            Some((&first, rest)) => {
                self.lanes[node_lane] = Some(Lane {
                    expects: first,
                    color: node_color,
                });
                push_edge(
                    &mut out,
                    node_lane,
                    node_lane,
                    node_color,
                    EdgeKind::OutOfNode,
                );
                for &parent in rest {
                    let joined = self
                        .lanes
                        .iter()
                        .position(|lane| lane.is_some_and(|lane| lane.expects == parent));
                    let (lane, color) = match joined {
                        Some(lane) => (lane, self.lanes[lane].expect("occupied").color),
                        None => {
                            let lane = self.free_lane(node_lane + 1);
                            let color = self.new_color();
                            self.lanes[lane] = Some(Lane {
                                expects: parent,
                                color,
                            });
                            (lane, color)
                        }
                    };
                    push_edge(&mut out, node_lane, lane, color, EdgeKind::OutOfNode);
                }
            }
        }

        while self.lanes.last().is_some_and(Option::is_none) {
            self.lanes.pop();
        }
        let lane_count = lanes_above.max(self.lanes.len()).max(node_lane + 1) as u16;
        if let Some(row) = out {
            row.node_lane = node_lane as u16;
            row.node_color = node_color;
            row.lane_count = lane_count;
        }
        lane_count
    }

    fn free_lane(&mut self, from: usize) -> usize {
        match self.lanes.iter().skip(from).position(Option::is_none) {
            Some(offset) => from + offset,
            None => {
                self.lanes.resize(self.lanes.len().max(from), None);
                self.lanes.push(None);
                self.lanes.len() - 1
            }
        }
    }

    fn new_color(&mut self) -> u16 {
        let color = self.next_color;
        self.next_color = self.next_color.wrapping_add(1);
        color
    }
}

fn push_edge(out: &mut Option<&mut GraphRow>, from: usize, to: usize, color: u16, kind: EdgeKind) {
    if let Some(row) = out {
        row.edges.push(GraphEdge {
            from_lane: from as u16,
            to_lane: to as u16,
            color,
            kind,
        });
    }
}

/// Lane-state checkpoints for a whole history.
#[derive(Debug, Default)]
pub(crate) struct GraphLayout {
    checkpoints: Vec<LaneState>,
    pub max_lanes: u16,
}

impl GraphLayout {
    pub fn compute(index: &CommitIndex, order: &[u32]) -> Self {
        let mut state = LaneState::default();
        let mut checkpoints = Vec::with_capacity(order.len() / CHECKPOINT_INTERVAL + 1);
        let mut max_lanes = 0;
        for (row, &node) in order.iter().enumerate() {
            if row.is_multiple_of(CHECKPOINT_INTERVAL) {
                checkpoints.push(state.clone());
            }
            max_lanes = max_lanes.max(state.step(node, index.parents(node), None));
        }
        Self {
            checkpoints,
            max_lanes,
        }
    }

    /// Graph rows for `order[start..end]`.
    pub fn rows(
        &self,
        index: &CommitIndex,
        order: &[u32],
        start: usize,
        end: usize,
    ) -> Vec<GraphRow> {
        let checkpoint = start / CHECKPOINT_INTERVAL;
        let Some(state) = self.checkpoints.get(checkpoint) else {
            return Vec::new();
        };
        let mut state = state.clone();
        for &node in &order[checkpoint * CHECKPOINT_INTERVAL..start] {
            state.step(node, index.parents(node), None);
        }
        order[start..end]
            .iter()
            .map(|&node| {
                let mut row = GraphRow::default();
                state.step(node, index.parents(node), Some(&mut row));
                row
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lays out rows given as `(node, parents)` in display order.
    fn layout(rows: &[(u32, &[u32])]) -> Vec<GraphRow> {
        let mut state = LaneState::default();
        rows.iter()
            .map(|(node, parents)| {
                let mut row = GraphRow::default();
                state.step(*node, parents, Some(&mut row));
                row
            })
            .collect()
    }

    #[test]
    fn linear_history_stays_in_one_lane() {
        let rows = layout(&[(0, &[1]), (1, &[2]), (2, &[])]);
        assert!(
            rows.iter()
                .all(|row| row.node_lane == 0 && row.lane_count == 1)
        );
        assert_eq!(
            rows[0].edges.len(),
            1,
            "tip has no line coming in from above"
        );
        assert_eq!(rows[2].edges.len(), 1, "root has no line going out below");
    }

    #[test]
    fn merge_opens_a_lane_that_closes_at_the_fork_point() {
        // 0 = merge(1, 2), 1 and 2 both have parent 3.
        let rows = layout(&[(0, &[1, 2]), (1, &[3]), (2, &[3]), (3, &[])]);
        assert_eq!(rows[0].node_lane, 0);
        assert!(rows[0].edges.contains(&GraphEdge {
            from_lane: 0,
            to_lane: 1,
            color: 1,
            kind: EdgeKind::OutOfNode
        }));
        assert_eq!(rows[2].node_lane, 1);
        assert_eq!(rows[3].node_lane, 0);
        let into_fork_point: Vec<_> = rows[3]
            .edges
            .iter()
            .filter(|edge| edge.kind == EdgeKind::IntoNode)
            .map(|edge| edge.from_lane)
            .collect();
        assert_eq!(into_fork_point, [0, 1]);
    }
}
