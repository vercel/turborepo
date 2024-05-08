use std::{cmp::max, collections::HashMap, hash::Hash, mem::take};

use indexmap::IndexSet;

use super::{balance_edge, AggregationContext};

pub struct BalanceQueue<I> {
    queue: IndexSet<(I, I)>,
    aggregation_numbers: HashMap<I, u32>,
}

impl<I: Hash + Eq + Clone> BalanceQueue<I> {
    pub fn new() -> Self {
        Self {
            queue: IndexSet::default(),
            aggregation_numbers: HashMap::default(),
        }
    }

    fn add_number(&mut self, id: I, number: u32) {
        self.aggregation_numbers
            .entry(id)
            .and_modify(|n| *n = max(*n, number))
            .or_insert(number);
    }

    pub fn balance(
        &mut self,
        upper_id: I,
        upper_aggregation_number: u32,
        target_id: I,
        target_aggregation_number: u32,
    ) {
        self.add_number(upper_id.clone(), upper_aggregation_number);
        self.add_number(target_id.clone(), target_aggregation_number);
        self.queue.insert((upper_id.clone(), target_id.clone()));
    }

    pub fn balance_all(&mut self, edges: Vec<(I, u32, I, u32)>) {
        for (upper_id, upper_aggregation_number, target_id, target_aggregation_number) in edges {
            self.balance(
                upper_id,
                upper_aggregation_number,
                target_id,
                target_aggregation_number,
            );
        }
    }

    pub fn process<C: AggregationContext<NodeRef = I>>(mut self, ctx: &C) {
        while !self.queue.is_empty() {
            let queue = take(&mut self.queue);
            for (upper_id, target_id) in queue {
                let upper_aggregation_number = self
                    .aggregation_numbers
                    .get(&upper_id)
                    .copied()
                    .unwrap_or_default();
                let target_aggregation_number = self
                    .aggregation_numbers
                    .get(&target_id)
                    .copied()
                    .unwrap_or_default();

                let (u, t) = balance_edge(
                    ctx,
                    &mut self,
                    &upper_id,
                    upper_aggregation_number,
                    &target_id,
                    target_aggregation_number,
                );
                if u != upper_aggregation_number {
                    self.add_number(upper_id, u);
                }
                if t != target_aggregation_number {
                    self.add_number(target_id, t);
                }
            }
        }
    }
}
