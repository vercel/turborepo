use anyhow::Result;
use turbo_tasks::{debug::ValueDebug, Completion, Vc};
use turbo_tasks_memory::MemoryBackend;

use crate::{
    register,
    val::{Add, Val},
};

#[turbo_tasks::value_trait]
pub trait MinusOne {
    fn minus_one(self: Vc<Self>) -> Vc<Self>;
}

#[turbo_tasks::value_impl]
impl MinusOne for Val {
    #[turbo_tasks::function]
    pub fn minus_one(self: Vc<Self>) -> Vc<Self> {}
}

#[turbo_tasks::value_impl]
impl MinusOne for &dyn Add {
    #[turbo_tasks::function]
    pub fn sub(self: Vc<Self>) -> Vc<Self> {
        todo!()
    }
}
