// SPDX-License-Identifier: Apache-2.0
//! Pearce-Kelly incremental DAG cycle detection.
//!
//! Reference: David J. Pearce, Paul H.J. Kelly, "A Dynamic Topological Sort
//! Algorithm for Directed Acyclic Graphs" (JEA 2007).

pub const PK_MAX_NODES: usize = 8;

/// Topological rank state for a bounded wait-for graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PearceKellyState {
    /// Topological position for each node. Lower values precede higher values.
    pub ord: [u32; PK_MAX_NODES],
    /// Number of currently valid nodes.
    pub n: usize,
}

impl PearceKellyState {
    /// Creates `n` nodes in their initial topological order.
    pub fn new(n: usize) -> Self {
        assert!(n <= PK_MAX_NODES, "too many PK nodes: {n}");

        let mut ord = [0; PK_MAX_NODES];
        for (idx, rank) in ord.iter_mut().enumerate().take(n) {
            *rank = idx as u32;
        }

        Self { ord, n }
    }

    /// Inserts edge `from -> to` into the graph represented by `adj`.
    ///
    /// Returns `true` if the edge would create a cycle and must not be added.
    /// Returns `false` if the edge is safe; ranks are refreshed when needed.
    pub fn insert_edge(
        &mut self,
        adj: &[[bool; PK_MAX_NODES]; PK_MAX_NODES],
        from: usize,
        to: usize,
    ) -> bool {
        assert!(from < self.n, "from node out of bounds: {from}");
        assert!(to < self.n, "to node out of bounds: {to}");

        if from == to || reaches(adj, self.n, to, from) {
            return true;
        }

        if self.ord[to] < self.ord[from] {
            self.reorder_after_safe_insert(adj, from, to);
        }

        false
    }

    fn reorder_after_safe_insert(
        &mut self,
        adj: &[[bool; PK_MAX_NODES]; PK_MAX_NODES],
        from: usize,
        to: usize,
    ) {
        let mut indegree = [0usize; PK_MAX_NODES];
        for src in 0..self.n {
            for (dst, degree) in indegree.iter_mut().enumerate().take(self.n) {
                if adj[src][dst] || (src == from && dst == to) {
                    *degree += 1;
                }
            }
        }

        let mut used = [false; PK_MAX_NODES];
        for rank in 0..self.n {
            let next = (0..self.n)
                .filter(|&node| !used[node] && indegree[node] == 0)
                .min_by_key(|&node| self.ord[node])
                .expect("safe edge insertion must preserve a DAG");

            used[next] = true;
            self.ord[next] = rank as u32;

            for (dst, degree) in indegree.iter_mut().enumerate().take(self.n) {
                if adj[next][dst] || (next == from && dst == to) {
                    *degree -= 1;
                }
            }
        }
    }
}

fn reaches(
    adj: &[[bool; PK_MAX_NODES]; PK_MAX_NODES],
    n: usize,
    start: usize,
    target: usize,
) -> bool {
    let mut visited = [false; PK_MAX_NODES];
    let mut stack = [0usize; PK_MAX_NODES];
    let mut len = 1;
    stack[0] = start;

    while len > 0 {
        len -= 1;
        let node = stack[len];
        if node == target {
            return true;
        }
        if visited[node] {
            continue;
        }
        visited[node] = true;

        for next in 0..n {
            if adj[node][next] && !visited[next] {
                stack[len] = next;
                len += 1;
            }
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_forward_edge_is_safe() {
        let adj = [[false; PK_MAX_NODES]; PK_MAX_NODES];
        let mut state = PearceKellyState::new(3);

        assert!(!state.insert_edge(&adj, 0, 1));
        assert!(state.ord[0] < state.ord[1]);
    }

    #[test]
    fn insert_back_edge_reorders_when_acyclic() {
        let adj = [[false; PK_MAX_NODES]; PK_MAX_NODES];
        let mut state = PearceKellyState::new(3);

        assert!(!state.insert_edge(&adj, 2, 0));
        assert!(state.ord[2] < state.ord[0]);
    }

    #[test]
    fn insert_edge_detects_cycle() {
        let mut adj = [[false; PK_MAX_NODES]; PK_MAX_NODES];
        adj[0][1] = true;
        adj[1][2] = true;
        let mut state = PearceKellyState::new(3);

        assert!(state.insert_edge(&adj, 2, 0));
    }
}
