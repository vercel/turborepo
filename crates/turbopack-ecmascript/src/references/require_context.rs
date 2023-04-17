use std::{collections::VecDeque, sync::Arc};

use anyhow::{bail, Result};
use indexmap::IndexMap;
use swc_core::{
    common::DUMMY_SP,
    ecma::{
        ast::{
            Expr, ExprStmt, KeyValueProp, Lit, Module, ModuleItem, ObjectLit, Prop, PropName,
            PropOrSpread, Stmt,
        },
        codegen::{text_writer::JsWriter, Emitter},
    },
    quote, quote_expr,
};
use turbo_tasks::{
    primitives::{RegexVc, StringVc},
    Value, ValueToString, ValueToStringVc,
};
use turbo_tasks_fs::{DirectoryContent, DirectoryEntry, FileSystemPathVc};
use turbopack_core::{
    asset::{Asset, AssetContentVc, AssetVc},
    chunk::{
        availability_info::AvailabilityInfo, ChunkItem, ChunkItemVc, ChunkVc, ChunkableAsset,
        ChunkableAssetReference, ChunkableAssetReferenceVc, ChunkableAssetVc, ChunkingContextVc,
    },
    ident::AssetIdentVc,
    issue::{IssueSeverityVc, OptionIssueSourceVc},
    reference::{AssetReference, AssetReferenceVc, AssetReferencesVc},
    resolve::{
        origin::{ResolveOrigin, ResolveOriginVc},
        parse::RequestVc,
        ResolveResult, ResolveResultVc,
    },
};

use crate::{
    chunk::{
        EcmascriptChunkItem, EcmascriptChunkItemContent, EcmascriptChunkItemContentVc,
        EcmascriptChunkItemVc, EcmascriptChunkPlaceable, EcmascriptChunkVc,
        EcmascriptChunkingContextVc, EcmascriptExports, EcmascriptExportsVc,
    },
    chunk_group_files_asset::ChunkGroupFilesAssetVc,
    code_gen::{CodeGenerateable, CodeGeneration, CodeGenerationVc},
    create_visitor,
    references::{
        pattern_mapping::{PatternMappingVc, ResolveType::Cjs},
        AstPathVc,
    },
    resolve::{cjs_resolve, try_to_severity},
    utils::module_id_to_lit,
    CodeGenerateableVc, EcmascriptChunkPlaceableVc,
};

#[turbo_tasks::value]
#[derive(Debug)]
pub(crate) enum DirListEntry {
    File(FileSystemPathVc),
    Dir(DirListVc),
}

#[turbo_tasks::value(transparent)]
pub(crate) struct DirList(IndexMap<String, DirListEntry>);

#[turbo_tasks::value_impl]
impl DirListVc {
    #[turbo_tasks::function]
    pub(crate) fn read(dir: FileSystemPathVc, recursive: bool, filter: RegexVc) -> Self {
        Self::read_internal(dir, dir, recursive, filter)
    }

