//! Inclusive upstream traversal over the HFX drainage graph.

use std::collections::{HashSet, VecDeque};

use hfx_core::{DrainageGraph, UnitId};
use tracing::{debug, instrument};

// ── TraversalError ────────────────────────────────────────────────────────────

/// Errors from upstream graph traversal.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TraversalError {
    /// The terminal unit ID does not exist in the drainage graph.
    ///
    /// Fired when `DrainageGraph::get(terminal)` returns `None` at the
    /// start of traversal.
    #[error("terminal unit {unit_id} not found in drainage graph")]
    TerminalNotFound {
        /// The raw i64 value of the missing unit ID.
        unit_id: i64,
    },

    /// An unit references an upstream neighbour that is absent from the graph.
    ///
    /// Fired when an `upstream_ids` entry points to an unit ID that has no
    /// row in the [`DrainageGraph`]. This indicates a referential integrity
    /// violation in the graph data.
    #[error("unit {source_id} references upstream unit {target_id} which is absent from the graph")]
    DanglingUpstreamRef {
        /// The raw i64 value of the unit that contains the dangling reference.
        source_id: i64,
        /// The raw i64 value of the missing upstream unit.
        target_id: i64,
    },
}

// ── UpstreamUnits ─────────────────────────────────────────────────────────────

/// The set of unit IDs reachable upstream from a terminal unit, inclusive.
///
/// Produced by [`collect_upstream`]. Contains the terminal unit itself plus
/// every unit reachable via BFS over upstream adjacency edges.
///
/// # Ordering
///
/// The terminal unit is always the first element of the backing slice
/// (accessible via [`terminal()`](Self::terminal)). Beyond that, the
/// iteration order is deterministic but **not a stable API contract** —
/// callers must rely only on [`terminal()`](Self::terminal) and
/// membership semantics ([`contains`](Self::contains), [`len`](Self::len)),
/// not on the position of non-terminal units.
#[derive(Debug, Clone)]
pub struct UpstreamUnits {
    units: Vec<UnitId>,
    index: HashSet<UnitId>,
}

impl UpstreamUnits {
    /// Return the terminal unit ID — always the first element.
    pub fn terminal(&self) -> UnitId {
        self.units[0]
    }

    /// Return the unit IDs as a slice, terminal first.
    pub fn unit_ids(&self) -> &[UnitId] {
        &self.units
    }

    /// Return the number of unit IDs in this set.
    pub fn len(&self) -> usize {
        self.units.len()
    }

    /// Return `true` if this set contains no unit IDs.
    pub fn is_empty(&self) -> bool {
        self.units.is_empty()
    }

    /// Return `true` if `id` is part of this upstream set (O(1)).
    pub fn contains(&self, id: &UnitId) -> bool {
        self.index.contains(id)
    }

    /// Iterate over unit IDs, terminal first.
    pub fn iter(&self) -> std::slice::Iter<'_, UnitId> {
        self.units.iter()
    }

    /// Consume this set and return the underlying `Vec<UnitId>`.
    pub fn into_unit_ids(self) -> Vec<UnitId> {
        self.units
    }
}

impl PartialEq for UpstreamUnits {
    fn eq(&self, other: &Self) -> bool {
        self.units.first() == other.units.first() && self.index == other.index
    }
}

impl Eq for UpstreamUnits {}

impl IntoIterator for UpstreamUnits {
    type Item = UnitId;
    type IntoIter = std::vec::IntoIter<UnitId>;

