use std::{borrow::Cow, collections::VecDeque, sync::Arc};

use anyhow::{bail, Result};
use indexmap::IndexMap;
use swc_core::{
    common::DUMMY_SP,
    ecma::{
        ast::{
            Expr, ExprStmt, KeyValueProp, Lit, ModuleItem, ObjectLit, Prop, PropName, PropOrSpread,
            Stmt, {self},
        },
        codegen::{text_writer::JsWriter, Emitter},
    },
    quote, quote_expr,
};
use turbo_tasks::{primitives::Regex, Value, ValueToString, Vc};
use turbo_tasks_fs::{DirectoryContent, DirectoryEntry, FileSystemPath};
use turbopack_core::{
    asset::{Asset, AssetContent},
    chunk::{
        ChunkItem, ChunkItemExt, ChunkType, ChunkableModule, ChunkableModuleReference,
        ChunkingContext,
    },
    ident::AssetIdent,
    issue::{IssueSeverity, IssueSource},
    module::Module,
    reference::{ModuleReference, ModuleReferences},
    resolve::{origin::ResolveOrigin, parse::Request, ModuleResolveResult},
    source::Source,
};
use turbopack_resolve::ecmascript::{cjs_resolve, try_to_severity};

use crate::{
    chunk::{
        EcmascriptChunkItem, EcmascriptChunkItemContent, EcmascriptChunkType, EcmascriptExports,
    },
    code_gen::CodeGeneration,
    create_visitor,
    references::{
        pattern_mapping::{PatternMapping, ResolveType},
        AstPath,
    },
    utils::module_id_to_lit,
    CodeGenerateable, EcmascriptChunkPlaceable,
};

#[turbo_tasks::value]
#[derive(Debug)]
pub(crate) enum DirListEntry {
    File(Vc<FileSystemPath>),
    Dir(Vc<DirList>),
}

#[turbo_tasks::value(transparent)]
pub(crate) struct DirList(IndexMap<String, DirListEntry>);

#[turbo_tasks::value_impl]
impl DirList {
    #[turbo_tasks::function]
    pub(crate) fn read(dir: Vc<FileSystemPath>, recursive: bool, filter: Vc<Regex>) -> Vc<Self> {
        Self::read_internal(dir, dir, recursive, filter)
    }

    #[turbo_tasks::function]
    pub(crate) async fn read_internal(
        root: Vc<FileSystemPath>,
        dir: Vc<FileSystemPath>,
        recursive: bool,
        filter: Vc<Regex>,
    ) -> Result<Vc<Self>> {
        let root_val = &*dir.await?;
        let regex = &*filter.await?;

        let mut list = IndexMap::new();

        let dir_content = dir.read_dir().await?;
        let entries = match &*dir_content {
            DirectoryContent::Entries(entries) => Some(entries),
            DirectoryContent::NotFound => None,
        };

        for (_, entry) in entries.iter().flat_map(|m| m.iter()) {
            match entry {
                DirectoryEntry::File(path) => {
                    if let Some(relative_path) = root_val.get_relative_path_to(&*path.await?) {
                        if regex.is_match(&relative_path) {
                            list.insert(relative_path, DirListEntry::File(*path));
                        }
                    }
                }
                DirectoryEntry::Directory(path) if recursive => {
                    if let Some(relative_path) = root_val.get_relative_path_to(&*path.await?) {
                        list.insert(
                            relative_path,
                            DirListEntry::Dir(DirList::read_internal(
                                root, *path, recursive, filter,
                            )),
                        );
                    }
                }
                // ignore everything else
                _ => {}
            }
        }

        list.sort_keys();

        Ok(Vc::cell(list))
    }

    #[turbo_tasks::function]
    async fn flatten(self: Vc<Self>) -> Result<Vc<FlatDirList>> {
        let this = self.await?;

        let mut queue = VecDeque::from([this]);

        let mut list = IndexMap::new();

        while let Some(dir) = queue.pop_front() {
            for (k, entry) in &*dir {
                match entry {
                    DirListEntry::File(path) => {
                        list.insert(k.clone(), *path);
                    }
                    DirListEntry::Dir(d) => {
                        queue.push_back(d.await?);
                    }
                }
            }
        }

        Ok(Vc::cell(list))
    }
}

