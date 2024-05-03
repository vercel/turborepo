use std::mem::take;

use super::{balance_edge, AggregationContext, StackVec};

pub struct BalanceQueue<I> {
    queue: StackVec<(I, I)>,
}

impl<I> BalanceQueue<I> {
    pub fn new() -> Self {
        Self {
            queue: StackVec::new(),
        }
    }

    pub fn balance(&mut self, upper_id: I, target_id: I) {
        self.queue.push((upper_id, target_id));
    }

    pub fn balance_all(&mut self, edges: Vec<(I, I)>) {
        self.queue.extend(edges);
    }

    pub fn process<C: AggregationContext<NodeRef = I>>(mut self, ctx: &C) {
        while !self.queue.is_empty() {
            let queue = take(&mut self.queue);
            for (upper_id, target_id) in queue {
                balance_edge(ctx, &mut self, upper_id, target_id);
            }
        }
    }
}
