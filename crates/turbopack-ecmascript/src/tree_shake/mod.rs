use anyhow::{anyhow, Result};
use fxhash::FxHashMap;
use indexmap::IndexSet;
use swc_core::{
    common::GLOBALS,
    ecma::{
        ast::{Id, Module, Program},
        codegen::{text_writer::JsWriter, Emitter},
        visit::{VisitMutWith, VisitMutWithPath},
    },
};
use turbo_tasks::{primitives::StringVc, TryJoinIterExt, Value, ValueToString, ValueToStringVc};
use turbo_tasks_fs::FileSystemPathVc;
use turbopack_core::{
    asset::{Asset, AssetContentVc, AssetVc},
    chunk::{
        ChunkItem, ChunkItemVc, ChunkVc, ChunkableAsset, ChunkableAssetVc, ChunkingContextVc,
        ModuleId, ModuleIdVc,
    },
    reference::{AssetReferencesVc, SingleAssetReferenceVc},
    resolve::{ModulePart, ModulePartVc},
};

use self::graph::{DepGraph, ItemData, ItemId, ItemIdKind};
use crate::{
    chunk::{
        EcmascriptChunkItem, EcmascriptChunkItemContent, EcmascriptChunkItemContentVc,
        EcmascriptChunkItemOptions, EcmascriptChunkItemVc, EcmascriptChunkPlaceable,
        EcmascriptChunkPlaceableVc, EcmascriptChunkVc, EcmascriptExportsVc,
    },
    code_gen::{CodeGenerateable, CodeGenerateableVc},
    parse::{ParseResult, ParseResultVc},
    path_visitor::ApplyVisitors,
    references::{analyze_ecmascript_module, AnalyzeEcmascriptModuleResult},
    AnalyzeEcmascriptModuleResultVc, EcmascriptModuleAssetVc, ParseResultSourceMap,
};

mod graph;
pub mod merge;
#[cfg(test)]
mod tests;
mod util;

pub struct Analyzer<'a> {
    g: &'a mut DepGraph,
    item_ids: &'a Vec<ItemId>,
    items: &'a mut FxHashMap<ItemId, ItemData>,

    last_side_effect: Option<ItemId>,
    last_side_effects: Vec<ItemId>,

    vars: FxHashMap<Id, VarState>,
}

#[derive(Debug, Default)]
struct VarState {
    /// The module items that might triggered side effects on that variable.
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
            last_side_effect: Default::default(),
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
    fn hoist_vars_and_bindings(&mut self, module: &Module) -> IndexSet<Id> {
        let mut eventual_ids = IndexSet::default();

        for item_id in self.item_ids.iter() {
            if let Some(item) = self.items.get(item_id) {
                eventual_ids.extend(item.eventual_read_vars.iter().cloned());
                eventual_ids.extend(item.eventual_write_vars.iter().cloned());

                if item.is_hoisted && item.side_effects {
                    if let Some(last) = self.last_side_effect.take() {
                        self.g.add_strong_dep(item_id, &last)
                    }

                    self.last_side_effect = Some(item_id.clone());
                    self.last_side_effects.push(item_id.clone());
                }

                for id in item.var_decls.iter() {
                    let state = self.vars.entry(id.clone()).or_default();

                    if item.is_hoisted {
                        state.last_writes.push(item_id.clone());
                    } else {
                        // TODO: Create a fake module item
                        // state.last_writes.push(item_id.clone());
                    }
                }
            }
        }

        eventual_ids
    }

