use petgraph::graph::NodeIndex;

use crate::graph::{Coord, RoadGraph};

/// 无向连通分量大小，按从大到小排序。
pub fn component_sizes(g: &RoadGraph) -> Vec<usize> {
    let n = g.graph.node_count();
    if n == 0 {
        return Vec::new();
    }
    let mut seen = vec![false; n];
    let mut sizes = Vec::new();
    let mut stack: Vec<NodeIndex> = Vec::new();
    for start in g.graph.node_indices() {
        if seen[start.index()] {
            continue;
        }
        seen[start.index()] = true;
        stack.push(start);
        let mut size = 0usize;
        while let Some(cur) = stack.pop() {
            size += 1;
            for next in g.graph.neighbors_undirected(cur) {
                if !seen[next.index()] {
                    seen[next.index()] = true;
                    stack.push(next);
                }
            }
        }
        sizes.push(size);
    }
    sizes.sort_unstable_by(|a, b| b.cmp(a));
    sizes
}

/// 返回节点所在连通分量编号和该分量大小。
pub fn component_info(g: &RoadGraph, target: NodeIndex) -> Option<(usize, usize)> {
    let n = g.graph.node_count();
    if target.index() >= n {
        return None;
    }
    let mut seen = vec![false; n];
    let mut stack: Vec<NodeIndex> = Vec::new();
    let mut component_id = 0usize;
    for start in g.graph.node_indices() {
        if seen[start.index()] {
            continue;
        }
        seen[start.index()] = true;
        stack.push(start);
        let mut size = 0usize;
        let mut hit = false;
        while let Some(cur) = stack.pop() {
            size += 1;
            if cur == target {
                hit = true;
            }
            for next in g.graph.neighbors_undirected(cur) {
                if !seen[next.index()] {
                    seen[next.index()] = true;
                    stack.push(next);
                }
            }
        }
        if hit {
            return Some((component_id, size));
        }
        component_id += 1;
    }
    None
}

/// 给定坐标，返回最近道路边的两个端点、平面距离、垂足坐标。
pub fn nearest_edge_nodes(
    g: &RoadGraph,
    coord: Coord,
) -> Option<(NodeIndex, NodeIndex, f64, Coord)> {
    let (edge, _t, proj, distance_m) = g.nearest_edge(g.to_m(coord))?;
    let (a, b) = g.graph.edge_endpoints(edge)?;
    Some((a, b, distance_m, g.to_deg(proj)))
}