    fn into_iter(self) -> Self::IntoIter {
        self.units.into_iter()
    }
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Collect all units upstream of a terminal unit via breadth-first traversal.
///
/// Performs an inclusive BFS over the [`DrainageGraph`], starting from
/// `terminal`. The terminal itself is included in the result. Every upstream
/// path is followed regardless of mainstem status (inclusive mode per HFX
/// v0.1 engine behaviour contract).
///
/// A visited set is maintained unconditionally, as required by the HFX spec
/// for both tree and DAG topology datasets. For tree datasets this is a
/// no-op; for DAG datasets it prevents shared upstream nodes from being
/// visited more than once.
///
/// # Errors
///
/// | Condition | Error |
/// |-----------|-------|
/// | `terminal` not in graph | [`TraversalError::TerminalNotFound`] |
/// | An upstream reference points to a missing unit | [`TraversalError::DanglingUpstreamRef`] |
#[instrument(skip(graph), fields(terminal = terminal.get()))]
pub fn collect_upstream(
    terminal: UnitId,
    graph: &DrainageGraph,
) -> Result<UpstreamUnits, TraversalError> {
    if graph.get(terminal).is_none() {
        return Err(TraversalError::TerminalNotFound {
            unit_id: terminal.get(),
        });
    }

    let mut visited: HashSet<UnitId> = HashSet::new();
    let mut units: Vec<UnitId> = Vec::new();
    let mut queue: VecDeque<UnitId> = VecDeque::new();

    visited.insert(terminal);
    queue.push_back(terminal);

    while let Some(current) = queue.pop_front() {
        units.push(current);

        if let Some(row) = graph.get(current) {
            for &upstream_id in row.upstream_ids() {
                if visited.contains(&upstream_id) {
                    continue;
                }

                if graph.get(upstream_id).is_none() {
                    return Err(TraversalError::DanglingUpstreamRef {
                        source_id: current.get(),
                        target_id: upstream_id.get(),
                    });
                }

                visited.insert(upstream_id);
                queue.push_back(upstream_id);
            }
        }
    }

    debug!(unit_count = units.len(), "upstream traversal complete");

    Ok(UpstreamUnits {
        units,
        index: visited,
    })
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use hfx_core::{AdjacencyRow, Level};

    use super::*;

    fn aid(raw: i64) -> UnitId {
        UnitId::new(raw).unwrap()
    }

    fn level0() -> Level {
        Level::new(0).unwrap()
    }

    fn graph(specs: &[(i64, &[i64])]) -> DrainageGraph {
        let rows = specs
            .iter()
            .map(|&(id, ups)| {
                let upstream_ids = ups.iter().map(|&r| aid(r)).collect();
                AdjacencyRow::new(aid(id), level0(), upstream_ids)
            })
            .collect();
        DrainageGraph::new(rows).unwrap()
    }

    fn id_set(result: &UpstreamUnits) -> HashSet<i64> {
        result.unit_ids().iter().map(|a| a.get()).collect()
    }

    // ── Group A: Topology traversal ───────────────────────────────────────────

    #[test]
    fn single_headwater() {
        let g = graph(&[(1, &[])]);
        let result = collect_upstream(aid(1), &g).unwrap();
        assert_eq!(result.len(), 1);
        assert!(result.contains(&aid(1)));
        assert_eq!(result.terminal(), aid(1));
    }

    #[test]
    fn linear_chain() {
        let g = graph(&[(1, &[]), (2, &[1]), (3, &[2])]);
        let result = collect_upstream(aid(3), &g).unwrap();
        assert_eq!(result.len(), 3);
        assert_eq!(id_set(&result), HashSet::from([1, 2, 3]));
        assert_eq!(result.terminal(), aid(3));
    }

    #[test]
    fn y_confluence() {
        let g = graph(&[(1, &[]), (2, &[]), (3, &[1, 2])]);
        let result = collect_upstream(aid(3), &g).unwrap();
        assert_eq!(result.len(), 3);
        assert_eq!(id_set(&result), HashSet::from([1, 2, 3]));
    }

    #[test]
    fn diamond_dag_no_duplicates() {
        let g = graph(&[(1, &[]), (2, &[1]), (3, &[1]), (4, &[2, 3])]);
        let result = collect_upstream(aid(4), &g).unwrap();
        assert_eq!(result.len(), 4, "shared node 1 must not appear twice");
        assert_eq!(id_set(&result), HashSet::from([1, 2, 3, 4]));
    }

    #[test]
    fn deep_chain_100() {
        let specs: Vec<(i64, Vec<i64>)> = (1i64..=100)
            .map(|id| {
                if id == 1 {
                    (id, vec![])
                } else {
                    (id, vec![id - 1])
                }
            })
            .collect();
        let spec_refs: Vec<(i64, &[i64])> = specs
            .iter()
            .map(|(id, ups)| (*id, ups.as_slice()))
            .collect();
        let g = graph(&spec_refs);
        let result = collect_upstream(aid(100), &g).unwrap();
        assert_eq!(result.len(), 100);
        assert_eq!(result.terminal(), aid(100));
    }

    #[test]
    fn wide_fan_in() {
        let mut specs: Vec<(i64, Vec<i64>)> = (1i64..=50).map(|id| (id, vec![])).collect();
        let headwater_ids: Vec<i64> = (1..=50).collect();
        specs.push((51, headwater_ids));
        let spec_refs: Vec<(i64, &[i64])> = specs
            .iter()
            .map(|(id, ups)| (*id, ups.as_slice()))
            .collect();
        let g = graph(&spec_refs);
        let result = collect_upstream(aid(51), &g).unwrap();
        assert_eq!(result.len(), 51);
        assert_eq!(result.terminal(), aid(51));
    }

    #[test]
    fn multi_level_tree() {
        let g = graph(&[
            (1, &[]),
            (2, &[]),
            (3, &[1, 2]),
            (4, &[]),
            (5, &[3, 4]),
            (6, &[5]),
        ]);
        let result = collect_upstream(aid(6), &g).unwrap();
        assert_eq!(result.len(), 6);
        assert_eq!(id_set(&result), HashSet::from([1, 2, 3, 4, 5, 6]));
    }

    // ── Group B: Terminal-first guarantee ─────────────────────────────────────

    #[test]
    fn terminal_is_first_headwater() {
        let g = graph(&[(1, &[])]);
        let result = collect_upstream(aid(1), &g).unwrap();
        assert_eq!(result.unit_ids()[0], aid(1));
    }

    #[test]
    fn terminal_is_first_chain() {
        let g = graph(&[(1, &[]), (2, &[1]), (3, &[2])]);
        let result = collect_upstream(aid(3), &g).unwrap();
        assert_eq!(result.unit_ids()[0], aid(3));
    }

    #[test]
    fn terminal_is_first_dag() {
        let g = graph(&[(1, &[]), (2, &[1]), (3, &[1]), (4, &[2, 3])]);
        let result = collect_upstream(aid(4), &g).unwrap();
        assert_eq!(result.unit_ids()[0], aid(4));
    }

    #[test]
    fn bfs_visits_shallower_levels_before_deeper_levels() {
        let g = graph(&[(1, &[]), (2, &[1]), (3, &[1]), (4, &[2, 3])]);
        let result = collect_upstream(aid(4), &g).unwrap();
        let ids = result.unit_ids();

        let pos = |id: i64| ids.iter().position(|&unit_id| unit_id == aid(id)).unwrap();

        // BFS level order is the contract; sibling order within a level is not.
        assert_eq!(ids[0], aid(4));
        assert!(
            pos(2) < pos(1),
            "depth-1 unit 2 must appear before depth-2 unit 1"
        );
        assert!(
            pos(3) < pos(1),
            "depth-1 unit 3 must appear before depth-2 unit 1"
        );
    }

    // ── Group C: Error paths ──────────────────────────────────────────────────

    #[test]
    fn terminal_not_found() {
        let g = graph(&[(1, &[])]);
        let err = collect_upstream(aid(999), &g).unwrap_err();
        assert!(matches!(
            err,
            TraversalError::TerminalNotFound { unit_id: 999 }
        ));
    }

    #[test]
    fn error_display_contains_unit_id() {
        let g = graph(&[(1, &[])]);
        let err = collect_upstream(aid(999), &g).unwrap_err();
        assert!(err.to_string().contains("999"));
    }

    // ── Group D: Dangling upstream refs (hard error) ──────────────────────────

    #[test]
    fn dangling_upstream_ref_at_terminal() {
        let g = graph(&[(1, &[99])]);
        let err = collect_upstream(aid(1), &g).unwrap_err();
        assert!(matches!(
            err,
            TraversalError::DanglingUpstreamRef {
                source_id: 1,
                target_id: 99
            }
        ));
    }

    #[test]
    fn dangling_upstream_ref_deep() {
        let g = graph(&[(1, &[99]), (2, &[1]), (3, &[2])]);
        let err = collect_upstream(aid(3), &g).unwrap_err();
        assert!(matches!(
            err,
            TraversalError::DanglingUpstreamRef {
                source_id: 1,
                target_id: 99
            }
        ));
    }

    #[test]
    fn dangling_ref_display_contains_ids() {
        let g = graph(&[(1, &[99]), (2, &[1]), (3, &[2])]);
        let err = collect_upstream(aid(3), &g).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("1"), "message should contain source id: {msg}");
        assert!(
            msg.contains("99"),
            "message should contain target id: {msg}"
        );
    }

