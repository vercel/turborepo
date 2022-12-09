use std::collections::HashMap;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use turbo_tasks::{trace::TraceRawVcs, Value};
use turbo_tasks_fs::FileSystemPathVc;
use turbopack_core::{
    chunk::ChunkingContextVc, context::AssetContextVc, virtual_asset::VirtualAssetVc,
};
use turbopack_ecmascript::{
    EcmascriptInputTransform, EcmascriptInputTransformsVc, EcmascriptModuleAssetType,
    EcmascriptModuleAssetVc,
};
use turbopack_node::read_config::{JavaScriptConfig, JavaScriptConfigVc, JavaScriptValue};

use crate::embed_js::next_js_file;

#[turbo_tasks::value(transparent)]
pub struct NextConfigValue(NextConfig);

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, TraceRawVcs)]
#[serde(rename_all = "camelCase")]
pub struct NextConfig {
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
                Ok(NextConfigValue(serde_json::from_reader(val.read())?).cell())
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
