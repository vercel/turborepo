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

use nohash_hasher::IsEnabled;
use parking_lot::{Mutex, MutexGuard};
use ref_cast::RefCast;

use self::aggregation_data::prepare_aggregation_data;
use super::{
    aggregation_data, apply_change, new_edge::handle_new_edge, AggregationContext, AggregationNode,
    AggregationNodeGuard, RootQuery,
};
use crate::aggregation::{query_root_info, PreparedOperation, StackVec};

fn check_invariants<'a>(ctx: &NodeAggregationContext<'a>, node_ids: impl Iterator<Item = NodeRef>) {
    let mut queue = node_ids.collect::<Vec<_>>();
    // print(ctx, &queue[0], true);
    let mut visited = HashSet::new();
    while let Some(node_id) = queue.pop() {
        let node = ctx.node(&node_id);
        for child_id in node.children() {
            if visited.insert(child_id.clone()) {
                queue.push(child_id.clone());
            }
        }

        match &*node {
            AggregationNode::Leaf {
                aggregation_number,
                uppers,
                ..
            } => {
                let aggregation_number = *aggregation_number as u32;
                for upper_id in uppers.iter().cloned() {
                    {
                        let upper = ctx.node(&upper_id);
                        let upper_aggregation_number = upper.aggregation_number();
                        assert!(
                            upper_aggregation_number > aggregation_number
                                || aggregation_number == u32::MAX,
                            "upper #{} {} -> #{} {}",
                            node.guard.value,
                            aggregation_number,
                            upper.guard.value,
                            upper_aggregation_number
                        );
                    }
                    if visited.insert(upper_id.clone()) {
                        queue.push(upper_id);
                    }
                }
            }
            AggregationNode::Aggegating(aggegrating) => {
                let aggregation_number = aggegrating.aggregation_number;
                for upper_id in aggegrating.uppers.iter().cloned() {
                    {
                        let upper = ctx.node(&upper_id);
                        let upper_aggregation_number = upper.aggregation_number();
                        assert!(
                            upper_aggregation_number > aggregation_number
                                || aggregation_number == u32::MAX,
                            "upper #{} {} -> #{} {}",
                            node.guard.value,
                            aggregation_number,
                            upper.guard.value,
                            upper_aggregation_number
                        );
                    }
                    if visited.insert(upper_id.clone()) {
                        queue.push(upper_id);
                    }
                }
                for follower_id in aggegrating.followers.iter().cloned() {
                    {
                        let follower = ctx.node(&follower_id);
                        let follower_aggregation_number = follower.aggregation_number();
                        assert!(
                            follower_aggregation_number > aggregation_number
                                || aggregation_number == u32::MAX,
                            "follower #{} {} -> #{} {}",
                            node.guard.value,
                            aggregation_number,
                            follower.guard.value,
                            follower_aggregation_number
                        );
                    }
                    if visited.insert(follower_id.clone()) {
                        queue.push(follower_id);
                    }
                }
            }
        }
    }
}