#[turbo_tasks::value(transparent)]
pub(crate) struct FlatDirList(IndexMap<String, Vc<FileSystemPath>>);

#[turbo_tasks::value_impl]
impl FlatDirList {
    #[turbo_tasks::function]
    pub(crate) fn read(dir: Vc<FileSystemPath>, recursive: bool, filter: Vc<Regex>) -> Vc<Self> {
        DirList::read(dir, recursive, filter).flatten()
    }
}

#[turbo_tasks::value]
#[derive(Debug)]
pub struct RequireContextMapEntry {
    pub origin_relative: String,
    pub request: Vc<Request>,
    pub result: Vc<ModuleResolveResult>,
}

/// The resolved context map for a `require.context(..)` call.
#[turbo_tasks::value(transparent)]
pub struct RequireContextMap(IndexMap<String, RequireContextMapEntry>);

#[turbo_tasks::value_impl]
impl RequireContextMap {
    #[turbo_tasks::function]
    pub(crate) async fn generate(
        origin: Vc<Box<dyn ResolveOrigin>>,
        dir: Vc<FileSystemPath>,
        recursive: bool,
        filter: Vc<Regex>,
        issue_source: Option<Vc<IssueSource>>,
        issue_severity: Vc<IssueSeverity>,
    ) -> Result<Vc<Self>> {
        let origin_path = &*origin.origin_path().parent().await?;

        let list = &*FlatDirList::read(dir, recursive, filter).await?;

        let mut map = IndexMap::new();

        for (context_relative, path) in list {
            if let Some(origin_relative) = origin_path.get_relative_path_to(&*path.await?) {
                let request = Request::parse(Value::new(origin_relative.clone().into()));
                let result = cjs_resolve(origin, request, issue_source, issue_severity);

                map.insert(
                    context_relative.clone(),
                    RequireContextMapEntry {
                        origin_relative,
                        request,
                        result,
                    },
                );
            } else {
                bail!("invariant error: this was already checked in `list_dir`");
            }
        }

        Ok(Vc::cell(map))
    }
}

/// A reference for `require.context()`, will replace it with an inlined map
/// wrapped in `__turbopack_module_context__`;
#[turbo_tasks::value]
#[derive(Hash, Debug)]
pub struct RequireContextAssetReference {
    pub inner: Vc<RequireContextAsset>,
    pub dir: String,
    pub include_subdirs: bool,

    pub path: Vc<AstPath>,
    pub issue_source: Option<Vc<IssueSource>>,
    pub in_try: bool,
}

#[turbo_tasks::value_impl]
impl RequireContextAssetReference {
    #[turbo_tasks::function]
    pub fn new(
        source: Vc<Box<dyn Source>>,
        origin: Vc<Box<dyn ResolveOrigin>>,
        dir: String,
        include_subdirs: bool,
        filter: Vc<Regex>,
        path: Vc<AstPath>,
        issue_source: Option<Vc<IssueSource>>,
        in_try: bool,
    ) -> Vc<Self> {
        let map = RequireContextMap::generate(
            origin,
            origin.origin_path().parent().join(dir.clone()),
            include_subdirs,
            filter,
            issue_source,
            try_to_severity(in_try),
        );
        let inner = RequireContextAsset {
            source,
            origin,
            map,

            dir: dir.clone(),
            include_subdirs,
        }
        .cell();

        Self::cell(RequireContextAssetReference {
            inner,
            dir,
            include_subdirs,
            path,
            issue_source,
            in_try,
        })
    }
}

#[turbo_tasks::value_impl]
impl ModuleReference for RequireContextAssetReference {
    #[turbo_tasks::function]
    fn resolve_reference(&self) -> Vc<ModuleResolveResult> {
        ModuleResolveResult::module(Vc::upcast(self.inner)).cell()
    }
}

