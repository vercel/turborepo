use std::{fmt, hash::Hash};

use fxhash::{FxBuildHasher, FxHashMap, FxHashSet};
use indexmap::IndexSet;
use petgraph::{
    algo::{has_path_connecting, kosaraju_scc},
    prelude::DiGraphMap,
};
use swc_core::{
    common::{util::take::Take, Spanned, DUMMY_SP},
    ecma::{
        ast::{
            op, ClassDecl, Decl, ExportDecl, ExportNamedSpecifier, ExportSpecifier, Expr, ExprStmt,
            FnDecl, Id, Ident, ImportDecl, ImportSpecifier, KeyValueProp, Module, ModuleDecl,
            ModuleExportName, ModuleItem, NamedExport, ObjectLit, Prop, PropName, PropOrSpread,
            Stmt, VarDecl,
        },
        atoms::{js_word, JsWord},
        utils::find_pat_ids,
    },
};

use super::util::{ids_captured_by, ids_used_by, ids_used_by_ignoring_nested};

/// The id of an item
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) struct ItemId {
    /// The index of the module item in the module.
    pub index: usize,
    pub kind: ItemIdKind,
}

impl fmt::Debug for ItemId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.index == usize::MAX {
            return write!(f, "ItemId({:?})", self.kind);
        }

        write!(f, "ItemId({}, {:?})", self.index, self.kind)
    }
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
#[derive(Debug)]
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

    pub content: ModuleItem,
}

impl Default for ItemData {
    fn default() -> Self {
        Self {
            is_hoisted: Default::default(),
            var_decls: Default::default(),
            read_vars: Default::default(),
            eventual_read_vars: Default::default(),
            write_vars: Default::default(),
            eventual_write_vars: Default::default(),
            side_effects: Default::default(),
            content: ModuleItem::dummy(),
        }
    }
}

#[derive(Debug)]
pub(super) struct VarInfo {}

#[derive(Debug, Clone)]
pub struct InternedGraph<T>
where
    T: Eq + Hash + Clone,
{
    /// `bool`: Strong
    pub(super) idx_graph: DiGraphMap<u32, bool>,
    pub(super) graph_ix: IndexSet<T, FxBuildHasher>,
}

impl<T> Default for InternedGraph<T>
where
    T: Eq + Hash + Clone,
{
    fn default() -> Self {
        Self {
            idx_graph: Default::default(),
            graph_ix: Default::default(),
        }
    }
}

