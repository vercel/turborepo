//! Package keys and references as text, with the semantics of pnpm's
//! `PkgNameVerPeer`, `SnapshotDepRef` and `ImporterDepVersion`. A key is
//! `name@version` plus peer groups; a reference is `version`, `alias@version`
//! or `link:<path>`.

use std::cmp::Ordering;

pub(super) fn split(key: &str) -> Option<(&str, &str)> {
    let at = if let Some(scoped) = key.strip_prefix('@') {
        scoped.find('@')? + 1
    } else {
        key.find('@')?
    };
    let (name, suffix) = (&key[..at], &key[at + 1..]);
    (!name.is_empty() && name != "@" && !suffix.is_empty()).then_some((name, suffix))
}

pub(super) fn without_peer(suffix: &str) -> &str {
    if suffix.ends_with(')')
        && let Some(open) = suffix.find('(')
    {
        return &suffix[..open];
    }
    suffix
}

pub(super) fn package_key(key: &str) -> &str {
    match split(key) {
        Some((name, suffix)) => &key[..name.len() + 1 + without_peer(suffix).len()],
        None => key,
    }
}

/// Port of pnpm `without_groups` over a reference's text.
pub(super) fn without_groups(reference: &str) -> &str {
    if reference.starts_with("link:") {
        return reference;
    }
    without_peer(reference)
}

pub(super) fn as_link_target(reference: &str) -> Option<&str> {
    reference.strip_prefix("link:")
}

/// Port of pnpm `looks_like_alias`.
pub(super) fn looks_like_alias(value: &str) -> bool {
    if value.starts_with('@') {
        return true;
    }
    let Some(at) = value.find('@') else {
        return false;
    };
    value.find('(').is_none_or(|paren| at < paren) && value.find(':').is_none_or(|colon| at < colon)
}

pub(super) fn resolve(alias: &str, reference: &str) -> Option<String> {
    if reference.starts_with("link:") {
        return None;
    }
    if looks_like_alias(reference) {
        return Some(reference.to_string());
    }
    Some(format!("{alias}@{reference}"))
}

/// Port of pnpm `with_key` and `importer_version_with_key`.
pub(super) fn with_key(reference: &str, key: &str) -> String {
    if reference.starts_with("link:") {
        return reference.to_string();
    }
    if looks_like_alias(reference) {
        return key.to_string();
    }
    match split(key) {
        Some((_, suffix)) => suffix.to_string(),
        None => reference.to_string(),
    }
}

/// Port of pnpm `compare_names`.
pub(super) fn compare_names(left: &str, right: &str) -> Ordering {
    name_parts(left).cmp(&name_parts(right))
}

fn name_parts(name: &str) -> (Option<&str>, &str) {
    match name.strip_prefix('@').and_then(|rest| rest.split_once('/')) {
        Some((scope, bare)) => (Some(scope), bare),
        None => (None, name),
    }
}

/// Port of pnpm `link_path_to_peer_version`.
pub(super) fn link_path_to_peer_version(rel_path: &str) -> String {
    let trimmed = rel_path.trim_start_matches('.');
    let mut out = String::with_capacity(rel_path.len());
    let mut last_was_plus = true;
    for ch in trimmed.chars() {
        let needs_replacing = ch.is_control()
            || matches!(
                ch,
                '"' | '*' | '+' | '/' | ':' | '<' | '>' | '?' | '\\' | '|'
            );
        if !needs_replacing {
            out.push(ch);
            last_was_plus = false;
            continue;
        }
        if !last_was_plus {
            out.push('+');
            last_was_plus = true;
        }
    }
    let trimmed_end = out.trim_end_matches(['+', '.']).len();
    if trimmed_end > 0 {
        out.truncate(trimmed_end);
        return out;
    }
    if rel_path.is_empty() {
        String::new()
    } else {
        "+".to_string()
    }
}

/// Port of pnpm `create_peer_dep_graph_hash`.
pub(super) fn create_peer_dep_graph_hash(mut segments: Vec<String>, max_length: usize) -> String {
    use sha2::{Digest, Sha256};
    segments.sort();
    let body = segments.join(")(");
    if body.len() > max_length {
        let digest = Sha256::digest(body.as_bytes());
        let mut hex = format!("{digest:x}");
        hex.truncate(32);
        format!("({hex})")
    } else {
        format!("({body})")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_keys() {
        assert_eq!(split("react@18.2.0"), Some(("react", "18.2.0")));
        assert_eq!(
            split("@types/react@18.3.31"),
            Some(("@types/react", "18.3.31"))
        );
        assert_eq!(split("a@file:packages/a"), Some(("a", "file:packages/a")));
        assert_eq!(
            package_key("react-dom@18.3.1(react@18.2.0)"),
            "react-dom@18.3.1"
        );
        assert_eq!(
            package_key("is-even@1.0.0(patch_hash=abc)"),
            "is-even@1.0.0"
        );
    }

    #[test]
    fn resolves_references() {
        assert_eq!(resolve("react", "18.2.0").as_deref(), Some("react@18.2.0"));
        assert_eq!(
            resolve("odd", "is-odd@3.0.1").as_deref(),
            Some("is-odd@3.0.1")
        );
        assert_eq!(resolve("a", "link:../a"), None);
        assert_eq!(
            resolve("a", "file:packages/a").as_deref(),
            Some("a@file:packages/a")
        );
        assert_eq!(
            with_key("file:packages/a", "a@file:packages/a(react@18.2.0)"),
            "file:packages/a(react@18.2.0)"
        );
    }

    #[test]
    fn orders_names_like_pnpm() {
        assert_eq!(compare_names("zod", "@babel/core"), Ordering::Less);
        assert_eq!(compare_names("@a/z", "@b/a"), Ordering::Less);
    }

    #[test]
    fn link_peer_versions() {
        assert_eq!(
            link_path_to_peer_version("packages/provider"),
            "packages+provider"
        );
        assert_eq!(link_path_to_peer_version("../a"), "a");
    }

    #[test]
    fn hashes_long_suffixes() {
        let short = create_peer_dep_graph_hash(vec!["b@1".into(), "a@1".into()], 1000);
        assert_eq!(short, "(a@1)(b@1)");
        let long = create_peer_dep_graph_hash(vec!["a@1".into(), "b@1".into()], 3);
        assert_eq!(long.len(), 34);
    }
}
