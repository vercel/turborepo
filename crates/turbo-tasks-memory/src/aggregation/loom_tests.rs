use std::{
    collections::HashSet,
    hash::Hash,
    iter::once,
    ops::{ControlFlow, Deref, DerefMut},
    sync::{
        atomic::{AtomicU32, Ordering},
        Arc,
    },
    time::Instant,
};

use indexmap::IndexSet;
use loom::{
    sync::{Mutex, MutexGuard},
    thread,
};
use nohash_hasher::IsEnabled;
use rand::{
    rngs::{SmallRng, StdRng},
    Rng, SeedableRng,
};
use ref_cast::RefCast;
use rstest::*;

use self::aggregation_data::prepare_aggregation_data;
use super::{
    aggregation_data, apply_change, lost_edge::handle_lost_edge, new_edge::handle_new_edge,
    AggregationContext, AggregationNode, AggregationNodeGuard, RootQuery,
};
use crate::aggregation::{query_root_info, PreparedOperation, StackVec};

struct Node {
    inner: Mutex<NodeInner>,
}

impl Node {
    fn new(value: u32) -> Arc<Self> {
        Arc::new(Node {
            inner: Mutex::new(NodeInner {
                children: Vec::new(),
                aggregation_node: AggregationNode::new(),
                value,
            }),
        })
    }

    fn add_child(self: &Arc<Node>, aggregation_context: &NodeAggregationContext, child: Arc<Node>) {
        let mut guard = self.inner.lock().unwrap();
        guard.children.push(child.clone());
        handle_new_edge(
            aggregation_context,
            unsafe { NodeGuard::new(guard, self.clone()) },
            &NodeRef(self.clone()),
            &NodeRef(child),
        );
    }
}

#[derive(Copy, Clone)]
struct Change {}

struct NodeInner {
    children: Vec<Arc<Node>>,
    aggregation_node: AggregationNode<NodeRef, Aggregated>,
    value: u32,
}

struct NodeAggregationContext {}

#[derive(Clone, RefCast)]
#[repr(transparent)]
struct NodeRef(Arc<Node>);

impl Hash for NodeRef {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        Arc::as_ptr(&self.0).hash(state);
    }
}

impl IsEnabled for NodeRef {}

impl PartialEq for NodeRef {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for NodeRef {}

struct NodeGuard {
    guard: MutexGuard<'static, NodeInner>,
    // This field is important to keep the node alive
    #[allow(dead_code)]
    node: Arc<Node>,
}

impl NodeGuard {
    unsafe fn new(guard: MutexGuard<'_, NodeInner>, node: Arc<Node>) -> Self {
        NodeGuard {
            guard: unsafe { std::mem::transmute(guard) },
            node,
        }
    }
}

impl Deref for NodeGuard {
    type Target = AggregationNode<NodeRef, Aggregated>;

    fn deref(&self) -> &Self::Target {
        &self.guard.aggregation_node
    }
}

impl DerefMut for NodeGuard {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.guard.aggregation_node
    }
}

impl AggregationNodeGuard for NodeGuard {
    type Data = Aggregated;
    type NodeRef = NodeRef;
    type DataChange = Change;
    type ChildrenIter<'a> = impl Iterator<Item = NodeRef> + 'a;

    fn number_of_children(&self) -> usize {
        self.guard.children.len()
    }

    fn children(&self) -> Self::ChildrenIter<'_> {
        self.guard
            .children
            .iter()
            .map(|child| NodeRef(child.clone()))
    }

    fn get_remove_change(&self) -> Option<Change> {
        None
    }

    fn get_add_change(&self) -> Option<Change> {
        None
    }

    fn get_initial_data(&self) -> Self::Data {
        Aggregated {}
    }
}

impl AggregationContext for NodeAggregationContext {
    type Guard<'l> = NodeGuard where Self: 'l;
    type Data = Aggregated;
    type NodeRef = NodeRef;
    type DataChange = Change;

    fn node<'b>(&'b self, reference: &Self::NodeRef) -> Self::Guard<'b> {
        let r = reference.0.clone();
        let guard = reference.0.inner.lock().unwrap();
        unsafe { NodeGuard::new(guard, r) }
    }

    fn apply_change(&self, info: &mut Aggregated, change: &Change) -> Option<Change> {
        None
    }

    fn data_to_add_change(&self, info: &Self::Data) -> Option<Self::DataChange> {
        None
    }

    fn data_to_remove_change(&self, info: &Self::Data) -> Option<Self::DataChange> {
        None
    }
}

#[derive(Default)]
struct Aggregated {}

// #[test]
fn fuzzy_loom_new() {
    for size in [10, 20] {
        for _ in 0..100000 {
            let seed = rand::random();
            println!("Seed {} Size {}", seed, size);
            fuzzy_loom(seed, size);
        }
    }
}

#[rstest]
#[case::a(3302552607, 10)]
#[case::b(3629477471, 50)]
fn fuzzy_loom(#[case] seed: u32, #[case] count: u32) {
    loom::model(move || {
        loom::skip_branch();
        thread::Builder::new()
            .stack_size(80000)
            .spawn(move || {
                loom::skip_branch();
                let ctx = NodeAggregationContext {};

                let mut seed_buffer = [0; 32];
                seed_buffer[0..4].copy_from_slice(&seed.to_be_bytes());
                let mut r = SmallRng::from_seed(seed_buffer);
                let mut nodes = Vec::new();
                for i in 0..count {
                    nodes.push(Node::new(i));
                }
                prepare_aggregation_data(&ctx, &NodeRef(nodes[0].clone()));

                // setup graph
                for _ in 0..50 {
                    let parent = r.gen_range(0..nodes.len() - 1);
                    let child = r.gen_range(parent + 1..nodes.len());
                    let parent_node = nodes[parent].clone();
                    let child_node = nodes[child].clone();
                    parent_node.add_child(&ctx, child_node);
                }

                let mut edges = Vec::new();
                for _ in 0..2 {
                    let parent = r.gen_range(0..nodes.len() - 1);
                    let child = r.gen_range(parent + 1..nodes.len());
                    let parent_node = nodes[parent].clone();
                    let child_node = nodes[child].clone();
                    edges.push((parent_node, child_node));
                }

                let ctx = Arc::new(ctx);

                loom::explore();

                let mut threads = Vec::new();

                // Fancy testing
                for (parent_node, child_node) in edges.iter() {
                    let parent_node = parent_node.clone();
                    let child_node = child_node.clone();
                    let ctx = ctx.clone();
                    threads.push(
                        thread::Builder::new()
                            .stack_size(80000)
                            .spawn(move || {
                                parent_node.add_child(&ctx, child_node);
                            })
                            .unwrap(),
                    );
                }

                for thread in threads {
                    thread.join().unwrap();
                }
            })
            .unwrap()
            .join()
            .unwrap();
    });
}
