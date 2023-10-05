use anyhow::{bail, Result};
use serde::Deserialize;
use turbo_tasks::Vc;
use turbo_tasks_fs::FileSystemPath;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PackageJson {
    #[serde(default = "true_by_default")]
    side_effects: bool,
}

fn true_by_default() -> bool {
    true
}

#[turbo_tasks::function]
pub async fn is_side_effect_free(filename: Vc<FileSystemPath>) -> Result<Vc<bool>> {
    let package_json = find_package_json_for(filename.clone());

    let content = package_json.read().await?;

    let json = serde_json::from_slice::<PackageJson>(&content)?;

    Ok(!json.side_effects.into())
}

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