    // ── Group E: Edge behavior ────────────────────────────────────────────────

    #[test]
    fn traversal_from_interior_node() {
        // 5-node chain: 1 ← 2 ← 3 ← 4 ← 5, start at 3
        let g = graph(&[(1, &[]), (2, &[1]), (3, &[2]), (4, &[3]), (5, &[4])]);
        let result = collect_upstream(aid(3), &g).unwrap();
        assert_eq!(result.len(), 3);
        assert_eq!(id_set(&result), HashSet::from([1, 2, 3]));
        assert!(!result.contains(&aid(4)));
        assert!(!result.contains(&aid(5)));
    }

    #[test]
    fn disconnected_headwaters() {
        let g = graph(&[(1, &[]), (2, &[]), (3, &[])]);
        let result = collect_upstream(aid(2), &g).unwrap();
        assert_eq!(result.len(), 1);
        assert!(result.contains(&aid(2)));
    }

    #[test]
    fn large_unit_ids() {
        let g = graph(&[(i64::MAX - 1, &[]), (i64::MAX, &[i64::MAX - 1])]);
        let result = collect_upstream(aid(i64::MAX), &g).unwrap();
        assert_eq!(result.len(), 2);
        assert!(result.contains(&aid(i64::MAX)));
        assert!(result.contains(&aid(i64::MAX - 1)));
    }

