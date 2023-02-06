use fxhash::{FxBuildHasher, FxHashMap};
use indexmap::IndexSet;
use petgraph::{prelude::DiGraphMap, Directed};
use swc_core::ecma::{
    ast::{
        op, ClassDecl, Decl, ExportDecl, ExportSpecifier, Expr, ExprStmt, FnDecl, Id,
        ImportSpecifier, Module, ModuleDecl, ModuleExportName, ModuleItem, Stmt,
    },
    atoms::js_word,
    utils::find_pat_ids,
};

use super::util::{ids_captured_by, ids_used_by, ids_used_by_ignoring_nested};

/// The id of an item
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) struct ItemId {
    /// The index of the module item in the module.
    pub index: usize,
    pub kind: ItemIdKind,
}

/// ## Import
///
/// ```js
/// import { upper } from "module";
/// ```
///
/// becomes [ItemIdKind::ImportOfModule] and [ItemIdKind::ImportBinding].
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) enum ItemIdKind {
    ///
    Normal,

    ImportOfModule,
    /// Imports are splitted as multiple items.
    ImportBinding(u32),
    VarDeclarator(u32),

    ModuleEvaluation,

    Export(Id),
}

/// Data about a module item
#[derive(Debug, Default)]
pub(super) struct ItemData {
    /// If the module item is hoisted?
    pub is_hoisted: bool,

    /// Variables declared or bound by this module item?
    pub var_decls: Vec<Id>,

    /// Variables read by this module item during evaluation?
    pub read_vars: Vec<Id>,

    /// Variables read by this module item eventually?
    ///
    /// - e.g. variables read in the body of function declarations are
    ///   considered
    ///  as eventually read
    /// - This is only used when reads are not trigger directly by this module
    ///   item, but require a side effect to be triggered. We don’t know when
    ///   this is executed.
    /// - Note: This doesn’t mean they are only read “after” initial evaluation.
    ///   They might also be read “during” initial evaluation on any module item
    ///   with SIDE_EFFECTS. This kind of interaction is handled by the module
    ///   item with SIDE_EFFECTS.
    pub eventual_read_vars: Vec<Id>,

    /// Side effects that are triggered on local variables during evaluation?
    pub write_vars: Vec<Id>,

    /// Side effects that are triggered on local variables eventually?
    pub eventual_write_vars: Vec<Id>,

    /// Are other unknown side effects that are trigger during evaluation?
    pub side_effects: bool,
}

#[derive(Debug)]
pub(super) struct VarInfo {}

#[derive(Debug, Default)]
pub struct Graph {
    /// `bool`: Strong
    pub(super) inner: DiGraphMap<u32, bool>,
    pub(super) graph_ix: IndexSet<ItemId, FxBuildHasher>,
}

impl Graph {
    pub(super) fn finalize(&self) -> petgraph::Graph<Vec<u32>, bool, Directed, u32> {
        let graph = self.inner.clone().into_graph();

        let mut condensed: petgraph::Graph<_, _, _, u32> =
            super::condensation::condensation(graph, |strong1, strong2| strong1 || strong2);

        condensed
    }

    pub(super) fn node(&mut self, id: &ItemId) -> u32 {
        self.graph_ix.get_index_of(id).unwrap_or_else(|| {
            let ix = self.graph_ix.len();
            self.graph_ix.insert_full(id.clone());
            ix
        }) as _
    }

