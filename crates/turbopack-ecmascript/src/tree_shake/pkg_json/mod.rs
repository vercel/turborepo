use anyhow::Result;
use turbo_tasks::Vc;
use turbo_tasks_fs::FileSystemPath;
use turbopack_core::{
    package_json::read_package_json,
    resolve::{find_context_file, FindContextFileResult},
};

#[turbo_tasks::function]
pub async fn is_side_effect_free(filename: Vc<FileSystemPath>) -> Result<Vc<bool>> {
    let package_json = find_context_file(
        filename.parent(),
        Vc::cell(vec!["package.json".to_string()]),
    )
    .await?;

    let package_json = match &*package_json {
        FindContextFileResult::Found(path, ..) => *path,
        _ => return Ok(Vc::cell(false)),
    };
    let content = read_package_json(package_json).await?;

    let has_side_effect = match &*content {
        Some(json) => json
            .as_object()
            .and_then(|json| json.get("sideEffects"))
            .and_then(|side_effects| side_effects.as_bool())
            .unwrap_or(true),
        None => true,
    };

    Ok(Vc::cell(!has_side_effect))
}
