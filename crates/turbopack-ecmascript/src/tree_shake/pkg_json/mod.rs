use anyhow::{bail, Result};
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

#[turbo_tasks::function]
async fn find_package_json_for(filename: Vc<FileSystemPath>) -> Result<Vc<FileSystemPath>> {
    let mut f = filename;

    while !f.await?.is_root() {
        let cur = f.parent();
        let p = cur.join("package.json".to_string());

        if p.exists() {
            return Ok(p.into());
        }

        f = cur;
    }

    bail!("Could not find package.json for {}", filename.display())
}
