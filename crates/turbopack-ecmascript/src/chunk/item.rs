use std::{collections::HashMap, fmt::Write as _, io::Write as _};

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use turbo_tasks::{trace::TraceRawVcs, Upcast, ValueToString, Vc};
use turbo_tasks_fs::rope::Rope;
use turbopack_core::{
    chunk::{AsyncModuleInfo, ChunkItem, ChunkItemExt, ChunkingContext},
    code_builder::{Code, CodeBuilder},
    error::PrettyPrintError,
    ident::AssetIdent,
    issue::{code_gen::CodeGenerationIssue, IssueExt, IssueSeverity, StyledString},
    source_map::GenerateSourceMap,
};

use super::EcmascriptChunkingContext;
use crate::{
    references::async_module::{AsyncModuleOptions, OptionAsyncModuleOptions},
    utils::FormatIter,
    EcmascriptModuleContent,
};

#[turbo_tasks::value(shared)]
#[derive(Clone)]
pub struct EcmascriptChunkItemContent {
    pub source_url: SourceUrl,
    pub inner_code: Rope,
    pub source_map: Option<Vc<Box<dyn GenerateSourceMap>>>,
    pub options: EcmascriptChunkItemOptions,
    pub placeholder_for_future_extensions: (),
}

#[turbo_tasks::value_impl]
impl EcmascriptChunkItemContent {
    #[turbo_tasks::function]
    pub async fn new(ident: Vc<AssetIdent>, code: Vc<Code>) -> Result<Vc<Self>> {
        Ok(EcmascriptChunkItemContent {
            source_url: SourceUrl::from_asset_ident(ident).await?,
            inner_code: code.await?.source_code().clone(),
            source_map: Some(Vc::upcast(code)),
            options: Default::default(),
            placeholder_for_future_extensions: (),
        }
        .cell())
    }

    #[turbo_tasks::function]
    pub async fn new_with_options(
        ident: Vc<AssetIdent>,
        code: Vc<Code>,
        options: Vc<EcmascriptChunkItemOptions>,
    ) -> Result<Vc<Self>> {
        Ok(EcmascriptChunkItemContent {
            source_url: SourceUrl::from_asset_ident(ident).await?,
            inner_code: code.await?.source_code().clone(),
            source_map: Some(Vc::upcast(code)),
            options: options.await?.clone_value(),
            placeholder_for_future_extensions: (),
        }
        .cell())
    }

    #[turbo_tasks::function]
    pub async fn new_from_content(
        ident: Vc<AssetIdent>,
        content: Vc<EcmascriptModuleContent>,
        chunking_context: Vc<Box<dyn EcmascriptChunkingContext>>,
        async_module_options: Vc<OptionAsyncModuleOptions>,
    ) -> Result<Vc<Self>> {
        let refresh = *chunking_context.has_react_refresh().await?;
        let externals = *chunking_context
            .environment()
            .supports_commonjs_externals()
            .await?;

        let content = content.await?;
        let async_module = async_module_options.await?.clone_value();

        Ok(EcmascriptChunkItemContent {
            source_url: SourceUrl::from_asset_ident(ident).await?,
            inner_code: content.inner_code.clone(),
            source_map: content.source_map,
            options: if content.is_esm {
                EcmascriptChunkItemOptions {
                    strict: true,
                    refresh,
                    externals,
                    async_module,
                    ..Default::default()
                }
            } else {
                if async_module.is_some() {
                    bail!("CJS module can't be async.");
                }

                EcmascriptChunkItemOptions {
                    refresh,
                    externals,
                    // These things are not available in ESM
                    module: true,
                    exports: true,
                    require: true,
                    this: true,
                    ..Default::default()
                }
            },
            placeholder_for_future_extensions: (),
        }
        .cell())
    }

