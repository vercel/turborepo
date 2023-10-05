use anyhow::{bail, Result};
use serde::Deserialize;
use turbo_tasks::Vc;
use turbo_tasks_fs::{File, FileContent, FileSystemPath};

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
    let package_json = read_package_json_for(filename.clone()).await?;

    match &*package_json {
        FileContent::Content(file) => {
            let json = serde_json::from_slice::<PackageJson>(&**file)?;

            Ok(!json.side_effects.into())
        }
        FileContent::NotFound => Ok(false),
    }
}

#[turbo_tasks::function]
async fn read_package_json_for(filename: Vc<FileSystemPath>) -> Result<Vc<FileContent>> {
    let mut f = filename;

    while !f.await?.is_root() {
        let cur = f.parent();
        let p = cur.join("package.json".to_string());

        let content_vc = p.read();
        let content = content_vc.await?;

        if let FileContent::Content(f) = &*content {
            return Ok(content_vc);
        }

        f = cur;
    }

    bail!("Could not find package.json for {}", filename.display())
}
