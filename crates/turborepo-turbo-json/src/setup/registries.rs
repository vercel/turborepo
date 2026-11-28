use super::additional_tools::identifier;

/// The name of the registry built into `turbo`.
pub const BUILTIN_REGISTRY: &str = "aqua";

/// Splits a tool's source, written as `<registry>:<package ID>`, into its
/// registry name and package ID. The registry follows the registry-name rule.
/// The package ID is one or more `/`-separated segments of ASCII letters,
/// digits, `.`, `_`, and `-`, at most 256 bytes. `from` and `turbo.lock` share
/// this spelling.
pub fn registry_package(source: &str) -> Option<(&str, &str)> {
    let (registry, package) = source.split_once(':')?;
    let segment = |segment: &str| {
        !segment.is_empty()
            && segment
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
    };
    (identifier(registry) && package.len() <= 256 && package.split('/').all(segment))
        .then_some((registry, package))
}

#[cfg(test)]
mod tests;
