use anyhow::Error;
use fxhash::FxHashSet;
use swc_core::ecma::{
    ast::{Expr, KeyValueProp, Lit, Module, ModuleDecl, ModuleItem, Prop, PropName, PropOrSpread},
    atoms::JsWord,
};

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

                    let chunk_id = import.asserts.as_deref().and_then(|asserts| {
                        asserts.props.iter().find_map(|prop| match prop {
                            PropOrSpread::Prop(box Prop::KeyValue(KeyValueProp {
                                key: PropName::Ident(key),
                                value: box Expr::Lit(Lit::Num(chunk_id)),
                            })) if key.sym == *"__turbopack_chunk__" => Some(chunk_id.value as u32),
                            _ => None,
                        })
                    });

                    if let Some(chunk_id) = chunk_id {
                        if self.done.insert((import.src.value.clone(), chunk_id)) {
                            if let Some(dep) = self.loader.load(&import.src.value, chunk_id)? {
                                let mut dep = self.merge_recursively(dep)?;

                                content.append(&mut dep.body);
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
