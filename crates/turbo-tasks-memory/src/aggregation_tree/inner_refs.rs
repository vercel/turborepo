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
