use anyhow::Error;
use fxhash::FxHashSet;
use swc_core::ecma::{ast::Module, atoms::JsWord};

pub trait Load {
    fn load(&mut self, uri: &str) -> Result<Option<Module>, Error>;
}

pub struct Merger<L> {
    loader: L,

    done: FxHashSet<JsWord>,
}

impl<L> Merger<L> {}
