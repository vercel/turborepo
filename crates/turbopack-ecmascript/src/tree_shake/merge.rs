use anyhow::Error;
use rustc_hash::FxHashSet;
use swc_core::ecma::{
    ast::{Module, ModuleDecl, ModuleItem},
    atoms::JsWord,
};

use super::graph::find_turbopack_chunk_id_in_asserts;

pub trait Load {
    fn load(&mut self, uri: &str, chunk_id: u32) -> Result<Option<Module>, Error>;
}

pub struct Merger<L>
where
    L: Load,
{
    loader: L,

    done: FxHashSet<(JsWord, u32)>,
}

impl<L> Merger<L>
where
    L: Load,
{
    pub fn new(loader: L) -> Self {
        Merger {
            loader,
            done: Default::default(),
        }
    }

    pub fn merge_recursively(&mut self, entry: Module) -> Result<Module, Error> {
        let mut content = vec![];
        let mut extra_body = vec![];

        for stmt in entry.body {
            match stmt {
                ModuleItem::ModuleDecl(ModuleDecl::Import(import)) => {
                    // Try to prepend the content of module

                    let chunk_id = import
                        .asserts
                        .as_deref()
                        .and_then(|asserts| find_turbopack_chunk_id_in_asserts(asserts));

                    if let Some(chunk_id) = chunk_id {
                        if self.done.insert((import.src.value.clone(), chunk_id)) {
                            if let Some(dep) = self.loader.load(&import.src.value, chunk_id)? {
                                let mut dep = self.merge_recursively(dep)?;

                                extra_body.append(&mut dep.body);
                            } else {
                                content.push(ModuleItem::ModuleDecl(ModuleDecl::Import(import)));
                            }
                        } else {
                            // Remove import
                        }
                    } else {
                        // Preserve normal imports
                        content.push(ModuleItem::ModuleDecl(ModuleDecl::Import(import)));
                    }
                }
                _ => extra_body.push(stmt),
            }
        }

        content.append(&mut extra_body);

        Ok(Module {
            span: entry.span,
            body: content,
            shebang: entry.shebang,
        })
    }
}