#[turbo_tasks::value_impl]
impl ValueToString for RequireContextAssetReference {
    #[turbo_tasks::function]
    async fn to_string(&self) -> Result<Vc<String>> {
        Ok(Vc::cell(format!(
            "require.context {}/{}",
            self.dir,
            if self.include_subdirs { "**" } else { "*" },
        )))
    }
}

#[turbo_tasks::value_impl]
impl ChunkableModuleReference for RequireContextAssetReference {}

#[turbo_tasks::value_impl]
impl CodeGenerateable for RequireContextAssetReference {
    #[turbo_tasks::function]
    async fn code_generation(
        &self,
        chunking_context: Vc<Box<dyn ChunkingContext>>,
    ) -> Result<Vc<CodeGeneration>> {
        let chunk_item = self.inner.as_chunk_item(Vc::upcast(chunking_context));
        let module_id = chunk_item.id().await?.clone_value();

        let mut visitors = Vec::new();

        let path = &self.path.await?;
        visitors.push(create_visitor!(path, visit_mut_expr(expr: &mut Expr) {
            if let Expr::Call(_) = expr {
                *expr = quote!(
                    "__turbopack_module_context__(__turbopack_require__($id))" as Expr,
                    id: Expr = module_id_to_lit(&module_id)
                );
            }
        }));

        Ok(CodeGeneration { visitors }.into())
    }
}

#[turbo_tasks::value(transparent)]
pub struct ResolvedModuleReference(Vc<ModuleResolveResult>);

#[turbo_tasks::value_impl]
impl ModuleReference for ResolvedModuleReference {
    #[turbo_tasks::function]
    fn resolve_reference(&self) -> Vc<ModuleResolveResult> {
        self.0
    }
}

#[turbo_tasks::value_impl]
impl ValueToString for ResolvedModuleReference {
    #[turbo_tasks::function]
    async fn to_string(&self) -> Result<Vc<String>> {
        Ok(Vc::cell("resolved reference".to_string()))
    }
}

#[turbo_tasks::value_impl]
impl ChunkableModuleReference for ResolvedModuleReference {}

#[turbo_tasks::value]
pub struct RequireContextAsset {
    source: Vc<Box<dyn Source>>,

    origin: Vc<Box<dyn ResolveOrigin>>,
    map: Vc<RequireContextMap>,

    dir: String,
    include_subdirs: bool,
}

#[turbo_tasks::function]
fn modifier(dir: String, include_subdirs: bool) -> Vc<String> {
    Vc::cell(format!(
        "require.context {}/{}",
        dir,
        if include_subdirs { "**" } else { "*" },
    ))
}

#[turbo_tasks::value_impl]
impl Module for RequireContextAsset {
    #[turbo_tasks::function]
    fn ident(&self) -> Vc<AssetIdent> {
        self.source
            .ident()
            .with_modifier(modifier(self.dir.clone(), self.include_subdirs))
    }

    #[turbo_tasks::function]
    async fn references(&self) -> Result<Vc<ModuleReferences>> {
        let map = &*self.map.await?;

        Ok(Vc::cell(
            map.iter()
                .map(|(_, entry)| Vc::upcast(Vc::<ResolvedModuleReference>::cell(entry.result)))
                .collect(),
        ))
    }
}

#[turbo_tasks::value_impl]
impl Asset for RequireContextAsset {
    #[turbo_tasks::function]
    fn content(&self) -> Vc<AssetContent> {
        unimplemented!()
    }
}

#[turbo_tasks::value_impl]
impl ChunkableModule for RequireContextAsset {
    #[turbo_tasks::function]
    async fn as_chunk_item(
        self: Vc<Self>,
        chunking_context: Vc<Box<dyn ChunkingContext>>,
    ) -> Result<Vc<Box<dyn turbopack_core::chunk::ChunkItem>>> {
        let this = self.await?;
        Ok(Vc::upcast(
            RequireContextChunkItem {
                chunking_context,
                inner: self,

                origin: this.origin,
                map: this.map,
            }
            .cell(),
        ))
    }
}

