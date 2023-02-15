use anyhow::Error;
use swc_core::ecma::ast::Module;

pub trait Load {
    fn load(&mut self, uri: &str) -> Result<Option<Module>, Error>;
}

pub struct Merger<L> {
    loader: L,
}

impl<L> Merger<L> {}
