use std::{
    hash::{Hash, Hasher},
    sync::Arc,
};

use super::{bottom_tree::BottomTree, top_tree::TopTree, AggregationContext};

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChildLocation {
    Left,
    Middle,
    Right,
}

pub struct BottomRef<T: AggregationContext> {
    pub parent: Arc<BottomTree<T>>,
    pub location: ChildLocation,
}

impl<T: AggregationContext> Hash for BottomRef<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        Arc::as_ptr(&self.parent).hash(state);
        self.location.hash(state);
    }
}

impl<T: AggregationContext> PartialEq for BottomRef<T> {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.parent, &other.parent) && self.location == other.location
    }
}

impl<T: AggregationContext> Eq for BottomRef<T> {}

pub struct TopRef<T: AggregationContext> {
    pub parent: Arc<TopTree<T>>,
}

impl<T: AggregationContext> Hash for TopRef<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        Arc::as_ptr(&self.parent).hash(state);
    }
}

impl<T: AggregationContext> PartialEq for TopRef<T> {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.parent, &other.parent)
    }
}

impl<T: AggregationContext> Eq for TopRef<T> {}
