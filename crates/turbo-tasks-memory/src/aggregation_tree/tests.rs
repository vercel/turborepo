use std::{
    hash::Hash,
    sync::{
        atomic::{AtomicU32, Ordering},
        Arc,
    },
};

use parking_lot::{Mutex, MutexGuard};

use super::{aggregation_info, AggregationContext, AggregationItemLock, AggregationTreeLeaf};

struct Node {
    blue: bool,
    inner: Mutex<NodeInner>,
}

impl Node {
    fn incr(&self, context: &NodeAggregationContext) {
        let mut guard = self.inner.lock();
        guard.value += 10000;
        guard
            .aggregation_leaf
            .change(context, &Change { value: 10000 });
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
    aggregation_leaf: AggregationTreeLeaf<NodeAggregationContext>,
    value: u32,
}

struct NodeAggregationContext {
    additions: AtomicU32,
}

#[derive(Clone)]
struct NodeRef(Arc<Node>);

impl Hash for NodeRef {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        Arc::as_ptr(&self.0).hash(state);
    }
}

impl PartialEq for NodeRef {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for NodeRef {}

struct NodeGuard {
    guard: MutexGuard<'static, NodeInner>,
    node: Arc<Node>,
}

impl AggregationItemLock for NodeGuard {
    type Context = NodeAggregationContext;
    type ChildrenIter<'a> = impl Iterator<Item = NodeRef> + 'a;

    fn leaf(&mut self) -> &mut AggregationTreeLeaf<Self::Context> {
        &mut self.guard.aggregation_leaf
    }

    fn children(&self) -> Self::ChildrenIter<'_> {
        self.guard
            .children
            .iter()
            .map(|child| NodeRef(child.clone()))
    }

    fn is_blue(&self) -> bool {
        self.node.blue
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
}

impl AggregationContext for NodeAggregationContext {
    type ItemLock = NodeGuard;
    type Info = Aggregated;
    type ItemRef = NodeRef;
    type ItemChange = Change;

    fn new_info() -> Self::Info {
        Aggregated { value: 0 }
    }

    fn item(&self, reference: Self::ItemRef) -> Self::ItemLock {
        let r = reference.0.clone();
        let guard = r.inner.lock();
        NodeGuard {
            guard: unsafe { std::mem::transmute(guard) },
            node: r,
        }
    }

    fn apply_change(&self, info: &mut Aggregated, change: &Change) -> Option<Change> {
        if info.value != 0 {
            self.additions.fetch_add(1, Ordering::SeqCst);
            println!(
                "apply_change {:?} + {:?} = {:?}",
                info.value,
                change.value,
                info.value + change.value
            );
        }
        info.value += change.value;
        Some(change.clone())
    }

    fn info_to_add_change(&self, info: &Self::Info) -> Option<Self::ItemChange> {
        let change = Change {
            value: info.value as i32,
        };
        if change.is_empty() {
            None
        } else {
            Some(change)
        }
    }

    fn info_to_remove_change(&self, info: &Self::Info) -> Option<Self::ItemChange> {
        let change = Change {
            value: -(info.value as i32),
        };
        if change.is_empty() {
            None
        } else {
            Some(change)
        }
    }
}

struct Aggregated {
    value: i32,
}

#[test]
fn test() {
    let context = NodeAggregationContext {
        additions: AtomicU32::new(0),
    };
    let leaf = Arc::new(Node {
        blue: true,
        inner: Mutex::new(NodeInner {
            children: vec![],
            aggregation_leaf: AggregationTreeLeaf::new(),
            value: 10000,
        }),
    });
    let mut current = leaf.clone();
    for i in 1..=100 {
        current = Arc::new(Node {
            blue: i % 2 == 0,
            inner: Mutex::new(NodeInner {
                children: vec![current],
                aggregation_leaf: AggregationTreeLeaf::new(),
                value: i,
            }),
        });
    }
    let current = NodeRef(current);

    println!("aggregate");
    {
        let aggregated = aggregation_info(&context, current.clone());
        assert_eq!(aggregated.value, 15050);
    }
    assert_eq!(context.additions.load(Ordering::SeqCst), 100);
    context.additions.store(0, Ordering::SeqCst);

    println!("incr");
    leaf.incr(&context);
    assert_eq!(context.additions.load(Ordering::SeqCst), 12);
    context.additions.store(0, Ordering::SeqCst);

    println!("aggregate");
    {
        let aggregated = aggregation_info(&context, current.clone());
        assert_eq!(aggregated.value, 25050);
    }
    assert_eq!(context.additions.load(Ordering::SeqCst), 0);
    context.additions.store(0, Ordering::SeqCst);

    let current = Arc::new(Node {
        blue: false,
        inner: Mutex::new(NodeInner {
            children: vec![current.0],
            aggregation_leaf: AggregationTreeLeaf::new(),
            value: 101,
        }),
    });
    let current = NodeRef(current);

    println!("aggregate + 1");
    {
        let aggregated = aggregation_info(&context, current);
        assert_eq!(aggregated.value, 25151);
    }
    // This should be less the 100 to prove that we are reusing trees
    assert_eq!(context.additions.load(Ordering::SeqCst), 52);
    context.additions.store(0, Ordering::SeqCst);
}