impl<T> InternedGraph<T>
where
    T: Eq + Hash + Clone,
{
    pub(super) fn node(&mut self, id: &T) -> u32 {
        self.graph_ix.get_index_of(id).unwrap_or_else(|| {
            let ix = self.graph_ix.len();
            self.graph_ix.insert_full(id.clone());
            ix
        }) as _
    }

    /// Panics if `id` is not found.
    pub(super) fn get_node(&self, id: &T) -> u32 {
        self.graph_ix.get_index_of(id).unwrap() as _
    }

    pub(super) fn map<N, F>(self, mut map: F) -> InternedGraph<N>
    where
        N: Clone + Eq + Hash,
        F: FnMut(T) -> N,
    {
        let ix = self.graph_ix.into_iter().map(|v| map(v)).collect();
        InternedGraph {
            idx_graph: self.idx_graph,
            graph_ix: ix,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct DepGraph {
    pub(super) g: InternedGraph<ItemId>,
}

impl DepGraph {
    /// Weak imports are imports only if it's is referenced strongly. But this
    /// is production-only, and week dependencies are treated as strong
    /// dependency in development mode.
    pub(super) fn handle_weak(&mut self, is_development: bool) {
        if is_development {
        } else {
            for start in self.g.graph_ix.iter() {
                let start = self.g.get_node(start);
                for end in self.g.graph_ix.iter() {
                    let end = self.g.get_node(end);

                    if let Some(false) = self.g.idx_graph.edge_weight(start, end) {
                        self.g.idx_graph.remove_edge(start, end);
                    }
                }
            }
        }
    }

    /// Note: ESM imports are immutable, but we does not handle it.
    pub(super) fn split_module(
        &self,
        uri_of_module: &JsWord,
        data: &FxHashMap<ItemId, ItemData>,
    ) -> Vec<Module> {
        let groups = self.finalize();

        let mut modules = vec![];

        for (ix, group) in groups.graph_ix.iter().enumerate() {
            let mut chunk = Module {
                span: DUMMY_SP,
                body: vec![],
                shebang: None,
            };

            for dep in groups
                .idx_graph
                .neighbors_directed(ix as u32, petgraph::Direction::Outgoing)
            {
                let mut specifiers = vec![];

                chunk
                    .body
                    .push(ModuleItem::ModuleDecl(ModuleDecl::Import(ImportDecl {
                        span: DUMMY_SP,
                        specifiers,
                        src: box uri_of_module.clone().into(),
                        type_only: false,
                        asserts: Some(box ObjectLit {
                            span: DUMMY_SP,
                            props: vec![PropOrSpread::Prop(box Prop::KeyValue(KeyValueProp {
                                key: PropName::Ident(Ident::new(
                                    "__turbopack_chunk__".into(),
                                    DUMMY_SP,
                                )),
                                value: (dep as f64).into(),
                            }))],
                        }),
                    })));
            }

            for g in group {
                chunk.body.push(data[g].content.clone());
            }

            modules.push(chunk);
        }

        modules
    }

    pub(super) fn finalize(&self) -> InternedGraph<Vec<ItemId>> {
        /// Returns true if it should be called again
        fn add_to_group(
            graph: &InternedGraph<ItemId>,
            group: &mut Vec<ItemId>,
            start_ix: u32,
            done: &mut FxHashSet<u32>,
        ) -> bool {
            // TODO: Consider cycles
            //

            let mut changed = false;

            // Check deps of `start`.
            for dep_ix in graph
                .idx_graph
                .neighbors_directed(start_ix, petgraph::Direction::Outgoing)
            {
                // Check if the the only dependant of dep is start

                if done.insert(dep_ix) {
                    changed = true;

                    let dep_id = graph.graph_ix.get_index(dep_ix as _).unwrap().clone();
                    group.push(dep_id);

                    add_to_group(graph, group, dep_ix, done);
                }
            }

            changed
        }

        let mut cycles = kosaraju_scc(&self.g.idx_graph);
        cycles.retain(|v| v.len() > 1);

        // If a node have two or more dependants, it should be in a separate
        // group.

        let mut groups = vec![];
        let mut done = FxHashSet::default();

        // Module evaluation node and export nodes starts a group
        for id in self.g.graph_ix.iter() {
            let ix = self.g.get_node(id);

            if id.index == usize::MAX {
                groups.push(vec![id.clone()]);
                done.insert(ix);
                continue;
            }
        }

        // Expand **starting** nodes
        for (ix, id) in self.g.graph_ix.iter().enumerate() {
            // If a node is reachable from two or more nodes, it should be in a
            // separate group.

            if done.contains(&(ix as u32)) {
                continue;
            }

            let count = done
                .iter()
                .filter(|&&staring_point| {
                    has_path_connecting(&self.g.idx_graph, staring_point, ix as _, None)
                })
                .count();

            if count >= 2 {
                groups.push(vec![id.clone()]);
                done.insert(ix as u32);
            }
        }

        //

        loop {
            let mut changed = false;

            for group in &mut groups {
                let start = group[0].clone();
                let start_ix = self.g.get_node(&start);
                if add_to_group(&self.g, group, start_ix, &mut done) {
                    changed = true;
                }
            }

            if !changed {
                break;
            }
        }

        for group in groups.iter_mut() {
            group.sort()
        }

        let mut new_graph = InternedGraph::default();
        let mut group_ix_by_item_ix = FxHashMap::default();

        for group in &groups {
            let group_ix = new_graph.node(group);

            for item in group {
                let item_ix = self.g.get_node(item);
                group_ix_by_item_ix.insert(item_ix, group_ix);
            }
        }

        for group in &groups {
            let group_ix = new_graph.node(group);

            for item in group {
                let item_ix = self.g.get_node(item);

                for item_dep_ix in self
                    .g
                    .idx_graph
                    .neighbors_directed(item_ix, petgraph::Direction::Outgoing)
                {
                    let dep_group_ix = group_ix_by_item_ix.get(&item_dep_ix);
                    if let Some(&dep_group_ix) = dep_group_ix {
                        if group_ix == dep_group_ix {
                            continue;
                        }
                        new_graph.idx_graph.add_edge(group_ix, dep_group_ix, true);
                    }
                }
            }
        }

        new_graph
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
                                content: ModuleItem::ModuleDecl(ModuleDecl::Import(ImportDecl {
                                    specifiers: Default::default(),
                                    ..item.clone()
                                })),
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
                                content: ModuleItem::ModuleDecl(ModuleDecl::Import(ImportDecl {
                                    specifiers: vec![s.clone()],
                                    ..item.clone()
                                })),
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
                            content: item.clone(),
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

                        let var_decl = Box::new(VarDecl {
                            decls: vec![decl.clone()],
                            ..*v.clone()
                        });
                        let content = if item.is_module_decl() {
                            ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(ExportDecl {
                                span: item.span(),
                                decl: Decl::Var(var_decl),
                            }))
                        } else {
                            ModuleItem::Stmt(Stmt::Decl(Decl::Var(var_decl)))
                        };
                        items.insert(
                            id,
                            ItemData {
                                var_decls: decl_ids.clone(),
                                read_vars: r,
                                eventual_read_vars: er,
                                write_vars: decl_ids.into_iter().chain(w).collect(),
                                eventual_write_vars: ew,
                                content,
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
                        content: item.clone(),
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
                content: item.clone(),
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
                    content: ModuleItem::Stmt(Stmt::Expr(ExprStmt {
                        span: DUMMY_SP,
                        expr: "module evaluation".into(),
                    })),
                    ..Default::default()
                },
            );
        }

        for export in exports {
            let id = ItemId {
                index: usize::MAX,
                kind: ItemIdKind::Export(export.clone()),
            };
            ids.push(id.clone());
            items.insert(
                id,
                ItemData {
                    content: ModuleItem::ModuleDecl(ModuleDecl::ExportNamed(NamedExport {
                        span: DUMMY_SP,
                        specifiers: vec![ExportSpecifier::Named(ExportNamedSpecifier {
                            span: DUMMY_SP,
                            orig: ModuleExportName::Ident(export.into()),
                            // TODO
                            exported: None,
                            is_type_only: false,
                        })],
                        src: None,
                        type_only: false,
                        asserts: None,
                    })),
                    ..Default::default()
                },
            );
        }

        (ids, items)
    }

    pub(super) fn add_strong_dep(&mut self, item: &ItemId, dep: &ItemId) {
        let from = self.g.node(item);
        let to = self.g.node(dep);

        self.g.idx_graph.add_edge(from, to, true);
    }

    pub(super) fn add_weak_dep(&mut self, item: &ItemId, dep: &ItemId) {
        let from = self.g.node(item);
        let to = self.g.node(dep);

        if let Some(true) = self.g.idx_graph.edge_weight(from, to) {
            return;
        }
        self.g.idx_graph.add_edge(from, to, false);
    }
}
