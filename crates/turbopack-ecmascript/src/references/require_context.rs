use std::collections::VecDeque;

use anyhow::Result;
use indexmap::IndexMap;
use swc_core::{
    common::DUMMY_SP,
    ecma::ast::{Expr, KeyValueProp, Lit, ObjectLit, Prop, PropName, PropOrSpread},
    quote, quote_expr,
};
use turbo_tasks::{
    primitives::{RegexVc, StringVc},
    Value, ValueToString, ValueToStringVc,
};
use turbo_tasks_fs::{DirectoryContent, DirectoryEntry, FileSystemPathVc};
use turbopack_core::{
    chunk::{ChunkableAssetReference, ChunkableAssetReferenceVc},
    issue::{IssueSeverityVc, OptionIssueSourceVc},
    reference::{AssetReference, AssetReferenceVc},
    resolve::{
        origin::{ResolveOrigin, ResolveOriginVc},
        parse::RequestVc,
        ResolveResult, ResolveResultVc,
    },
};

use crate::{
    chunk::EcmascriptChunkingContextVc,
    code_gen::{CodeGenerateable, CodeGeneration, CodeGenerationVc},
    create_visitor,
    references::{
        pattern_mapping::{PatternMappingVc, ResolveType::Cjs},
        AstPathVc,
    },
    resolve::{cjs_resolve, try_to_severity},
    CodeGenerateableVc,
};

#[turbo_tasks::value(transparent)]
pub(crate) struct FlatDirList(Vec<(String, FileSystemPathVc)>);

#[turbo_tasks::function]
pub(crate) async fn list_dir(
    dir: FileSystemPathVc,
    recursive: bool,
    filter: RegexVc,
) -> Result<FlatDirListVc> {
    let root = &*dir.await?;
    let filter = &*filter.await?;

    let mut queue = VecDeque::with_capacity(1);
    queue.push_back(dir);

    let mut list = Vec::new();

    while let Some(dir) = queue.pop_front() {
        let dir_content = dir.read_dir().await?;
        let entries = match &*dir_content {
            DirectoryContent::Entries(entries) => entries,
            DirectoryContent::NotFound => continue,
        };

        for (_, entry) in entries {
            match entry {
                DirectoryEntry::File(path) => {
                    if let Some(relative_path) = root.get_relative_path_to(&*path.await?) {
                        if filter.is_match(&relative_path) {
                            list.push((relative_path, *path));
                        }
                    }
                }
                DirectoryEntry::Directory(path) if recursive => {
                    queue.push_back(*path);
                }
                // ignore everything else
                _ => {}
            }
        }
    }

    Ok(FlatDirListVc::cell(list))
}

#[turbo_tasks::value]
#[derive(Debug)]
pub(crate) struct RequireContextMapEntry {
    pub origin_relative: String,
    pub request: RequestVc,
    pub result: ResolveResultVc,
}

#[turbo_tasks::value(transparent)]
pub(crate) struct RequireContextMap(IndexMap<String, RequireContextMapEntry>);

#[turbo_tasks::function]
pub(crate) async fn generate_require_context_map(
    origin: ResolveOriginVc,
    dir: FileSystemPathVc,
    recursive: bool,
    filter: RegexVc,
    issue_source: OptionIssueSourceVc,
    issue_severity: IssueSeverityVc,
) -> Result<RequireContextMapVc> {
    let origin_path = &*origin.origin_path().parent().await?;

    let list = &*list_dir(dir, recursive, filter).await?;

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
            // TODO: does this need to throw an error?
        }
    }

    Ok(RequireContextMapVc::cell(map))
}

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