fn print_graph<C: AggregationContext>(
    ctx: &C,
    entry: &C::NodeRef,
    show_internal: bool,
    name_fn: impl Fn(&C::NodeRef) -> String,
) {
    let mut queue = vec![entry.clone()];
    let mut visited = HashSet::new();
    while let Some(node_id) = queue.pop() {
        let name = name_fn(&node_id);
        let node = ctx.node(&node_id);
        let n = node.aggregation_number();
        let n = if n == u32::MAX {
            "♾".to_string()
        } else {
            n.to_string()
        };
        let color = if matches!(*node, AggregationNode::Leaf { .. }) {
            "gray"
        } else {
            "#99ff99"
        };
        let children = node.children().collect::<StackVec<_>>();
        let uppers = node.uppers().iter().cloned().collect::<StackVec<_>>();
        let followers = match &*node {
            AggregationNode::Aggegating(aggegrating) => aggegrating
                .followers
                .iter()
                .cloned()
                .collect::<StackVec<_>>(),
            AggregationNode::Leaf { .. } => StackVec::new(),
        };
        drop(node);

        if show_internal {
            println!(
                "\"{}\" [label=\"{}\\n{}\", style=filled, fillcolor=\"{}\"];",
                name, name, n, color
            );
        } else {
            println!(
                "\"{}\" [label=\"{}\\n{}\\n{}U {}F\", style=filled, fillcolor=\"{}\"];",
                name,
                name,
                n,
                uppers.len(),
                followers.len(),
                color,
            );
        }

        for child_id in children {
            let child_name = name_fn(&child_id);
            println!("\"{}\" -> \"{}\";", name, child_name);
            if visited.insert(child_id.clone()) {
                queue.push(child_id);
            }
        }
        if show_internal {
            for upper_id in uppers {
                let upper_name = name_fn(&upper_id);
                println!(
                    "\"{}\" -> \"{}\" [style=dashed, color=green];",
                    name, upper_name
                );
            }
            for follower_id in followers {
                let follower_name = name_fn(&follower_id);
                println!(
                    "\"{}\" -> \"{}\" [style=dashed, color=red];",
                    name, follower_name
                );
            }
        }
    }
}

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

    fn new_with_children(
        aggregation_context: &NodeAggregationContext,
        value: u32,
        children: Vec<Arc<Node>>,
    ) -> Arc<Self> {
        let node = Self::new(value);
        for child in children {
            node.add_child(aggregation_context, child);
        }
        node
    }

    fn add_child(self: &Arc<Node>, aggregation_context: &NodeAggregationContext, child: Arc<Node>) {
        self.add_child_unchecked(aggregation_context, child);
        check_invariants(aggregation_context, once(NodeRef(self.clone())));
    }

    fn add_child_unchecked(
        self: &Arc<Node>,
        aggregation_context: &NodeAggregationContext,
        child: Arc<Node>,
    ) {
        let mut guard = self.inner.lock();
        guard.children.push(child.clone());
        handle_new_edge(
            aggregation_context,
            unsafe { NodeGuard::new(guard, self.clone()) },
            &NodeRef(self.clone()),
            &NodeRef(child),
        );
    }

    fn incr(self: &Arc<Node>, aggregation_context: &NodeAggregationContext) {
        let mut guard = self.inner.lock();
        guard.value += 10000;
        apply_change(
            aggregation_context,
            unsafe { NodeGuard::new(guard, self.clone()) },
            Change { value: 10000 },
        );
        check_invariants(aggregation_context, once(NodeRef(self.clone())));
    }
}

#[derive(Copy, Clone)]
struct Change {
    value: i32,
}

impl Change {
    fn is_empty(&self) -> bool {
        self.value == 0
    }
}

struct NodeInner {
    children: Vec<Arc<Node>>,
    aggregation_node: AggregationNode<NodeRef, Aggregated>,
    value: u32,
}

struct NodeAggregationContext<'a> {
    additions: AtomicU32,
    #[allow(dead_code)]
    something_with_lifetime: &'a u32,
    add_value: bool,
}

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
        let change = Change {
            value: -(self.guard.value as i32),
        };
        if change.is_empty() {
            None
        } else {
            Some(change)
        }
    }

    fn get_add_change(&self) -> Option<Change> {
        let change = Change {
            value: self.guard.value as i32,
        };
        if change.is_empty() {
            None
        } else {
            Some(change)
        }
    }

    fn get_initial_data(&self) -> Self::Data {
        Aggregated {
            value: self.guard.value as i32,
            active: false,
        }
    }
}

impl<'a> AggregationContext for NodeAggregationContext<'a> {
    type Guard<'l> = NodeGuard where Self: 'l;
    type Data = Aggregated;
    type NodeRef = NodeRef;
    type DataChange = Change;

    fn node<'b>(&'b self, reference: &Self::NodeRef) -> Self::Guard<'b> {
        let r = reference.0.clone();
        let guard = reference.0.inner.lock();
        unsafe { NodeGuard::new(guard, r) }
    }

    fn apply_change(&self, info: &mut Aggregated, change: &Change) -> Option<Change> {
        if info.value != 0 {
            self.additions.fetch_add(1, Ordering::SeqCst);
        }
        if self.add_value {
            info.value += change.value;
            Some(*change)
        } else {
            None
        }
    }

    fn data_to_add_change(&self, info: &Self::Data) -> Option<Self::DataChange> {
        let change = Change { value: info.value };
        if change.is_empty() {
            None
        } else {
            Some(change)
        }
    }

    fn data_to_remove_change(&self, info: &Self::Data) -> Option<Self::DataChange> {
        let change = Change { value: -info.value };
        if change.is_empty() {
            None
        } else {
            Some(change)
        }
    }
}

