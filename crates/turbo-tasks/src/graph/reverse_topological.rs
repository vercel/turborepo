use std::{
    collections::{HashMap, HashSet},
    fmt::{Display, Write},
    future::Future,
    path::Path,
    sync::atomic::AtomicUsize,
};

use super::graph_store::GraphStore;
use crate::{debug::ValueDebug, TryJoinIterExt};

/// A graph traversal that returns nodes in reverse topological order.
pub struct ReverseTopological<T>
where
    T: Eq + std::hash::Hash + Clone,
{
    adjacency_map: HashMap<T, Vec<T>>,
    roots: Vec<T>,
}

impl<T> Default for ReverseTopological<T>
where
    T: Eq + std::hash::Hash + Clone,
{
    fn default() -> Self {
        Self {
            adjacency_map: HashMap::new(),
            roots: Vec::new(),
        }
    }
}

impl<T> GraphStore<T> for ReverseTopological<T>
where
    T: Eq + std::hash::Hash + Clone,
{
    type Handle = T;

    fn insert(&mut self, from_handle: Option<T>, node: T) -> Option<(Self::Handle, &T)> {
        let vec = if let Some(from_handle) = from_handle {
            self.adjacency_map
                .entry(from_handle)
                .or_insert_with(|| Vec::with_capacity(1))
        } else {
            &mut self.roots
        };

        vec.push(node.clone());
        Some((node, vec.last().unwrap()))
    }
}

impl<T> ReverseTopological<T>
where
    T: Eq + std::hash::Hash + Clone + ValueDebug,
{
    pub async fn viz<F, Fut, Fmt, P>(self, path: P, label: F) -> anyhow::Result<Self>
    where
        F: Fn(&T) -> Fut + Sync,
        Fut: Future<Output = anyhow::Result<Fmt>>,
        Fmt: Display,
        P: AsRef<Path>,
    {
        let mut dot = String::new();

        write!(dot, "digraph {{\n")?;

        let mut all_nodes = HashSet::new();

        for node in &self.roots {
            all_nodes.insert(node);
        }

        for (node, neighbors) in &self.adjacency_map {
            all_nodes.insert(node);

            for neighbor in neighbors {
                all_nodes.insert(neighbor);
            }
        }

        let idx = AtomicUsize::new(0);
        let label = &label;
        let node_map: HashMap<_, _> = all_nodes
            .into_iter()
            .map(|node| {
                let idx = idx.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                async move {
                    let label = label(node).await?;
                    Ok((node, (idx, label)))
                }
            })
            .try_join()
            .await?
            .into_iter()
            .collect();

        for (_, (idx, label)) in &node_map {
            write!(dot, "  {} [label={}];\n", idx, label)?;
        }

        for (node, neighbors) in &self.adjacency_map {
            let (node_idx, _) = node_map.get(node).unwrap();
            for neighbor in neighbors {
                let (neighbor_idx, _) = node_map.get(neighbor).unwrap();
                write!(dot, "  {} -> {};\n", node_idx, neighbor_idx)?;
            }
        }

        write!(dot, "}}\n")?;

        std::fs::write(path, dot)?;

        Ok(self)
    }
}

#[derive(Debug)]
enum ReverseTopologicalPass {
    Pre,
    Post,
}

impl<T> IntoIterator for ReverseTopological<T>
where
    T: Eq + std::hash::Hash + Clone,
{
    type Item = T;
    type IntoIter = ReverseTopologicalIntoIter<T>;

    fn into_iter(self) -> Self::IntoIter {
        ReverseTopologicalIntoIter {
            adjacency_map: self.adjacency_map,
            stack: self
                .roots
                .into_iter()
                .map(|root| (ReverseTopologicalPass::Pre, root))
                .collect(),
            visited: HashSet::new(),
        }
    }
}

pub struct ReverseTopologicalIntoIter<T>
where
    T: Eq + std::hash::Hash + Clone,
{
    adjacency_map: HashMap<T, Vec<T>>,
    stack: Vec<(ReverseTopologicalPass, T)>,
    visited: HashSet<T>,
}

impl<T> Iterator for ReverseTopologicalIntoIter<T>
where
    T: Eq + std::hash::Hash + Clone,
{
    type Item = T;

    fn next(&mut self) -> Option<Self::Item> {
        let current = loop {
            let (pass, current) = self.stack.pop()?;

            match pass {
                ReverseTopologicalPass::Post => {
                    break current;
                }
                ReverseTopologicalPass::Pre => {
                    if self.visited.contains(&current) {
                        continue;
                    }

                    self.visited.insert(current.clone());

                    let Some(neighbors) = self.adjacency_map.get(&current) else {
                        break current;
                    };

                    self.stack.push((ReverseTopologicalPass::Post, current));
                    self.stack.extend(
                        neighbors
                            .iter()
                            .map(|neighbor| (ReverseTopologicalPass::Pre, neighbor.clone())),
                    );
                }
            }
        };

        Some(current)
    }
}