    // ── Group F: UpstreamUnits API ────────────────────────────────────────────

    #[test]
    fn contains_false_for_absent_unit() {
        let g = graph(&[(1, &[]), (2, &[1]), (3, &[2])]);
        let result = collect_upstream(aid(3), &g).unwrap();
        assert!(!result.contains(&aid(999)));
    }

    #[test]
    fn into_unit_ids_consumes() {
        let g = graph(&[(1, &[]), (2, &[1])]);
        let result = collect_upstream(aid(2), &g).unwrap();
        let vec = result.into_unit_ids();
        assert_eq!(vec.len(), 2);
    }

    #[test]
    fn iter_count_matches_len() {
        let g = graph(&[(1, &[]), (2, &[]), (3, &[1, 2])]);
        let result = collect_upstream(aid(3), &g).unwrap();
        assert_eq!(result.iter().count(), result.len());
    }

    #[test]
    fn is_empty_always_false() {
        let g = graph(&[(1, &[])]);
        let result = collect_upstream(aid(1), &g).unwrap();
        assert!(!result.is_empty());
    }

    #[test]
    fn into_iterator_yields_all() {
        let g = graph(&[(1, &[]), (2, &[1]), (3, &[2])]);
        let result = collect_upstream(aid(3), &g).unwrap();
        assert_eq!(result.into_iter().count(), 3);
    }

    #[test]
    fn partial_eq_is_set_based() {
        // Two UpstreamUnits with the same terminal and same set are equal
        // regardless of internal Vec ordering.
        let units_a = vec![aid(3), aid(1), aid(2)];
        let units_b = vec![aid(3), aid(2), aid(1)];
        let index: HashSet<UnitId> = [aid(1), aid(2), aid(3)].into_iter().collect();
        let a = UpstreamUnits {
            units: units_a,
            index: index.clone(),
        };
        let b = UpstreamUnits {
            units: units_b,
            index,
        };
        assert_eq!(
            a, b,
            "same set with different non-terminal order must be equal"
        );
    }
}