#[turbo_tasks::value_impl]
impl EcmascriptChunkPlaceable for RequireContextAsset {
    #[turbo_tasks::function]
    fn get_exports(&self) -> Vc<EcmascriptExports> {
        EcmascriptExports::Value.cell()
    }
}

#[turbo_tasks::value]
pub struct RequireContextChunkItem {
    chunking_context: Vc<Box<dyn ChunkingContext>>,
    inner: Vc<RequireContextAsset>,

    origin: Vc<Box<dyn ResolveOrigin>>,
    map: Vc<RequireContextMap>,
}

#[turbo_tasks::value_impl]
impl EcmascriptChunkItem for RequireContextChunkItem {
    #[turbo_tasks::function]
    fn chunking_context(&self) -> Vc<Box<dyn ChunkingContext>> {
        self.chunking_context
    }

    #[turbo_tasks::function]
    async fn content(&self) -> Result<Vc<EcmascriptChunkItemContent>> {
        let map = &*self.map.await?;

        let mut context_map = ObjectLit {
            span: DUMMY_SP,
            props: vec![],
        };

        for (key, entry) in map {
            let pm = PatternMapping::resolve_request(
                entry.request,
                self.origin,
                Vc::upcast(self.chunking_context),
                entry.result,
                Value::new(ResolveType::ChunkItem),
            )
            .await?;

            let PatternMapping::Single(pm) = &*pm else {
                continue;
            };

            let key_expr = Expr::Lit(Lit::Str(entry.origin_relative.as_str().into()));

            let prop = KeyValueProp {
                key: PropName::Str(key.as_str().into()),
                value: quote_expr!(
                    "{ id: () => $id, module: () => $module }",
                    id: Expr =
                        pm.create_id(Cow::Borrowed(&key_expr)),
                    module: Expr =
                        pm.create_require(Cow::Borrowed(&key_expr)),
                ),
            };

            context_map
                .props
                .push(PropOrSpread::Prop(Box::new(Prop::KeyValue(prop))));
        }

        let expr = quote_expr!(
            "__turbopack_export_value__($obj);",
            obj: Expr = Expr::Object(context_map),
        );

        let module = ast::Module {
            span: DUMMY_SP,
            body: vec![ModuleItem::Stmt(Stmt::Expr(ExprStmt {
                span: DUMMY_SP,
                expr,
            }))],
            shebang: None,
        };

        let source_map: Arc<swc_core::common::SourceMap> = Default::default();
        let mut bytes: Vec<u8> = vec![];
        let mut emitter = Emitter {
            cfg: swc_core::ecma::codegen::Config::default(),
            cm: source_map.clone(),
            comments: None,
            wr: JsWriter::new(source_map, "\n", &mut bytes, None),
        };

        emitter.emit_module(&module)?;

        Ok(EcmascriptChunkItemContent {
            inner_code: bytes.into(),
            ..Default::default()
        }
        .cell())
    }
}

#[turbo_tasks::value_impl]
impl ChunkItem for RequireContextChunkItem {
    #[turbo_tasks::function]
    fn asset_ident(&self) -> Vc<AssetIdent> {
        self.inner.ident()
    }

    #[turbo_tasks::function]
    fn references(&self) -> Vc<ModuleReferences> {
        self.inner.references()
    }

    #[turbo_tasks::function]
    async fn chunking_context(&self) -> Vc<Box<dyn ChunkingContext>> {
        Vc::upcast(self.chunking_context)
    }

    #[turbo_tasks::function]
    async fn ty(&self) -> Result<Vc<Box<dyn ChunkType>>> {
        Ok(Vc::upcast(
            Vc::<EcmascriptChunkType>::default().resolve().await?,
        ))
    }

    #[turbo_tasks::function]
    fn module(&self) -> Vc<Box<dyn Module>> {
        Vc::upcast(self.inner)
    }
}
