use anyhow::Result;
use serde::Deserialize;
use turbo_tasks::Vc;
use turbo_tasks_fs::FileSystemPath;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PackageJson {
    side_effects: bool,
}

#[turbo_tasks::function]
pub async fn is_side_effect_free(filename: Vc<FileSystemPath>) -> Result<Vc<bool>> {}
