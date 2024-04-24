use std::{hash::Hash, ops::ControlFlow};

use auto_hash_map::AutoSet;

use super::{AggregationContext, AggregationNode, StackVec};

pub trait RootQuery {
    type Data;
    type Result;

    fn query(&mut self, data: &Self::Data) -> ControlFlow<()>;
    fn result(self) -> Self::Result;
}

impl<I: Clone + Eq + Hash, D> AggregationNode<I, D> {
    pub fn query_root_info<C: AggregationContext<NodeRef = I, Data = D>, Q: RootQuery<Data = D>>(
        &self,
        ctx: &C,
        mut query: Q,
        node_id: C::NodeRef,
    ) -> Q::Result {
        let mut queue = StackVec::new();
        let mut visited = AutoSet::new();
        visited.insert(node_id);
        match self {
            AggregationNode::Leaf { uppers, .. } => {
                for upper_id in uppers.iter() {
                    if visited.insert(upper_id.clone()) {
                        queue.push(upper_id.clone());
                    }
                }
            }
            AggregationNode::Aggegating(aggegrating) => {
                if let ControlFlow::Break(_) = query.query(&aggegrating.data) {
                    return query.result();
                }
                for upper_id in aggegrating.uppers.iter() {
                    if visited.insert(upper_id.clone()) {
                        queue.push(upper_id.clone());
                    }
                }
            }
        }
        process_queue(ctx, query, queue, visited)
    }
}

pub fn query_root_info<C: AggregationContext, Q: RootQuery<Data = C::Data>>(
    ctx: &C,
    query: Q,
    node_id: C::NodeRef,
) -> Q::Result {
    let mut queue = StackVec::new();
    queue.push(node_id);
    let visited = AutoSet::new();
    process_queue(ctx, query, queue, visited)
}

fn process_queue<C: AggregationContext, Q: RootQuery<Data = C::Data>>(
    ctx: &C,
    mut query: Q,
    mut queue: StackVec<C::NodeRef>,
    mut visited: AutoSet<C::NodeRef>,
) -> Q::Result {
    while let Some(node_id) = queue.pop() {
        let node = ctx.node(&node_id);
        match &*node {
            AggregationNode::Leaf { uppers, .. } => {
                for upper_id in uppers.iter() {
                    if visited.insert(upper_id.clone()) {
                        queue.push(upper_id.clone());
                    }
                }
            }
            AggregationNode::Aggegating(aggegrating) => {
                if let ControlFlow::Break(_) = query.query(&aggegrating.data) {
                    return query.result();
                }
                for upper_id in aggegrating.uppers.iter() {
                    if visited.insert(upper_id.clone()) {
                        queue.push(upper_id.clone());
                    }
                }
            }
        }
    }
    query.result()
}
