use std::collections::HashMap;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use turbo_tasks::{trace::TraceRawVcs, Value};
use turbo_tasks_fs::{FileContent, FileSystemPath, FileSystemPathVc, FileSystemVc};
use turbopack_core::{
    asset::{AssetContent, AssetVc},
    chunk::ChunkingContextVc,
    context::AssetContextVc,
    source_asset::SourceAssetVc,
    virtual_asset::VirtualAssetVc,
};
use turbopack_ecmascript::{
    EcmascriptInputTransform, EcmascriptInputTransformsVc, EcmascriptModuleAssetType,
    EcmascriptModuleAssetVc,
};
use turbopack_node::read_config::{JavaScriptConfig, JavaScriptConfigVc, JavaScriptValue};

use crate::embed_js::next_js_file;

#[turbo_tasks::value(transparent)]
pub struct NextConfigValue(NextConfig);

#[turbo_tasks::value_impl]
impl NextConfigValueVc {
    #[turbo_tasks::function]
    pub async fn config_asset(self, fs: FileSystemVc, context: AssetContextVc) -> Result<AssetVc> {
        let this = self.await?;
        if let Some(config_file) = &this.config_file {
            let path = FileSystemPath {
                fs,
                path: config_file.clone(),
            };
            if let Some(relative) = path.get_relative_path_to(&*fs.root().await?) {
                let next_config_path = fs.root().join(&relative);
                let is_typescript = relative.ends_with(".ts");
                let asset_vc = EcmascriptModuleAssetVc::new(
                    SourceAssetVc::new(next_config_path).into(),
                    context,
                    Value::new(if is_typescript {
                        EcmascriptModuleAssetType::Typescript
                    } else {
                        EcmascriptModuleAssetType::Ecmascript
                    }),
                    EcmascriptInputTransformsVc::cell(if is_typescript {
                        vec![EcmascriptInputTransform::TypeScript]
                    } else {
                        vec![]
                    }),
                    context.environment(),
                );
                return Ok(asset_vc.into());
            }
        }
        Ok(VirtualAssetVc::new(
            fs.root().join("__EMPTY_NEXT_CONFIG__.js"),
            AssetContent::File(FileContent::NotFound.cell()).cell(),
        )
        .into())
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, TraceRawVcs)]
#[serde(rename_all = "camelCase")]
pub struct NextConfig {
    pub config_file: Option<String>,
    pub config_file_name: String,
    pub typescript: Option<TypeScriptConfig>,
    pub react_strict_mode: Option<bool>,
    pub experimental: Option<ExperimentalConfig>,
    pub env: Option<HashMap<String, String>>,
    pub compiler: Option<CompilerConfig>,
}

#[derive(Clone, Debug, Ord, PartialOrd, PartialEq, Eq, Serialize, Deserialize, TraceRawVcs)]
#[serde(rename_all = "camelCase")]
pub struct TypeScriptConfig {
    pub ignore_build_errors: Option<bool>,
    pub ts_config_path: Option<String>,
}

#[derive(Clone, Debug, Ord, PartialOrd, PartialEq, Eq, Serialize, Deserialize, TraceRawVcs)]
#[serde(rename_all = "camelCase")]
pub struct ExperimentalConfig {
    pub server_components_external_packages: Option<Vec<String>>,
}

#[derive(Clone, Debug, Ord, PartialOrd, PartialEq, Eq, Serialize, Deserialize, TraceRawVcs)]
#[serde(rename_all = "camelCase")]
pub struct CompilerConfig {
    pub react_remove_properties: Option<bool>,
    pub relay: Option<RelayConfig>,
    pub remove_console: Option<RemoveConsoleConfig>,
}

#[derive(Clone, Debug, Ord, PartialOrd, PartialEq, Eq, Serialize, Deserialize, TraceRawVcs)]
#[serde(untagged, rename_all = "camelCase")]
pub enum ReactRemoveProperties {
    Boolean(bool),
    Config { properties: Option<Vec<String>> },
}

#[derive(Clone, Debug, Ord, PartialOrd, PartialEq, Eq, Serialize, Deserialize, TraceRawVcs)]
#[serde(rename_all = "camelCase")]
pub struct RelayConfig {
    pub src: String,
    pub artifact_directory: Option<String>,
    pub language: Option<RelayLanguage>,
}

#[derive(Clone, Debug, Ord, PartialOrd, PartialEq, Eq, Serialize, Deserialize, TraceRawVcs)]
#[serde(untagged, rename_all = "lowercase")]
pub enum RelayLanguage {
    TypeScript,
    Flow,
    JavaScript,
}

#[derive(Clone, Debug, Ord, PartialOrd, PartialEq, Eq, Serialize, Deserialize, TraceRawVcs)]
#[serde(untagged)]
pub enum RemoveConsoleConfig {
    Boolean(bool),
    Config { exclude: Option<Vec<String>> },
}

#[turbo_tasks::value(shared)]
pub struct NextConfigLoader {
    path: FileSystemPathVc,
    entry_module: EcmascriptModuleAssetVc,
    chunking_context: ChunkingContextVc,
    intermediate_output_path: FileSystemPathVc,
}

#[turbo_tasks::value_impl]
impl NextConfigLoaderVc {
    #[turbo_tasks::function]
    pub fn new(
        context: AssetContextVc,
        chunking_context: ChunkingContextVc,
        project_root: FileSystemPathVc,
        intermediate_output_path: FileSystemPathVc,
    ) -> Self {
        let entry_module = EcmascriptModuleAssetVc::new(
            VirtualAssetVc::new(
                intermediate_output_path.join("next.js"),
                next_js_file("entry/config/next.ts").into(),
            )
            .into(),
            context,
            Value::new(EcmascriptModuleAssetType::Typescript),
            EcmascriptInputTransformsVc::cell(vec![
                EcmascriptInputTransform::React { refresh: false },
                EcmascriptInputTransform::TypeScript,
            ]),
            context.environment(),
        );
        NextConfigLoader {
            path: project_root,
            entry_module,
            chunking_context,
            intermediate_output_path,
        }
        .cell()
    }

    #[turbo_tasks::function]
    pub async fn load_value(self) -> Result<NextConfigValueVc> {
        let val = self.load().await?;
        match &*val {
            JavaScriptValue::Value(val) => {
                let next_config: NextConfig = serde_json::from_reader(val.read())?;
                Ok(NextConfigValue(next_config).cell())
            }
            JavaScriptValue::Stream(_) => {
                unimplemented!("Stream not supported now");
            }
        }
    }
}

#[turbo_tasks::value_impl]
impl JavaScriptConfig for NextConfigLoader {
    #[turbo_tasks::function]
    fn path(&self) -> FileSystemPathVc {
        self.path.clone()
    }

    #[turbo_tasks::function]
    fn entry(&self) -> EcmascriptModuleAssetVc {
        self.entry_module.clone()
    }

    #[turbo_tasks::function]
    fn chunking_context(&self) -> ChunkingContextVc {
        self.chunking_context.clone()
    }

    #[turbo_tasks::function]
    fn intermediate_output_path(&self) -> FileSystemPathVc {
        self.intermediate_output_path.clone()
    }
}