    /// Phase 2: Immediate evaluation
    fn evaluate_immediate(&mut self, module: &Module, eventual_ids: &IndexSet<Id>) {
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

                    // (the write need to be executed before this read)
                    if let Some(state) = self.vars.get(id) {
                        for last_write in state.last_writes.iter() {
                            self.g.add_strong_dep(item_id, last_write);

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

                    // (the read need to be executed before this write, when
                    // it’s needed)

                    if let Some(state) = self.vars.get(id) {
                        for last_read in state.last_reads.iter() {
                            self.g.add_weak_dep(item_id, last_read);
                        }
                    }
                }

                if item.side_effects {
                    // Create a strong dependency to LAST_SIDE_EFFECT.

                    if let Some(last) = &self.last_side_effect {
                        self.g.add_strong_dep(item_id, last);
                    }

                    // Create weak dependencies to all LAST_WRITES and
                    // LAST_READS.
                    for id in eventual_ids.iter() {
                        if let Some(state) = self.vars.get(id) {
                            for last_write in state.last_writes.iter() {
                                self.g.add_weak_dep(item_id, last_write);
                            }

                            for last_read in state.last_reads.iter() {
                                self.g.add_weak_dep(item_id, last_read);
                            }
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
                    self.last_side_effect = Some(item_id.clone());
                    self.last_side_effects.push(item_id.clone());
                }
            }
        }
    }

    /// Phase 3: Eventual evaluation
    fn evaluate_eventual(&mut self, module: &Module) {
        for item_id in self.item_ids.iter() {
            if let Some(item) = self.items.get(item_id) {
                // For each var in EVENTUAL_READ_VARS:

                for id in item.eventual_read_vars.iter() {
                    // Create a strong dependency to all module items listed in
                    // LAST_WRITES for that var.

                    if let Some(state) = self.vars.get(id) {
                        for last_write in state.last_writes.iter() {
                            self.g.add_strong_dep(item_id, last_write);
                        }
                    }
                }

                // For each var in EVENTUAL_WRITE_VARS:
                for id in item.eventual_write_vars.iter() {
                    // Create a weak dependency to all module items listed in
                    // LAST_READS for that var.

                    if let Some(state) = self.vars.get(id) {
                        for last_read in state.last_reads.iter() {
                            self.g.add_weak_dep(item_id, last_read);
                        }
                    }
                }

                // (no state update happens, since this is only triggered by
                // side effects, which we already handled)
            }
        }
    }

    /// Phase 4: Exports
    fn handle_exports(&mut self, module: &Module) {
        for item_id in self.item_ids.iter() {
            if item_id.index == usize::MAX {
                match &item_id.kind {
                    ItemIdKind::ModuleEvaluation => {
                        // Create a strong dependency to LAST_SIDE_EFFECTS

                        for last in self.last_side_effects.iter() {
                            self.g.add_strong_dep(item_id, last);
                        }

                        // // Create weak dependencies to all LAST_WRITES and
                        // // LAST_READS.

                        // for (.., state) in self.vars.iter() {
                        //     for last_write in state.last_writes.iter() {
                        //         self.g.add_weak_dep(item_id, last_write);
                        //     }

                        //     for last_read in state.last_reads.iter() {
                        //         self.g.add_weak_dep(item_id, last_read);
                        //     }
                        // }
                    }
                    ItemIdKind::Export(id) => {
                        // Create a strong dependency to LAST_WRITES for this var

                        if let Some(state) = self.vars.get(id) {
                            for last_write in state.last_writes.iter() {
                                self.g.add_strong_dep(item_id, last_write);
                            }
                        }
                    }

                    _ => {}
                }
            }
        }
    }
}

#[turbo_tasks::value]
pub struct EcmascriptModulePartAsset {
    full_module: EcmascriptModuleAssetVc,
    split_data: SplitResultVc,
    chunk_id: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum Key {
    ModuleEvaluation,
    Export(String),
}

#[turbo_tasks::value(shared, serialization = "none", eq = "manual")]
pub(crate) struct SplitResult {
    #[turbo_tasks(debug_ignore, trace_ignore)]
    pub data: FxHashMap<Key, u32>,

    #[turbo_tasks(debug_ignore, trace_ignore)]
    pub modules: Vec<Module>,

    #[turbo_tasks(debug_ignore, trace_ignore)]
    pub deps: FxHashMap<u32, Vec<u32>>,
}

impl PartialEq for SplitResult {
    fn eq(&self, other: &Self) -> bool {
        false
    }
}

#[turbo_tasks::function]
pub(super) async fn split(path: FileSystemPathVc, parsed: ParseResultVc) -> Result<SplitResultVc> {
    let filename = path.await?.file_name().to_string();
    let parsed = parsed.await?;

    match &*parsed {
        ParseResult::Ok { program, .. } => {
            if let Program::Module(module) = program {
                let (mut dep_graph, items) = Analyzer::analyze(module);

                dep_graph.handle_weak(true);

                let (data, deps, modules) =
                    dep_graph.split_module(&format!("./{filename}").into(), &items);

                Ok(SplitResult {
                    data,
                    deps,
                    modules,
                }
                .cell())
            } else {
                todo!("handle non-module")
            }
        }
        _ => {
            todo!("handle parse error")
        }
    }
}

impl EcmascriptModulePartAssetVc {
    pub async fn from_splitted(
        module: EcmascriptModuleAssetVc,
        part: ModulePartVc,
    ) -> Result<Self> {
        let split_data = split(module.path(), module.parse());
        let result = split_data.await?;
        let part = part.await?;

        let key = match &*part {
            ModulePart::ModuleEvaluation => Key::ModuleEvaluation,
            ModulePart::Export(export) => Key::Export(export.await?.to_string()),
        };

        let chunk_id = match result.data.get(&key) {
            Some(id) => *id,
            None => return Err(anyhow!("could not find chunk id for module part {:?}", key)),
        };

        Ok(EcmascriptModulePartAsset {
            full_module: module,
            chunk_id,
            split_data,
        }
        .cell())
    }
}

#[turbo_tasks::value_impl]
impl Asset for EcmascriptModulePartAsset {
    #[turbo_tasks::function]
    fn path(&self) -> FileSystemPathVc {
        self.full_module.path()
    }

