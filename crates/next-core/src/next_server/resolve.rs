use anyhow::Result;
use turbo_tasks::primitives::StringsVc;
use turbo_tasks_fs::{glob::GlobVc, FileJsonContent, FileSystemPathVc};
use turbopack_core::resolve::{
    find_context_file, package_json,
    parse::RequestVc,
    plugin::{ResolvePlugin, ResolvePluginConditionVc, ResolvePluginVc, ResolveResultOptionVc},
    FindContextFileResult, ResolveResult, SpecialType,
};

#[turbo_tasks::value]
pub(crate) struct ExternalCjsModulesResolvePlugin {
    root: FileSystemPathVc,
    transpiled_packages: StringsVc,
}

#[turbo_tasks::value_impl]
impl ExternalCjsModulesResolvePluginVc {
    #[turbo_tasks::function]
    pub fn new(root: FileSystemPathVc, transpiled_packages: StringsVc) -> Self {
        ExternalCjsModulesResolvePlugin {
            root,
            transpiled_packages,
        }
        .cell()
    }
}

#[turbo_tasks::value_impl]
impl ResolvePlugin for ExternalCjsModulesResolvePlugin {
    #[turbo_tasks::function]
    fn condition(&self) -> ResolvePluginConditionVc {
        ResolvePluginConditionVc::new(self.root, GlobVc::new("**/node_modules"))
    }

    #[turbo_tasks::function]
    async fn after_resolve(
        &self,
        fs_path: FileSystemPathVc,
        _request: RequestVc,
    ) -> Result<ResolveResultOptionVc> {
        let transpiled_glob = packages_glob(self.transpiled_packages).await?;

        // always bundle transpiled modules
        if transpiled_glob.execute(&fs_path.await?.path) {
            return Ok(ResolveResultOptionVc::none());
        }

        // check `package.json` for `"type": "module"`
        if let FindContextFileResult::Found(package_json, _) =
            &*find_context_file(fs_path.parent(), package_json()).await?
        {
            if let FileJsonContent::Content(package) = &*package_json.read_json().await? {
                if let Some("module") = package["type"].as_str() {
                    return Ok(ResolveResultOptionVc::none());
                }
            }
        }

        Ok(ResolveResultOptionVc::some(
            ResolveResult::Special(SpecialType::OriginalReferenceExternal, Vec::new()).cell(),
        ))
    }
}

#[turbo_tasks::function]
async fn packages_glob(packages: StringsVc) -> Result<GlobVc> {
    Ok(GlobVc::new(&format!(
        "**/node_modules/{{{}}}/**",
        packages.await?.join(",")
    )))
}
