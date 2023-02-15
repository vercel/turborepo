use anyhow::Error;
use fxhash::FxHashSet;
use swc_core::ecma::{
    ast::{Module, ModuleDecl, ModuleItem},
    atoms::JsWord,
};

pub trait Load {
    fn load(&mut self, uri: &str) -> Result<Option<Module>, Error>;
}

pub struct Merger<L>
where
    L: Load,
{
    loader: L,

    done: FxHashSet<JsWord>,
}

impl<L> Merger<L>
where
    L: Load,
{
    pub fn merge_recursively(&mut self, entry: Module) -> Result<Module, Error> {
        let mut content = vec![];
        let mut extra_body = vec![];

        for stmt in entry.body {
            match stmt {
                ModuleItem::ModuleDecl(ModuleDecl::Import(import)) => {
                    // Try to prepend the content of module

                    if let Some(dep) = self.loader.load(&import.src.value)? {
                        let mut dep = self.merge_recursively(dep)?;

                        content.append(&mut dep.body);
                    } else {
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
