use anyhow::{bail, Result};
use indexmap::IndexSet;
use rustc_hash::FxHashMap;
use swc_core::{
    base::SwcComments,
    ecma::ast::{Id, Module, Program},
};
use turbo_tasks_fs::FileSystemPathVc;
use turbopack_core::resolve::{origin::ResolveOrigin, ModulePart, ModulePartVc};

use self::graph::{DepGraph, ItemData, ItemId, ItemIdGroupKind, Mode};
use crate::{
    analyzer::graph::EvalContext,
    parse::{ParseResult, ParseResultVc},
    EcmascriptModuleAssetVc,
};

pub mod asset;
pub mod chunk_item;
mod graph;
pub mod merge;
#[cfg(test)]
mod tests;
mod util;

pub struct Analyzer<'a> {
    g: &'a mut DepGraph,
    item_ids: &'a Vec<ItemId>,
    items: &'a mut FxHashMap<ItemId, ItemData>,

    last_side_effects: Vec<ItemId>,

    vars: FxHashMap<Id, VarState>,
}

#[derive(Debug, Default)]
struct VarState {
    /// The module items that might trigger side effects on that variable.
    /// We also store if this is a `const` write, so no further change will
    /// happen to this var.
    last_writes: Vec<ItemId>,
    /// The module items that might read that variable.
    last_reads: Vec<ItemId>,
}

