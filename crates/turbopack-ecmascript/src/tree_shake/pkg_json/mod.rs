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
    let mut f = Some(filename);

    while let Some(cur) = f {
        let cur = cur.parent();
        let p = cur.join("package.json".to_string());

        if p.exists() {
            return Ok(p.into());
        }

        f = cur.parent().map(|p| p.into());
    }

    bail!("Could not find package.json for {}", filename.display())
}
