//! `affected`: which modified files are part of an import cycle.
//!
//! Export-level tracking (`turborepo_trace_lang_js::symbols`) compares what a
//! modified module does when it is loaded. For a module outside every import
//! cycle that is the code of its load-time statements: everything it imports
//! has run before it. A module on a cycle can be loaded while a module it
//! imports has not run yet, and then a statement that only names an import
//! (`export const routes = [Home]`, `memo(Component)`) finds a binding that is
//! not initialized: under ES module semantics the read throws, under a CommonJS
//! transpilation it yields `undefined`. So for those modules such statements
//! are compared as load-time code.

use turborepo_trace_graph::{GraphView, NodeId, RefFlags, RefKind};

/// Edges that load a module when the importing module is loaded.
fn loads(kind: RefKind, flags: RefFlags) -> bool {
    matches!(kind, RefKind::Import | RefKind::Require | RefKind::ReExport)
        && !flags.contains(RefFlags::TYPE_ONLY)
}

/// For each of `files`: whether the node is on a cycle of static, non-type-only
/// module loads (an iterative Tarjan over what the files reach).
pub fn on_cycle<G: GraphView>(g: &G, files: &[NodeId]) -> Vec<bool> {
    const UNSEEN: u32 = u32::MAX;
    let n = g.node_count() as usize;
    let mut index = vec![UNSEEN; n];
    let mut low = vec![0u32; n];
    let mut on_stack = vec![false; n];
    let mut cyclic = vec![false; n];
    let mut stack: Vec<u32> = Vec::new();
    let mut next = 0u32;
    let targets = |v: u32| -> Vec<u32> {
        g.out_edges(NodeId(v))
            .filter(|e| loads(e.kind(), e.flags()))
            .filter_map(|e| e.to)
            .filter(|t| !g.is_external(*t))
            .map(|t| t.0)
            .collect()
    };
    for &root in files {
        if index[root.index()] != UNSEEN {
            continue;
        }
        // (node, its targets, the next target to look at)
        let mut work: Vec<(u32, Vec<u32>, usize)> = Vec::new();
        let mut enter = |v: u32,
                         work: &mut Vec<(u32, Vec<u32>, usize)>,
                         index: &mut Vec<u32>,
                         low: &mut Vec<u32>,
                         stack: &mut Vec<u32>,
                         on_stack: &mut Vec<bool>| {
            index[v as usize] = next;
            low[v as usize] = next;
            next += 1;
            stack.push(v);
            on_stack[v as usize] = true;
            work.push((v, targets(v), 0));
        };
        enter(
            root.0,
            &mut work,
            &mut index,
            &mut low,
            &mut stack,
            &mut on_stack,
        );
        while let Some((v, out, at)) = work.last_mut() {
            let v = *v;
            if let Some(&w) = out.get(*at) {
                *at += 1;
                if w == v {
                    cyclic[v as usize] = true;
                } else if index[w as usize] == UNSEEN {
                    enter(
                        w,
                        &mut work,
                        &mut index,
                        &mut low,
                        &mut stack,
                        &mut on_stack,
                    );
                } else if on_stack[w as usize] {
                    low[v as usize] = low[v as usize].min(index[w as usize]);
                }
                continue;
            }
            work.pop();
            if let Some((parent, _, _)) = work.last() {
                let p = *parent as usize;
                low[p] = low[p].min(low[v as usize]);
            }
            if low[v as usize] == index[v as usize] {
                let mut members = Vec::new();
                while let Some(w) = stack.pop() {
                    on_stack[w as usize] = false;
                    members.push(w);
                    if w == v {
                        break;
                    }
                }
                if members.len() > 1 {
                    for w in members {
                        cyclic[w as usize] = true;
                    }
                }
            }
        }
    }
    files.iter().map(|f| cyclic[f.index()]).collect()
}