    #[turbo_tasks::function]
    pub fn module_factory(&self) -> Result<Vc<Code>> {
        let indent = "    ";

        let mut args = vec![
            "r: __turbopack_require__",
            "f: __turbopack_module_context__",
            "i: __turbopack_import__",
            "s: __turbopack_esm__",
            "v: __turbopack_export_value__",
            "n: __turbopack_export_namespace__",
            "c: __turbopack_cache__",
            "M: __turbopack_modules__",
            "l: __turbopack_load__",
            "j: __turbopack_dynamic__",
            "P: __turbopack_resolve_absolute_path__",
            "U: __turbopack_relative_url__",
            "R: __turbopack_resolve_module_id_path__",
            "g: global",
            // HACK
            "__dirname",
        ];
        if self.options.async_module.is_some() {
            args.push("a: __turbopack_async_module__");
        }
        if self.options.externals {
            args.push("x: __turbopack_external_require__");
            args.push("y: __turbopack_external_import__");
        }
        if self.options.refresh {
            args.push("k: __turbopack_refresh__");
        }
        if self.options.module {
            args.push("m: module");
        }
        if self.options.exports {
            args.push("e: exports");
        }
        if self.options.require {
            args.push("t: require");
        }
        if self.options.wasm {
            args.push("w: __turbopack_wasm__");
            args.push("u: __turbopack_wasm_module__");
        }
        let mut code = CodeBuilder::default();
        let args = FormatIter(|| args.iter().copied().intersperse(", "));
        if self.options.this {
            writeln!(code, "(function({{ {} }}) {{ !function() {{", args,)?;
        } else {
            writeln!(code, "(({{ {} }}) => (() => {{", args,)?;
        }
        if self.options.strict {
            writeln!(code, "{indent}\"use strict\";")?;
            writeln!(code)?;
        } else {
            writeln!(code)?;
        }

        if self.options.async_module.is_some() {
            code += indent;
            code += "__turbopack_async_module__(async (__turbopack_handle_async_dependencies__, \
                     __turbopack_async_result__) => { try {\n";
        }

        code.push_source(&self.inner_code, self.source_map);

        if let Some(opts) = &self.options.async_module {
            writeln!(code, "{indent}__turbopack_async_result__();")?;
            writeln!(
                code,
                "{indent}}} catch(e) {{ __turbopack_async_result__(e); }} }}, {});",
                opts.has_top_level_await
            )?;
        }

        writeln!(code)?;
        writeln!(
            code,
            "{indent}//# sourceURL={}",
            self.source_url.to_string()?
        )?;

        if self.options.this {
            code += "}.call(this) })";
        } else {
            code += "})())";
        }

        Ok(code.build().cell())
    }
}

#[derive(PartialEq, Eq, Debug, Clone, Serialize, Deserialize, TraceRawVcs)]
pub struct SourceUrlQuery {
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub query: HashMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fragment: Option<String>,
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub assets: HashMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifiers: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub part: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layer: Option<String>,
}

#[derive(PartialEq, Eq, Debug, Clone, Serialize, Deserialize, TraceRawVcs)]
pub struct SourceUrl {
    pub path: String,
    pub query: SourceUrlQuery,
}

impl SourceUrl {
    pub async fn from_asset_ident(ident: Vc<AssetIdent>) -> Result<Self> {
        let ident = &*ident.await?;

        let path = ident.path.to_string().await?.clone_value();

        let query = &*ident.query.await?;
        let query = if !query.is_empty() {
            serde_qs::from_str(query)?
        } else {
            Default::default()
        };

        let fragment = if let Some(fragment) = &ident.fragment {
            Some(fragment.await?.clone_value())
        } else {
            None
        };

        let mut assets = HashMap::with_capacity(ident.assets.len());
        for (key, asset) in &ident.assets {
            assets.insert(
                key.await?.clone_value(),
                asset.to_string().await?.clone_value(),
            );
        }

        let layer = if let Some(layer) = &ident.layer {
            Some(layer.await?.clone_value())
        } else {
            None
        };

        let modifiers = if !ident.modifiers.is_empty() {
            let mut s = String::new();

            for (i, modifier) in ident.modifiers.iter().enumerate() {
                if i > 0 {
                    s.push(',');
                }
                s.push_str(&modifier.await?);
            }

            Some(s)
        } else {
            None
        };

        let part = if let Some(part) = &ident.part {
            Some(part.to_string().await?.clone_value())
        } else {
            None
        };

        Ok(Self {
            path,
            query: SourceUrlQuery {
                query,
                fragment,
                assets,
                modifiers,
                part,
                layer,
            },
        })
    }

    pub fn to_string(&self) -> Result<String> {
        let query = serde_qs::to_string(&self.query)?;

        let mut url = "turbopack://".to_string();
        url += &self.path.replace(' ', "+");

        if !query.is_empty() {
            write!(url, "?{}", &*query)?;
        }

        Ok(url)
    }
}

