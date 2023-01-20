use anyhow::{anyhow, bail, Result};
use serde::{Deserialize, Serialize};
use swc_core::ecma::ast::{Lit, Program};
use turbo_tasks::{primitives::StringVc, trace::TraceRawVcs, ValueToString};
use turbo_tasks_fs::FileSystemPathVc;
use turbopack::condition::ContextCondition;
use turbopack_core::{
    asset::AssetVc,
    issue::{Issue, IssueSeverity, IssueSeverityVc, IssueVc},
};
use turbopack_ecmascript::{parse::ParseResult, EcmascriptModuleAssetVc};

use crate::next_config::NextConfigVc;

/// Converts a filename within the server root into a next pathname.
#[turbo_tasks::function]
pub async fn pathname_for_path(
    server_root: FileSystemPathVc,
    server_path: FileSystemPathVc,
    has_extension: bool,
) -> Result<StringVc> {
    let server_path_value = &*server_path.await?;
    let path = if let Some(path) = server_root.await?.get_path_to(server_path_value) {
        path
    } else {
        bail!(
            "server_path ({}) is not in server_root ({})",
            server_path.to_string().await?,
            server_root.to_string().await?
        )
    };
    let path = if has_extension {
        path.rsplit_once('.')
            .ok_or_else(|| anyhow!("path ({}) has no extension", path))?
            .0
    } else {
        path
    };
    let path = if path == "index" {
        ""
    } else {
        path.strip_suffix("/index").unwrap_or(path)
    };

    Ok(StringVc::cell(path.to_string()))
}

// Adapted from https://github.com/vercel/next.js/blob/canary/packages/next/shared/lib/router/utils/get-asset-path-from-route.ts
pub fn get_asset_path_from_route(route: &str, ext: &str) -> String {
    if route.is_empty() {
        format!("index{}", ext)
    } else if route == "index" || route.starts_with("index/") {
        format!("index/{}{}", route, ext)
    } else {
        format!("{}{}", route, ext)
    }
}

pub async fn foreign_code_context_condition(next_config: NextConfigVc) -> Result<ContextCondition> {
    let transpile_packages = next_config.transpile_packages().await?;
    let result = if transpile_packages.is_empty() {
        ContextCondition::InDirectory("node_modules".to_string())
    } else {
        ContextCondition::all(vec![
            ContextCondition::InDirectory("node_modules".to_string()),
            ContextCondition::not(ContextCondition::any(
                transpile_packages
                    .iter()
                    .map(|package| ContextCondition::InDirectory(format!("node_modules/{package}")))
                    .collect(),
            )),
        ])
    };
    Ok(result)
}

#[derive(Default, PartialEq, Eq, Clone, Copy, Debug, TraceRawVcs, Serialize, Deserialize)]
pub enum NextRuntime {
    #[default]
    NodeJs,
    Edge,
}

#[turbo_tasks::value]
#[derive(Default)]
pub struct NextSourceConfig {
    pub runtime: NextRuntime,
}

/// An issue that occurred while resolving the React Refresh runtime module.
#[turbo_tasks::value(shared)]
pub struct NextSourceConfigParsingIssue {
    path: FileSystemPathVc,
    detail: StringVc,
}

#[turbo_tasks::value_impl]
impl Issue for NextSourceConfigParsingIssue {
    #[turbo_tasks::function]
    fn severity(&self) -> IssueSeverityVc {
        IssueSeverity::Warning.into()
    }

    #[turbo_tasks::function]
    fn title(&self) -> StringVc {
        StringVc::cell("Unable to parse config export in source file".to_string())
    }

    #[turbo_tasks::function]
    fn category(&self) -> StringVc {
        StringVc::cell("parsing".to_string())
    }

    #[turbo_tasks::function]
    fn context(&self) -> FileSystemPathVc {
        self.path
    }

    #[turbo_tasks::function]
    fn description(&self) -> StringVc {
        StringVc::cell(
            "The exported configuration object in a source file need to have a very specific \
             format from which some properties can be statically parsed at compiled-time."
                .to_string(),
        )
    }

    #[turbo_tasks::function]
    fn detail(&self) -> StringVc {
        self.detail
    }
}

#[turbo_tasks::function]
pub async fn parse_config_from_source(module_asset: AssetVc) -> Result<NextSourceConfigVc> {
    if let Some(ecmascript_asset) = EcmascriptModuleAssetVc::resolve_from(module_asset).await? {
        if let ParseResult::Ok { program, .. } = &*ecmascript_asset.parse().await? {
            if let Program::Module(module) = &program {
                for item in &module.body {
                    if let Some(decl) = item
                        .as_module_decl()
                        .and_then(|mod_decl| mod_decl.as_export_decl())
                        .and_then(|export_decl| export_decl.decl.as_var())
                    {
                        for decl in &decl.decls {
                            if decl
                                .name
                                .as_ident()
                                .map(|ident| &*ident.sym == "config")
                                .unwrap_or_default()
                            {
                                let invalid_config = |detail: &str| {
                                    NextSourceConfigParsingIssue {
                                        path: module_asset.path(),
                                        detail: StringVc::cell(detail.to_string()),
                                    }
                                    .cell()
                                    .as_issue()
                                    .emit()
                                };
                                if let Some(obj) =
                                    decl.init.as_ref().and_then(|init| init.as_object())
                                {
                                    let mut config = NextSourceConfig::default();
                                    for prop in &obj.props {
                                        if let Some(key_value) =
                                            prop.as_prop().and_then(|prop| prop.as_key_value())
                                        {
                                            if key_value
                                                .key
                                                .as_ident()
                                                .map(|ident| &*ident.sym == "runtime")
                                                .unwrap_or_default()
                                            {
                                                if let Some(value) =
                                                    key_value.value.as_lit().and_then(|lit| {
                                                        if let Lit::Str(str) = lit {
                                                            Some(&*str.value)
                                                        } else {
                                                            None
                                                        }
                                                    })
                                                {
                                                    match value {
                                                        "edge" | "experimental-edge" => {
                                                            config.runtime = NextRuntime::Edge
                                                        }
                                                        "nodejs" => {
                                                            config.runtime = NextRuntime::Edge
                                                        }
                                                        _ => {
                                                            invalid_config(
                                                                "The value of the runtime \
                                                                 property is not a valid runtime \
                                                                 name.",
                                                            );
                                                        }
                                                    }
                                                } else {
                                                    invalid_config(
                                                        "The value of the runtime property is not \
                                                         a simple string literal.",
                                                    );
                                                }
                                            }
                                        } else {
                                            invalid_config(
                                                "The exported config object must only contain \
                                                 simple key-value pairs.",
                                            );
                                        }
                                    }
                                    return Ok(config.cell());
                                } else {
                                    invalid_config(
                                        "The exported config object is not a valid object literal.",
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(NextSourceConfig::default().cell())
}