    /// Fills information per module items
    pub(super) fn init(&mut self, module: &Module) -> (Vec<ItemId>, FxHashMap<ItemId, ItemData>) {
        let mut exports = vec![];
        let mut items = FxHashMap::default();
        let mut ids = vec![];

        for (index, item) in module.body.iter().enumerate() {
            // Fill exports
            if let ModuleItem::ModuleDecl(item) = item {
                match item {
                    ModuleDecl::ExportDecl(item) => match &item.decl {
                        Decl::Fn(FnDecl { ident, .. }) | Decl::Class(ClassDecl { ident, .. }) => {
                            exports.push(ident.to_id());
                        }
                        Decl::Var(v) => {
                            for decl in &v.decls {
                                let ids: Vec<Id> = find_pat_ids(&decl.name);
                                for id in ids {
                                    exports.push(id);
                                }
                            }
                        }
                        _ => {}
                    },
                    ModuleDecl::ExportNamed(item) => {
                        if item.src.is_none() {
                            for s in &item.specifiers {
                                match s {
                                    ExportSpecifier::Named(s) => {
                                        match s.exported.as_ref().unwrap_or(&s.orig) {
                                            ModuleExportName::Ident(i) => {
                                                exports.push(i.to_id());
                                            }
                                            ModuleExportName::Str(s) => {}
                                        }
                                    }
                                    ExportSpecifier::Default(s) => {
                                        exports.push((js_word!("default"), Default::default()));
                                    }
                                    ExportSpecifier::Namespace(s) => match &s.name {
                                        ModuleExportName::Ident(i) => {
                                            exports.push(i.to_id());
                                        }
                                        ModuleExportName::Str(s) => {}
                                    },
                                }
                            }
                        }
                    }
                    ModuleDecl::ExportDefaultDecl(_) => {
                        exports.push((js_word!("default"), Default::default()));
                    }
                    ModuleDecl::ExportDefaultExpr(_) => {
                        exports.push((js_word!("default"), Default::default()));
                    }
                    ModuleDecl::ExportAll(_) => {}
                    _ => {}
                }
            }

            match item {
                ModuleItem::ModuleDecl(ModuleDecl::Import(item)) => {
                    // We create multiple items for each import.

                    {
                        // One item for the import itself
                        let id = ItemId {
                            index,
                            kind: ItemIdKind::ImportOfModule,
                        };
                        ids.push(id.clone());
                        items.insert(
                            id,
                            ItemData {
                                is_hoisted: true,
                                side_effects: true,
                                ..Default::default()
                            },
                        );
                    }

                    // One per binding
                    for (si, s) in item.specifiers.iter().enumerate() {
                        let id = ItemId {
                            index,
                            kind: ItemIdKind::ImportBinding(si as u32),
                        };
                        ids.push(id.clone());
                        let local = match s {
                            ImportSpecifier::Named(s) => s.local.to_id(),
                            ImportSpecifier::Default(s) => s.local.to_id(),
                            ImportSpecifier::Namespace(s) => s.local.to_id(),
                        };
                        items.insert(
                            id,
                            ItemData {
                                is_hoisted: true,
                                var_decls: vec![local],
                                ..Default::default()
                            },
                        );
                    }

                    continue;
                }
                ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(ExportDecl {
                    decl: Decl::Fn(f),
                    ..
                }))
                | ModuleItem::Stmt(Stmt::Decl(Decl::Fn(f))) => {
                    let id = ItemId {
                        index,
                        kind: ItemIdKind::Normal,
                    };
                    ids.push(id.clone());

                    let (read_vars, write_vars) = ids_used_by(&f.function);
                    items.insert(
                        id,
                        ItemData {
                            is_hoisted: true,
                            eventual_read_vars: read_vars,
                            eventual_write_vars: write_vars,
                            var_decls: vec![f.ident.to_id()],
                            ..Default::default()
                        },
                    );
                    continue;
                }
                ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(ExportDecl {
                    decl: Decl::Var(v),
                    ..
                }))
                | ModuleItem::Stmt(Stmt::Decl(Decl::Var(v))) => {
                    for (i, decl) in v.decls.iter().enumerate() {
                        let id = ItemId {
                            index,
                            kind: ItemIdKind::VarDeclarator(i as _),
                        };
                        ids.push(id.clone());

                        let decl_ids = find_pat_ids(&decl.name);
                        let (r, w) = ids_used_by_ignoring_nested(&decl.init);
                        let (er, ew) = ids_captured_by(&decl.init);
                        items.insert(
                            id,
                            ItemData {
                                var_decls: decl_ids.clone(),
                                read_vars: r,
                                eventual_read_vars: er,
                                write_vars: decl_ids.into_iter().chain(w).collect(),
                                eventual_write_vars: ew,
                                ..Default::default()
                            },
                        );
                    }

                    continue;
                }

                ModuleItem::Stmt(Stmt::Expr(ExprStmt {
                    expr: box Expr::Assign(assign),
                    ..
                })) => {
                    let mut used_ids = ids_used_by_ignoring_nested(item);
                    let captured_ids = ids_captured_by(item);

                    if assign.op != op!("=") {
                        let extra_ids = ids_used_by_ignoring_nested(&assign.left);
                        used_ids.0.extend(extra_ids.0);
                        used_ids.0.extend(extra_ids.1);
                    }

                    let data = ItemData {
                        read_vars: used_ids.0,
                        eventual_read_vars: captured_ids.0,
                        write_vars: used_ids.1,
                        eventual_write_vars: captured_ids.1,
                        ..Default::default()
                    };

                    let id = ItemId {
                        index,
                        kind: ItemIdKind::Normal,
                    };
                    ids.push(id.clone());
                    items.insert(id, data);
                    continue;
                }
                _ => {}
            }

            // Default to normal

            let used_ids = ids_used_by_ignoring_nested(item);
            let captured_ids = ids_captured_by(item);
            let data = ItemData {
                read_vars: used_ids.0,
                eventual_read_vars: captured_ids.0,
                write_vars: used_ids.1,
                eventual_write_vars: captured_ids.1,
                side_effects: true,
                ..Default::default()
            };

            let id = ItemId {
                index,
                kind: ItemIdKind::Normal,
            };
            ids.push(id.clone());
            items.insert(id, data);
        }

        {
            // `module evaluation side effects` Node
            let id = ItemId {
                index: usize::MAX,
                kind: ItemIdKind::ModuleEvaluation,
            };
            ids.push(id.clone());
            items.insert(
                id,
                ItemData {
                    ..Default::default()
                },
            );
        }

        for export in exports {
            let id = ItemId {
                index: usize::MAX,
                kind: ItemIdKind::Export(export),
            };
            ids.push(id.clone());
            items.insert(
                id,
                ItemData {
                    ..Default::default()
                },
            );
        }

        (ids, items)
    }

    pub(super) fn add_strong_dep(&mut self, item: &ItemId, dep: &ItemId) {
        let from = self.node(item);
        let to = self.node(dep);

        self.inner.add_edge(from, to, true);
    }

    pub(super) fn add_weak_dep(&mut self, item: &ItemId, dep: &ItemId) {
        let from = self.node(item);
        let to = self.node(dep);

        if let Some(true) = self.inner.edge_weight(from, to) {
            return;
        }
        self.inner.add_edge(from, to, false);
    }
}
