//! Graph algorithms over item links.

use std::collections::BTreeMap;

/// The cycles in a directed graph: each strongly connected component of two or more nodes, or a single node with an
/// edge to itself. Only nodes on a cycle are named, never nodes that merely reach or follow one. Each cycle is
/// sorted, and so is the list.
#[must_use]
pub fn cycles<N: Ord + Clone>(edges: &[(N, N)]) -> Vec<Vec<N>> {
    let mut ids: BTreeMap<&N, usize> = BTreeMap::new();
    let mut nodes: Vec<&N> = Vec::new();
    for (from, to) in edges {
        for node in [from, to] {
            ids.entry(node).or_insert_with(|| {
                nodes.push(node);
                nodes.len() - 1
            });
        }
    }
    let mut adjacent = vec![Vec::new(); ids.len()];
    let mut self_loops = vec![false; ids.len()];
    for (from, to) in edges {
        let (from, to) = (ids[from], ids[to]);
        adjacent[from].push(to);
        self_loops[from] |= from == to;
    }

    let mut found: Vec<Vec<N>> = tarjan(&adjacent)
        .into_iter()
        .filter(|component| component.len() > 1 || self_loops[component[0]])
        .map(|component| {
            let mut cycle: Vec<N> = component.into_iter().map(|id| nodes[id].clone()).collect();
            cycle.sort();
            cycle
        })
        .collect();
    found.sort();
    found
}

/// Tarjan's strongly connected components, iterative so deep graphs can't overflow the stack.
fn tarjan(adjacent: &[Vec<usize>]) -> Vec<Vec<usize>> {
    const UNVISITED: usize = usize::MAX;
    let n = adjacent.len();
    let mut index = vec![UNVISITED; n];
    let mut low = vec![0; n];
    let mut on_stack = vec![false; n];
    let mut stack = Vec::new();
    let mut components = Vec::new();
    let mut counter = 0;

    for root in 0..n {
        if index[root] != UNVISITED {
            continue;
        }
        // Each frame is a node and the position of the next neighbour to visit.
        let mut frames = vec![(root, 0)];
        index[root] = counter;
        low[root] = counter;
        counter += 1;
        stack.push(root);
        on_stack[root] = true;
        while let Some(&mut (node, ref mut next)) = frames.last_mut() {
            if let Some(&to) = adjacent[node].get(*next) {
                *next += 1;
                if index[to] == UNVISITED {
                    index[to] = counter;
                    low[to] = counter;
                    counter += 1;
                    stack.push(to);
                    on_stack[to] = true;
                    frames.push((to, 0));
                } else if on_stack[to] {
                    low[node] = low[node].min(index[to]);
                }
                continue;
            }
            frames.pop();
            if let Some(&(parent, _)) = frames.last() {
                low[parent] = low[parent].min(low[node]);
            }
            if low[node] == index[node] {
                let mut component = Vec::new();
                while let Some(member) = stack.pop() {
                    on_stack[member] = false;
                    component.push(member);
                    if member == node {
                        break;
                    }
                }
                components.push(component);
            }
        }
    }
    components
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_only_the_nodes_on_a_cycle() {
        assert_eq!(cycles(&[("a", "b"), ("b", "a"), ("a", "c"), ("d", "a")]), vec![vec!["a", "b"]]);
    }

    #[test]
    fn reports_each_cycle_separately() {
        let edges =
            [("a", "b"), ("b", "a"), ("x", "y"), ("y", "z"), ("z", "x"), ("b", "x"), ("s", "s")];
        assert_eq!(cycles(&edges), vec![vec!["a", "b"], vec!["s"], vec!["x", "y", "z"]]);
    }

    #[test]
    fn acyclic_graphs_have_none() {
        assert!(cycles::<&str>(&[]).is_empty());
        assert!(cycles(&[("a", "b"), ("b", "c"), ("a", "c")]).is_empty());
    }

    #[test]
    fn a_long_chain_does_not_overflow() {
        let edges: Vec<(u32, u32)> =
            (0..100_000).map(|i| (i, i + 1)).chain([(100_000, 0)]).collect();
        assert_eq!(cycles(&edges)[0].len(), 100_001);
    }
}
