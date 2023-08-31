use std::{
    hash::{Hash, Hasher},
    sync::Arc,
};

use super::{bottom_tree::BottomTree, top_tree::TopTree};

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChildLocation {
    Left,
    Middle,
    Right,
}

pub struct BottomRef<T, I> {
    pub parent: Arc<BottomTree<T, I>>,
    pub location: ChildLocation,
}

impl<T, I> Hash for BottomRef<T, I> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        Arc::as_ptr(&self.parent).hash(state);
        self.location.hash(state);
    }
}

impl<T, I> PartialEq for BottomRef<T, I> {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.parent, &other.parent) && self.location == other.location
    }
}

impl<T, I> Eq for BottomRef<T, I> {}

pub struct TopRef<T> {
    pub parent: Arc<TopTree<T>>,
}

impl<T> Hash for TopRef<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        Arc::as_ptr(&self.parent).hash(state);
    }
}

impl<T> PartialEq for TopRef<T> {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.parent, &other.parent)
    }
}

impl<T> Eq for TopRef<T> {}
