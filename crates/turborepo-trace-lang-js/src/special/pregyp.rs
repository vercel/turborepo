//! node-pre-gyp's `binary` configuration
//! (`@mapbox/node-pre-gyp/lib/util/versioning.js`): `binary.module_path` is a
//! template such as `./lib/binding/{configuration}/
//! {node_abi}-{platform}-{arch}` or `./lib/binding/napi-v{napi_build_version}`,
//! and the addon is `<module_path>/<module_name>.node`.
//!
//! Placeholders that depend on the target platform are expanded for
//! [`Platform`]. Ones that depend on the Node runtime (`node_abi`, several
//! `napi_versions`) become `*` and are reported, so the caller can attach a
//! partial-specifier uncertainty.

use serde_json::Value;

use super::Platform;

/// The addon location derived from a `binary` config.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModulePattern {
    /// Path relative to the package root, with `*` for unknown parts.
    pub pattern: String,
    /// Placeholders that could not be expanded.
    pub unknown: Vec<String>,
}

/// Expands the `binary` config of package.json `pj` for `platform`. `None` if
/// there is no `binary.module_name` or `binary.module_path`.
pub fn module_pattern(pj: &Value, platform: &Platform) -> Option<ModulePattern> {
    let binary = pj.get("binary")?;
    let module_name = binary.get("module_name")?.as_str()?;
    let module_path = binary.get("module_path")?.as_str()?;
    let version = pj.get("version").and_then(Value::as_str).unwrap_or("");
    let (core, build) = version.split_once('+').unwrap_or((version, ""));
    let (core, prerelease) = core.split_once('-').unwrap_or((core, ""));
    let mut nums = core.split('.');
    let (major, minor, patch) = (
        nums.next().unwrap_or(""),
        nums.next().unwrap_or(""),
        nums.next().unwrap_or(""),
    );
    let napi: Vec<String> = binary
        .get("napi_versions")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_u64().map(|n| n.to_string()))
                .collect()
        })
        .unwrap_or_default();
    let napi_build = (napi.len() == 1).then(|| napi[0].clone());
    let name = pj.get("name").and_then(Value::as_str).unwrap_or("");

    let lookup = |key: &str| -> Option<String> {
        Some(match key {
            "name" => name.to_string(),
            "module_name" => module_name.to_string(),
            "version" => version.to_string(),
            "major" => major.to_string(),
            "minor" => minor.to_string(),
            "patch" => patch.to_string(),
            "prerelease" => prerelease.to_string(),
            "build" => build.to_string(),
            "configuration" => "Release".to_string(),
            "platform" | "target_platform" => platform.node_os().to_string(),
            "arch" | "target_arch" => platform.node_arch().to_string(),
            "libc" => platform.libc.as_str().to_string(),
            "runtime" => "node".to_string(),
            "napi_build_version" | "napi_version" => napi_build.clone()?,
            "node_napi_label" => format!("napi-v{}", napi_build.clone()?),
            "node_abi_napi" if napi_build.is_some() => "napi".to_string(),
            _ => return None,
        })
    };

    let mut unknown = Vec::new();
    let mut expand = |tpl: &str| -> String {
        let mut out = String::new();
        let mut rest = tpl;
        while let Some(open) = rest.find('{') {
            let Some(close) = rest[open..].find('}') else {
                break;
            };
            out.push_str(&rest[..open]);
            let key = &rest[open + 1..open + close];
            match lookup(key) {
                Some(v) => out.push_str(&v),
                None => {
                    if !unknown.iter().any(|u| u == key) {
                        unknown.push(key.to_string());
                    }
                    out.push('*');
                }
            }
            rest = &rest[open + close + 1..];
        }
        out.push_str(rest);
        out
    };
    let dir = expand(module_path);
    let file = expand(module_name);
    let dir = dir.trim_start_matches("./").trim_end_matches('/');
    let pattern = if dir.is_empty() || dir == "." {
        format!("{file}.node")
    } else {
        format!("{dir}/{file}.node")
    };
    Some(ModulePattern { pattern, unknown })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lx() -> Platform {
        Platform::parse("linux/x64").unwrap()
    }

    #[test]
    fn napi_template() {
        let pj = serde_json::json!({
            "name": "bcrypt", "version": "5.1.1",
            "binary": {
                "module_name": "bcrypt_lib",
                "module_path": "./lib/binding/napi-v{napi_build_version}",
                "napi_versions": [3]
            }
        });
        let m = module_pattern(&pj, &lx()).unwrap();
        assert_eq!(m.pattern, "lib/binding/napi-v3/bcrypt_lib.node");
        assert!(m.unknown.is_empty());
    }

    #[test]
    fn abi_template_is_partial() {
        let pj = serde_json::json!({
            "name": "sqlite3", "version": "5.0.0",
            "binary": {
                "module_name": "node_sqlite3",
                "module_path": "./lib/binding/{configuration}/{node_abi}-{platform}-{libc}-{arch}/"
            }
        });
        let m = module_pattern(&pj, &lx()).unwrap();
        assert_eq!(
            m.pattern,
            "lib/binding/Release/*-linux-glibc-x64/node_sqlite3.node"
        );
        assert_eq!(m.unknown, ["node_abi"]);
        assert!(module_pattern(&serde_json::json!({"name": "x"}), &lx()).is_none());
    }
}