impl Analyzer<'_> {
    pub(super) fn analyze(module: &Module) -> (DepGraph, FxHashMap<ItemId, ItemData>) {
        let mut g = DepGraph::default();
        let (item_ids, mut items) = g.init(module);

        let mut analyzer = Analyzer {
            g: &mut g,
            item_ids: &item_ids,
            items: &mut items,
            last_side_effects: Default::default(),
            vars: Default::default(),
        };

        let eventual_ids = analyzer.hoist_vars_and_bindings(module);

        analyzer.evaluate_immediate(module, &eventual_ids);

        analyzer.evaluate_eventual(module);

        analyzer.handle_exports(module);

        (g, items)
    }

    /// Phase 1: Hoisted Variables and Bindings
    ///
    ///
    /// Returns all (EVENTUAL_READ/WRITE_VARS) in the module.
    fn hoist_vars_and_bindings(&mut self, _module: &Module) -> IndexSet<Id> {
        let mut eventual_ids = IndexSet::default();

        for item_id in self.item_ids.iter() {
            if let Some(item) = self.items.get(item_id) {
                eventual_ids.extend(item.eventual_read_vars.iter().cloned());
                eventual_ids.extend(item.eventual_write_vars.iter().cloned());

                if item.is_hoisted && item.side_effects {
                    self.g
                        .add_strong_deps(item_id, self.last_side_effects.iter());

                    self.last_side_effects.push(item_id.clone());
                }

                for id in item.var_decls.iter() {
                    let state = self.vars.entry(id.clone()).or_default();

                    if item.is_hoisted {
                        state.last_writes.push(item_id.clone());
                    } else {
                        // TODO(WEB-705): Create a fake module item
                        // state.last_writes.push(item_id.clone());
                    }
                }
            }
        }

        eventual_ids
    }

    /// Phase 2: Immediate evaluation
    fn evaluate_immediate(&mut self, _module: &Module, eventual_ids: &IndexSet<Id>) {
        for item_id in self.item_ids.iter() {
            if let Some(item) = self.items.get(item_id) {
                // Ignore HOISTED module items, they have been processed in phase 1 already.
                if item.is_hoisted {
                    continue;
                }

                let mut items_to_remove_from_last_reads = FxHashMap::<_, Vec<_>>::default();

                // For each var in READ_VARS:
                for id in item.read_vars.iter() {
                    // Create a strong dependency to all module items listed in LAST_WRITES for that
                    // var.

                    // (the writes need to be executed before this read)
                    if let Some(state) = self.vars.get(id) {
                        self.g.add_strong_deps(item_id, state.last_writes.iter());

                        for last_write in state.last_writes.iter() {
                            items_to_remove_from_last_reads
                                .entry(id.clone())
                                .or_default()
                                .push(last_write.clone());
                        }
                    }
                }

                // For each var in WRITE_VARS:
                for id in item.write_vars.iter() {
                    // Create a weak dependency to all module items listed in
                    // LAST_READS for that var.

                    // (the reads need to be executed before this write, when
                    // it’s needed)

                    if let Some(state) = self.vars.get(id) {
                        self.g.add_weak_deps(item_id, state.last_reads.iter());
                    }
                }

                if item.side_effects {
                    // Create a strong dependency to LAST_SIDE_EFFECT.

                    self.g
                        .add_strong_deps(item_id, self.last_side_effects.iter());

                    // Create weak dependencies to all LAST_WRITES and
                    // LAST_READS.
                    for id in eventual_ids.iter() {
                        if let Some(state) = self.vars.get(id) {
                            self.g.add_weak_deps(item_id, state.last_writes.iter());
                            self.g.add_weak_deps(item_id, state.last_reads.iter());
                        }
                    }
                }

                // For each var in WRITE_VARS:
                for id in item.write_vars.iter() {
                    // Add this module item to LAST_WRITES

                    let state = self.vars.entry(id.clone()).or_default();
                    state.last_writes.push(item_id.clone());

                    // TODO: Optimization: Remove each module item to which we
                    // just created a strong dependency from LAST_WRITES
                }

                // For each var in READ_VARS:
                for id in item.read_vars.iter() {
                    // Add this module item to LAST_READS

                    let state = self.vars.entry(id.clone()).or_default();
                    state.last_reads.push(item_id.clone());

                    // Optimization: Remove each module item to which we
                    // just created a strong dependency from LAST_READS

                    if let Some(items) = items_to_remove_from_last_reads.get(id) {
                        for item in items {
                            if let Some(pos) = state.last_reads.iter().position(|v| *v == *item) {
                                state.last_reads.remove(pos);
                            }
                        }
                    }
                }

                if item.side_effects {
                    self.last_side_effects.push(item_id.clone());
                }
            }
        }
    }

    /// Phase 3: Eventual evaluation
    fn evaluate_eventual(&mut self, _module: &Module) {
        for item_id in self.item_ids.iter() {
            if let Some(item) = self.items.get(item_id) {
                // For each var in EVENTUAL_READ_VARS:

                for id in item.eventual_read_vars.iter() {
                    // Create a strong dependency to all module items listed in
                    // LAST_WRITES for that var.

                    if let Some(state) = self.vars.get(id) {
                        self.g.add_strong_deps(item_id, state.last_writes.iter());
                    }
                }

                // For each var in EVENTUAL_WRITE_VARS:
                for id in item.eventual_write_vars.iter() {
                    // Create a weak dependency to all module items listed in
                    // LAST_READS for that var.

                    if let Some(state) = self.vars.get(id) {
                        self.g.add_weak_deps(item_id, state.last_reads.iter());
                    }
                }

                // (no state update happens, since this is only triggered by
                // side effects, which we already handled)
            }
        }
    }

    /// Phase 4: Exports
    fn handle_exports(&mut self, _module: &Module) {
        for item_id in self.item_ids.iter() {
            if let ItemId::Group(kind) = item_id {
                match kind {
                    ItemIdGroupKind::ModuleEvaluation => {
                        // Create a strong dependency to LAST_SIDE_EFFECTS

                        self.g
                            .add_strong_deps(item_id, self.last_side_effects.iter());
                    }
                    ItemIdGroupKind::Export(id) => {
                        // Create a strong dependency to LAST_WRITES for this var

                        if let Some(state) = self.vars.get(id) {
                            self.g.add_strong_deps(item_id, state.last_writes.iter());
                        }
                    }
                }
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum Key {
    ModuleEvaluation,
    Export(String),
}

/// Converts [ModulePartVc] to the index.
async fn get_part_id(result: &SplitResult, part: ModulePartVc) -> Result<u32> {
    let part = part.await?;

    let key = match &*part {
        ModulePart::ModuleEvaluation => Key::ModuleEvaluation,
        ModulePart::Export(export) => Key::Export(export.await?.to_string()),
        ModulePart::Internal(part_id) => return Ok(*part_id),
    };

    // If 'split' fails, it stores an empty value in result.data and this match will
    // fail.
    let part_id = match result.data.get(&key) {
        Some(id) => *id,
        None => {
            return Err(anyhow::anyhow!(
                "could not find part id for module part {:?}",
                key
            ))
        }
    };

    Ok(part_id)
}

#[turbo_tasks::value(shared, serialization = "none", eq = "manual")]
pub(crate) enum SplitResult {
    Ok {
        #[turbo_tasks(debug_ignore, trace_ignore)]
        data: FxHashMap<Key, u32>,

        #[turbo_tasks(debug_ignore, trace_ignore)]
        modules: Vec<Module>,

        #[turbo_tasks(debug_ignore, trace_ignore)]
        deps: FxHashMap<u32, Vec<u32>>,

        /// This field is required to implement part_of_module, which produces
        /// [ParseResultVc]
        parsed: ParseResultVc,
    },
    Unparseable,
    NotFound,
}

impl PartialEq for SplitResult {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Ok { .. }, Self::Ok { .. }) => false,
            _ => core::mem::discriminant(self) == core::mem::discriminant(other),
        }
    }
}