    #[turbo_tasks::function]
    fn content(&self) -> AssetContentVc {
        todo!()
    }

    #[turbo_tasks::function]
    fn references(&self) -> AssetReferencesVc {
        todo!()
    }
}

#[turbo_tasks::value_impl]
impl EcmascriptChunkPlaceable for EcmascriptModulePartAsset {
    #[turbo_tasks::function]
    async fn as_chunk_item(
        self_vc: EcmascriptModulePartAssetVc,
        context: ChunkingContextVc,
    ) -> Result<EcmascriptChunkItemVc> {
        let s = self_vc.await?;

        Ok(EcmascriptModulePartChunkItem {
            module: self_vc,
            context,
            chunk_id: s.chunk_id,
            full_module: s.full_module,
            split_data: s.split_data,
        }
        .cell()
        .into())
    }

    #[turbo_tasks::function]
    async fn get_exports(self_vc: EcmascriptModuleAssetVc) -> Result<EcmascriptExportsVc> {
        Ok(self_vc.analyze().await?.exports)
    }
}

#[turbo_tasks::value_impl]
impl ChunkableAsset for EcmascriptModulePartAsset {
    #[turbo_tasks::function]
    async fn as_chunk(self_vc: EcmascriptModulePartAssetVc, context: ChunkingContextVc) -> ChunkVc {
        EcmascriptChunkVc::new(context, self_vc.as_ecmascript_chunk_placeable()).into()
    }
}

#[turbo_tasks::value]
pub struct EcmascriptModulePartChunkItem {
    full_module: EcmascriptModuleAssetVc,

    split_data: SplitResultVc,

    module: EcmascriptModulePartAssetVc,
    context: ChunkingContextVc,

    chunk_id: u32,
}

#[turbo_tasks::value_impl]
impl EcmascriptModulePartAssetVc {
    #[turbo_tasks::function]
    async fn analyze(self) -> Result<AnalyzeEcmascriptModuleResultVc> {
        let part = self.await?;
        let this = part.full_module.await?;
        Ok(analyze_ecmascript_module(
            this.source,
            part.full_module.as_resolve_origin(),
            Value::new(this.ty),
            this.transforms,
            this.compile_time_info,
        ))
    }
}

#[turbo_tasks::value_impl]
impl ValueToString for EcmascriptModulePartChunkItem {
    #[turbo_tasks::function]
    async fn to_string(&self) -> Result<StringVc> {
        Ok(StringVc::cell(format!(
            "{} (ecmascript) -> chunk {}",
            self.full_module.await?.source.path().to_string().await?,
            self.chunk_id
        )))
    }
}

#[turbo_tasks::value_impl]
impl EcmascriptChunkItem for EcmascriptModulePartChunkItem {
    #[turbo_tasks::function]
    fn related_path(&self) -> FileSystemPathVc {
        self.module.path()
    }

