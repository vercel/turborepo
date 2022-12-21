use anyhow::Result;

/// Strips the base path from the given path. The base path must start with a
/// slash or be the empty string.
///
/// Returns `None` if the path does not start with the base path.
pub fn strip_base_path<'a, 'b>(path: &'a str, base_path: &'b str) -> Result<Option<&'a str>> {
    if base_path.is_empty() {
        return Ok(Some(path));
    }

    let base_path = base_path
        .strip_prefix('/')
        .ok_or_else(|| anyhow::anyhow!("base path must start with a slash, got {}", base_path))?;

    Ok(path
        .strip_prefix(base_path)
        .and_then(|path| path.strip_prefix('/')))
}