#[turbo_tasks::function]
pub(super) fn split_module(asset: EcmascriptModuleAssetVc) -> SplitResultVc {
    split(asset.origin_path(), asset.parse())
}

#[turbo_tasks::function]
pub(super) async fn split(path: FileSystemPathVc, parsed: ParseResultVc) -> Result<SplitResultVc> {
    let filename = path.await?.file_name().to_string();
    let parse_result = parsed.await?;

    match &*parse_result {
        ParseResult::Ok {
            program: Program::Module(module),
            ..
        } => {
            let (mut dep_graph, items) = Analyzer::analyze(module);

            dep_graph.handle_weak(Mode::Production);

            let (data, deps, modules) =
                dep_graph.split_module(&format!("./{filename}").into(), &items);

            Ok(SplitResult::Ok {
                data,
                deps,
                modules,
                parsed,
            }
            .cell())
        }
        ParseResult::NotFound => Ok(SplitResult::NotFound.cell()),
        ParseResult::Unparseable { .. } => Ok(SplitResult::Unparseable.cell()),
    }
}

#[turbo_tasks::function]
pub(super) async fn part_of_module(
    split_data: SplitResultVc,
    part: Option<ModulePartVc>,
) -> Result<ParseResultVc> {
    let split_data = split_data.await?;

    let part_id = match part {
        Some(part) => get_part_id(&split_data, part).await?,
        None => bail!("part {:?} is not found in the module", part),
    };

    match &*split_data {
        SplitResult::Ok {
            modules, parsed, ..
        } => match &*parsed.await? {
            ParseResult::Ok {
                comments,
                eval_context,
                source_map,
                globals,
                ..
            } => {
                let program = Program::Module(modules[part_id as usize].clone());
                let eval_context = EvalContext::new(&program, eval_context.unresolved_mark);

                Ok(ParseResultVc::cell(ParseResult::Ok {
                    program,
                    globals: globals.clone(),
                    comments: comments.clone(),
                    source_map: source_map.clone(),
                    eval_context,
                }))
            }
            ParseResult::Unparseable => bail!("module is unparseable"),
            ParseResult::NotFound => bail!("module is not found"),
        },
        SplitResult::Unparseable => Ok(ParseResult::Unparseable),
        SplitResult::NotFound => Ok(ParseResult::NotFound),
    }
}