#[turbo_tasks::value(shared)]
#[derive(Default, Debug, Clone)]
pub struct EcmascriptChunkItemOptions {
    /// Whether this chunk item should be in "use strict" mode.
    pub strict: bool,
    /// Whether this chunk item's module factory should include a
    /// `__turbopack_refresh__` argument.
    pub refresh: bool,
    /// Whether this chunk item's module factory should include a `module`
    /// argument.
    pub module: bool,
    /// Whether this chunk item's module factory should include an `exports`
    /// argument.
    pub exports: bool,
    /// Whether this chunk item's module factory should include a `require`
    /// argument.
    pub require: bool,
    /// Whether this chunk item's module factory should include a
    /// `__turbopack_external_require__` argument.
    pub externals: bool,
    /// Whether this chunk item's module is async (either has a top level await
    /// or is importing async modules).
    pub async_module: Option<AsyncModuleOptions>,
    pub this: bool,
    /// Whether this chunk item's module factory should include
    /// `__turbopack_wasm__` to load WebAssembly.
    pub wasm: bool,
    pub placeholder_for_future_extensions: (),
}

#[turbo_tasks::value_trait]
pub trait EcmascriptChunkItem: ChunkItem {
    fn content(self: Vc<Self>) -> Vc<EcmascriptChunkItemContent>;
    fn content_with_async_module_info(
        self: Vc<Self>,
        _async_module_info: Option<Vc<AsyncModuleInfo>>,
    ) -> Vc<EcmascriptChunkItemContent> {
        self.content()
    }
    fn chunking_context(self: Vc<Self>) -> Vc<Box<dyn EcmascriptChunkingContext>>;

    /// Specifies which availablility information the chunk item needs for code
    /// generation
    fn need_async_module_info(self: Vc<Self>) -> Vc<bool> {
        Vc::cell(false)
    }
}

pub trait EcmascriptChunkItemExt: Send {
    /// Generates the module factory for this chunk item.
    fn code(self: Vc<Self>, async_module_info: Option<Vc<AsyncModuleInfo>>) -> Vc<Code>;
}

impl<T> EcmascriptChunkItemExt for T
where
    T: Upcast<Box<dyn EcmascriptChunkItem>>,
{
    /// Generates the module factory for this chunk item.
    fn code(self: Vc<Self>, async_module_info: Option<Vc<AsyncModuleInfo>>) -> Vc<Code> {
        module_factory_with_code_generation_issue(Vc::upcast(self), async_module_info)
    }
}

#[turbo_tasks::function]
async fn module_factory_with_code_generation_issue(
    chunk_item: Vc<Box<dyn EcmascriptChunkItem>>,
    async_module_info: Option<Vc<AsyncModuleInfo>>,
) -> Result<Vc<Code>> {
    Ok(
        match chunk_item
            .content_with_async_module_info(async_module_info)
            .module_factory()
            .resolve()
            .await
        {
            Ok(factory) => factory,
            Err(error) => {
                let id = chunk_item.id().to_string().await;
                let id = id.as_ref().map_or_else(|_| "unknown", |id| &**id);
                let error = error.context(format!(
                    "An error occurred while generating the chunk item {}",
                    id
                ));
                let error_message = format!("{}", PrettyPrintError(&error));
                let js_error_message = serde_json::to_string(&error_message)?;
                CodeGenerationIssue {
                    severity: IssueSeverity::Error.cell(),
                    path: chunk_item.asset_ident().path(),
                    title: StyledString::Text("Code generation for chunk item errored".to_string())
                        .cell(),
                    message: StyledString::Text(error_message).cell(),
                }
                .cell()
                .emit();
                let mut code = CodeBuilder::default();
                code += "(() => {{\n\n";
                writeln!(code, "throw new Error({error});", error = &js_error_message)?;
                code += "\n}})";
                code.build().cell()
            }
        },
    )
}

#[turbo_tasks::value(transparent)]
pub struct EcmascriptChunkItemsChunk(Vec<Vc<Box<dyn EcmascriptChunkItem>>>);

#[turbo_tasks::value(transparent)]
pub struct EcmascriptChunkItems(pub(super) Vec<Vc<Box<dyn EcmascriptChunkItem>>>);