    #[turbo_tasks::function]
    pub(crate) async fn read_internal(
        root: FileSystemPathVc,
        dir: FileSystemPathVc,
        recursive: bool,
        filter: RegexVc,
    ) -> Result<Self> {
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
                            DirListEntry::Dir(DirListVc::read_internal(
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

        Ok(Self::cell(list))
    }

    #[turbo_tasks::function]
    async fn flatten(self) -> Result<FlatDirListVc> {
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

        Ok(FlatDirListVc::cell(list))
    }
}

#[turbo_tasks::value(transparent)]
pub(crate) struct FlatDirList(IndexMap<String, FileSystemPathVc>);

#[turbo_tasks::value_impl]
impl FlatDirListVc {
    #[turbo_tasks::function]
    pub(crate) fn read(dir: FileSystemPathVc, recursive: bool, filter: RegexVc) -> Self {
        DirListVc::read(dir, recursive, filter).flatten()
    }
}

#[turbo_tasks::value]
#[derive(Debug)]
pub struct RequireContextMapEntry {
    pub origin_relative: String,
    pub request: RequestVc,
    pub result: ResolveResultVc,
}

/// The resolved context map for a `require.context(..)` call.
#[turbo_tasks::value(transparent)]
pub struct RequireContextMap(IndexMap<String, RequireContextMapEntry>);

#[turbo_tasks::value_impl]
impl RequireContextMapVc {
    #[turbo_tasks::function]
    pub(crate) async fn generate(
        origin: ResolveOriginVc,
        dir: FileSystemPathVc,
        recursive: bool,
        filter: RegexVc,
        issue_source: OptionIssueSourceVc,
        issue_severity: IssueSeverityVc,
    ) -> Result<Self> {
        let origin_path = &*origin.origin_path().parent().await?;

        let list = &*FlatDirListVc::read(dir, recursive, filter).await?;

        let mut map = IndexMap::new();

        for (context_relative, path) in list {
            if let Some(origin_relative) = origin_path.get_relative_path_to(&*path.await?) {
                let request = RequestVc::parse(Value::new(origin_relative.clone().into()));
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

        Ok(Self::cell(map))
    }
}

/// A reference for `require.context()`, will replace it with an inlined map
/// wrapped in `__turbopack_require_context__`;
#[turbo_tasks::value]
#[derive(Hash, Debug)]
pub struct CjsRequireContextAssetReference {
    pub origin: ResolveOriginVc,
    pub dir: FileSystemPathVc,
    pub include_subdirs: bool,
    pub filter: RegexVc,
    pub path: AstPathVc,
    pub issue_source: OptionIssueSourceVc,
    pub in_try: bool,
}

#[turbo_tasks::value_impl]
impl CjsRequireContextAssetReferenceVc {
    #[turbo_tasks::function]
    pub fn new(
        origin: ResolveOriginVc,
        dir: String,
        include_subdirs: bool,
        filter: RegexVc,
        path: AstPathVc,
        issue_source: OptionIssueSourceVc,
        in_try: bool,
    ) -> Self {
        let dir = origin.origin_path().parent().join(&dir);

        Self::cell(CjsRequireContextAssetReference {
            origin,
            dir,
            include_subdirs,
            filter,
            path,
            issue_source,
            in_try,
        })
    }
}

#[turbo_tasks::value_impl]
impl AssetReference for CjsRequireContextAssetReference {
    #[turbo_tasks::function]
    async fn resolve_reference(&self) -> Result<ResolveResultVc> {
        let map = &*generate_require_context_map(
            self.origin,
            self.dir,
            self.include_subdirs,
            self.filter,
            self.issue_source,
            try_to_severity(self.in_try),
        )
        .await?;

        let mut result = ResolveResult::unresolveable();

        for (_, entry) in map {
            result.merge_alternatives(&*entry.result.await?);
        }

        Ok(result.cell())
    }
}

#[turbo_tasks::value_impl]
impl ValueToString for CjsRequireContextAssetReference {
    #[turbo_tasks::function]
    async fn to_string(&self) -> Result<StringVc> {
        Ok(StringVc::cell(format!(
            "require.context {}/{}",
            self.dir.to_string().await?,
            if self.include_subdirs { "**" } else { "*" },
        )))
    }
}

#[turbo_tasks::value_impl]
impl ChunkableAssetReference for CjsRequireContextAssetReference {}

#[turbo_tasks::value_impl]
impl CodeGenerateable for CjsRequireContextAssetReference {
    #[turbo_tasks::function]
    async fn code_generation(
        &self,
        context: EcmascriptChunkingContextVc,
    ) -> Result<CodeGenerationVc> {
        let map = &*generate_require_context_map(
            self.origin,
            self.dir,
            self.include_subdirs,
            self.filter,
            self.issue_source,
            try_to_severity(self.in_try),
        )
        .await?;

        let mut context_map = ObjectLit {
            span: DUMMY_SP,
            props: vec![],
        };

        for (key, entry) in map {
            let pm = PatternMappingVc::resolve_request(
                entry.request,
                self.origin,
                context.into(),
                entry.result,
                Value::new(Cjs),
            )
            .await?;

            let prop = KeyValueProp {
                key: PropName::Str(key.as_str().into()),
                value: quote_expr!(
                    "{ internal: $internal, id: () => $id }",
                    internal: Expr = pm.is_internal_import().into(),
                    id: Expr = pm.apply(Expr::Lit(Lit::Str(entry.origin_relative.as_str().into()))),
                ),
            };

            context_map
                .props
                .push(PropOrSpread::Prop(box Prop::KeyValue(prop)));
        }

        let mut visitors = Vec::new();

        let path = &self.path.await?;
        visitors.push(create_visitor!(path, visit_mut_expr(expr: &mut Expr) {
            if let Expr::Call(_) = expr {
                *expr = quote!(
                    "__turbopack_require_context__($map)" as Expr,
                    map: Expr = Expr::Object(context_map.clone())
                );
            }
        }));

        Ok(CodeGeneration { visitors }.into())
    }
}