#[derive(Default)]
struct ActiveQuery {
    active: bool,
}

impl RootQuery for ActiveQuery {
    type Data = Aggregated;
    type Result = bool;

    fn query(&mut self, data: &Self::Data) -> ControlFlow<()> {
        if data.active {
            self.active = true;
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    }

    fn result(self) -> Self::Result {
        self.active
    }
}

#[derive(Default)]
struct Aggregated {
    value: i32,
    active: bool,
}

#[test]
fn chain() {
    let something_with_lifetime = 0;
    let ctx = NodeAggregationContext {
        additions: AtomicU32::new(0),
        something_with_lifetime: &something_with_lifetime,
        add_value: true,
    };
    let root = Node::new(1);
    let mut current = root.clone();
    for i in 2..=100 {
        let node = Node::new(i);
        current.add_child(&ctx, node.clone());
        current = node;
    }
    let leaf = Node::new(10000);
    current.add_child(&ctx, leaf.clone());
    let current = NodeRef(root);

    {
        let root_info = query_root_info(&ctx, ActiveQuery::default(), NodeRef(leaf.clone()));
        assert!(!root_info);
    }

    {
        let aggregated = aggregation_data(&ctx, &current);
        assert_eq!(aggregated.value, 15050);
    }
    assert_eq!(ctx.additions.load(Ordering::SeqCst), 192);
    ctx.additions.store(0, Ordering::SeqCst);

    {
        let root_info = query_root_info(&ctx, ActiveQuery::default(), NodeRef(leaf.clone()));
        assert!(!root_info);
    }

    leaf.incr(&ctx);
    // The change need to propagate through 3 aggregated nodes
    assert_eq!(ctx.additions.load(Ordering::SeqCst), 3);
    ctx.additions.store(0, Ordering::SeqCst);

    {
        let mut aggregated = aggregation_data(&ctx, &current);
        assert_eq!(aggregated.value, 25050);
        aggregated.active = true;
    }
    assert_eq!(ctx.additions.load(Ordering::SeqCst), 0);
    ctx.additions.store(0, Ordering::SeqCst);

    {
        let root_info = query_root_info(&ctx, ActiveQuery::default(), NodeRef(leaf.clone()));
        assert!(root_info);
    }

    let i = 101;
    let current = Node::new_with_children(&ctx, i, vec![current.0]);
    let current = NodeRef(current);

    {
        let aggregated = aggregation_data(&ctx, &current);
        assert_eq!(aggregated.value, 25151);
    }
    // This should be way less the 100 to prove that we are reusing trees
    assert_eq!(ctx.additions.load(Ordering::SeqCst), 1);
    ctx.additions.store(0, Ordering::SeqCst);

    leaf.incr(&ctx);
    // This should be less the 20 to prove that we are reusing trees
    assert_eq!(ctx.additions.load(Ordering::SeqCst), 4);
    ctx.additions.store(0, Ordering::SeqCst);

    {
        let root_info = query_root_info(&ctx, ActiveQuery::default(), NodeRef(leaf.clone()));
        assert!(root_info);
    }

    print(&ctx, &current, true);
    check_invariants(&ctx, once(current.clone()));
}

#[test]
fn chain_double_connected() {
    let something_with_lifetime = 0;
    let ctx = NodeAggregationContext {
        additions: AtomicU32::new(0),
        something_with_lifetime: &something_with_lifetime,
        add_value: true,
    };
    let root = Node::new(1);
    let mut current = root.clone();
    let mut current2 = Node::new(2);
    current.add_child(&ctx, current2.clone());
    for i in 3..=30 {
        let node = Node::new(i);
        current.add_child(&ctx, node.clone());
        current2.add_child(&ctx, node.clone());
        current = current2;
        current2 = node;
    }
    let current = NodeRef(root);

    {
        let aggregated = aggregation_data(&ctx, &current);
        assert_eq!(aggregated.value, 1201);
    }
    check_invariants(&ctx, once(current.clone()));
    assert_eq!(ctx.additions.load(Ordering::SeqCst), 78);
    ctx.additions.store(0, Ordering::SeqCst);

    print(&ctx, &current, true);
}

const RECT_SIZE: usize = 20;
const RECT_MULT: usize = 100;

#[test]
fn rectangle_tree() {
    let something_with_lifetime = 0;
    let ctx = NodeAggregationContext {
        additions: AtomicU32::new(0),
        something_with_lifetime: &something_with_lifetime,
        add_value: false,
    };
    let mut nodes: Vec<Vec<Arc<Node>>> = Vec::new();
    for y in 0..RECT_SIZE {
        let mut line: Vec<Arc<Node>> = Vec::new();
        for x in 0..RECT_SIZE {
            let mut parents = Vec::new();
            if x > 0 {
                parents.push(line[x - 1].clone());
            }
            if y > 0 {
                parents.push(nodes[y - 1][x].clone());
            }
            let value = (x + y * RECT_MULT) as u32;
            let node = Node::new(value);
            if x == 0 || y == 0 {
                prepare_aggregation_data(&ctx, &NodeRef(node.clone()));
            }
            for parent in parents {
                parent.add_child(&ctx, node.clone());
            }
            line.push(node);
        }
        nodes.push(line);
    }

    let root = NodeRef(nodes[0][0].clone());

    print(&ctx, &root, false);
}

#[test]
fn many_children() {
    let something_with_lifetime = 0;
    let ctx = NodeAggregationContext {
        additions: AtomicU32::new(0),
        something_with_lifetime: &something_with_lifetime,
        add_value: false,
    };
    let mut roots: Vec<Arc<Node>> = Vec::new();
    let mut children: Vec<Arc<Node>> = Vec::new();
    const CHILDREN: u32 = 50000;
    const ROOTS: u32 = 1000;
    let inner_node = Node::new(0);
    let start = Instant::now();
    for i in 0..ROOTS {
        let node = Node::new(10000 + i);
        roots.push(node.clone());
        aggregation_data(&ctx, &NodeRef(node.clone())).active = true;
        node.add_child_unchecked(&ctx, inner_node.clone());
    }
    println!("Roots: {:?}", start.elapsed());
    let start = Instant::now();
    for i in 0..CHILDREN {
        let node = Node::new(20000 + i);
        children.push(node.clone());
        inner_node.add_child_unchecked(&ctx, node.clone());
    }
    println!("Children: {:?}", start.elapsed());
    let start = Instant::now();
    for i in 0..ROOTS {
        let node = Node::new(30000 + i);
        roots.push(node.clone());
        aggregation_data(&ctx, &NodeRef(node.clone())).active = true;
        node.add_child_unchecked(&ctx, inner_node.clone());
    }
    println!("Roots: {:?}", start.elapsed());
    let start = Instant::now();
    for i in 0..CHILDREN {
        let node = Node::new(40000 + i);
        children.push(node.clone());
        inner_node.add_child_unchecked(&ctx, node.clone());
    }
    let children_duration = start.elapsed();
    println!("Children: {:?}", children_duration);
    let mut number_of_slow_children = 0;
    for j in 0..10 {
        let start = Instant::now();
        for i in 0..CHILDREN {
            let node = Node::new(50000 + j * 10000 + i);
            children.push(node.clone());
            inner_node.add_child_unchecked(&ctx, node.clone());
        }
        let dur = start.elapsed();
        println!("Children: {:?}", dur);
        if dur > children_duration * 2 {
            number_of_slow_children += 1;
        }
    }

    // Technically it should always be 0, but the performance of the environment
    // might vary so we accept a few slow children
    assert!(number_of_slow_children < 3);

    let root = NodeRef(roots[0].clone());

    check_invariants(&ctx, roots.iter().cloned().map(NodeRef));

    // print(&ctx, &root, false);
}

fn connect_child(
    aggregation_context: &NodeAggregationContext<'_>,
    parent: &Arc<Node>,
    child: &Arc<Node>,
) {
    let state = parent.inner.lock();
    let node_guard = unsafe { NodeGuard::new(state, parent.clone()) };
    let NodeGuard {
        guard: mut state, ..
    } = node_guard;
    state.children.push(child.clone());
    let job = state.aggregation_node.handle_new_edge(
        aggregation_context,
        &NodeRef(parent.clone()),
        &NodeRef(child.clone()),
    );
    drop(state);
    job.apply(aggregation_context);
}

fn print(aggregation_context: &NodeAggregationContext<'_>, current: &NodeRef, show_internal: bool) {
    println!("digraph {{");
    print_graph(aggregation_context, current, show_internal, |item| {
        format!("#{}", item.0.inner.lock().value)
    });
    println!("\n}}");
}