    #[turbo_tasks::function]
    async fn content(&self) -> Result<EcmascriptChunkItemContentVc> {
        // TODO: Use self.split_data.modules[self.chunk_id] to generate the code
        let split_data = self.split_data.await?;

        let context = self.context;

        let AnalyzeEcmascriptModuleResult {
            references,
            code_generation,
            ..
        } = &*self.full_module.analyze().await?;

        let mut code_gens = Vec::new();
        for r in references.await?.iter() {
            if let Some(code_gen) = CodeGenerateableVc::resolve_from(r).await? {
                code_gens.push(code_gen.code_generation(context));
            }
        }
        for c in code_generation.await?.iter() {
            let c = c.resolve().await?;
            code_gens.push(c.code_generation(context));
        }
        // need to keep that around to allow references into that
        let code_gens = code_gens.into_iter().try_join().await?;
        let code_gens = code_gens.iter().map(|cg| &**cg).collect::<Vec<_>>();
        // TOOD use interval tree with references into "code_gens"
        let mut visitors = Vec::new();
        let mut root_visitors = Vec::new();
        for code_gen in code_gens {
            for (path, visitor) in code_gen.visitors.iter() {
                if path.is_empty() {
                    root_visitors.push(&**visitor);
                } else {
                    visitors.push((path, &**visitor));
                }
            }
        }

        let parsed = self.full_module.parse().await?;

        if let ParseResult::Ok {
            source_map,
            globals,
            eval_context,
            ..
        } = &*parsed
        {
            let mut program = split_data.modules[self.chunk_id as usize].clone();

            GLOBALS.set(globals, || {
                if !visitors.is_empty() {
                    program.visit_mut_with_path(
                        &mut ApplyVisitors::new(visitors),
                        &mut Default::default(),
                    );
                }
                for visitor in root_visitors {
                    program.visit_mut_with(&mut visitor.create());
                }
                program.visit_mut_with(&mut swc_core::ecma::transforms::base::hygiene::hygiene());
                program.visit_mut_with(&mut swc_core::ecma::transforms::base::fixer::fixer(None));

                // we need to remove any shebang before bundling as it's only valid as the first
                // line in a js file (not in a chunk item wrapped in the runtime)
                program.shebang = None;
            });

            let mut bytes: Vec<u8> = vec![];
            // TODO: Insert this as a sourceless segment so that sourcemaps aren't affected.
            // = format!("/* {} */\n", self.module.path().to_string().await?).into_bytes();

            let mut srcmap = vec![];

            let mut emitter = Emitter {
                cfg: swc_core::ecma::codegen::Config {
                    ..Default::default()
                },
                cm: source_map.clone(),
                comments: None,
                wr: JsWriter::new(source_map.clone(), "\n", &mut bytes, Some(&mut srcmap)),
            };

            emitter.emit_module(&program)?;

            let srcmap = ParseResultSourceMap::new(source_map.clone(), srcmap).cell();

            Ok(EcmascriptChunkItemContent {
                inner_code: bytes.into(),
                source_map: Some(srcmap),
                options: if eval_context.is_esm() {
                    EcmascriptChunkItemOptions {
                        ..Default::default()
                    }
                } else {
                    EcmascriptChunkItemOptions {
                        // These things are not available in ESM
                        module: true,
                        exports: true,
                        this: true,
                        ..Default::default()
                    }
                },
                ..Default::default()
            }
            .into())
        } else {
            Ok(EcmascriptChunkItemContent {
                inner_code: format!("__turbopack_wip__({{ wip: true }});",).into(),
                ..Default::default()
            }
            .cell())
        }
    }

    #[turbo_tasks::function]
    fn chunking_context(&self) -> ChunkingContextVc {
        self.context
    }

    #[turbo_tasks::function]
    async fn id(&self) -> Result<ModuleIdVc> {
        let module = self.full_module.path().await?;

        Ok(ModuleId::String(format!("{}_({})", module.path, self.chunk_id)).into())
    }
}

#[turbo_tasks::value_impl]
impl ChunkItem for EcmascriptModulePartChunkItem {
    #[turbo_tasks::function]
    async fn references(&self) -> Result<AssetReferencesVc> {
        let split_data = self.split_data.await?;
        let deps = match split_data.deps.get(&self.chunk_id) {
            Some(v) => v,
            None => return Ok(self.full_module.references()),
        };

        let mut assets = deps
            .iter()
            .map(|&chunk_id| {
                SingleAssetReferenceVc::new(
                    EcmascriptModulePartAssetVc::cell(EcmascriptModulePartAsset {
                        full_module: self.full_module,
                        chunk_id,
                        split_data: self.split_data,
                    })
                    .as_asset(),
                    StringVc::cell("ecmascript module part".to_string()),
                )
                .as_asset_reference()
            })
            .collect::<Vec<_>>();

        let external = self.full_module.references().await?;

        assets.extend(external.iter().cloned());

        Ok(AssetReferencesVc::cell(assets))
    }
}
