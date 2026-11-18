//! Compose files (`compose.yaml`, `docker-compose.yml`,
//! `docker-compose.<env>.yml`): one task per service with a `build:` section,
//! `docker:<compose>#<service>`, plus `docker:<compose>` for the whole file.
//!
//! `build` (a context string, or `context`, `dockerfile`, `dockerfile_inline`,
//! `args`, `target`, `additional_contexts`) decides how the service's
//! Dockerfile is analyzed: relative to the compose file's directory, with
//! variable interpolation (`${VAR}`, `${VAR:-default}`, `${VAR-default}`, `$$`)
//! from the `.env` next to the compose file (a config, a probe when missing).
//! YAML anchors and merge keys (`<<: *x`) are applied. `extends` and top-level
//! `include` are not followed (uncertainty).
//!
//! The file task also holds what configures the services without being built
//! into an image: bind mounts of repository paths (`volumes`), `env_file`, and
//! the files of top-level `configs` and `secrets`. A command that starts the
//! project (`docker compose up`) names the file task as a service of its task
//! ([`crate::spec::TestRun::Service`]).

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use serde_yaml_ng::Value;
use turborepo_trace_core::UncertaintyKind;
use turborepo_trace_frameworks::fsx::{self, PathFsx};

use super::dockerfile::BuildVars;
use crate::{
    sources::{DiscoveredTask, Workspace},
    spec::{TaskId, TaskRef, TaskSpec},
};

/// `KEY=VALUE` lines of a `.env` file.
fn dotenv(src: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for l in src.lines() {
        let t = l.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        let t = t.strip_prefix("export ").unwrap_or(t);
        if let Some((k, v)) = t.split_once('=') {
            let v = v.trim();
            let v = v
                .strip_prefix('"')
                .and_then(|x| x.strip_suffix('"'))
                .or_else(|| v.strip_prefix('\'').and_then(|x| x.strip_suffix('\'')))
                .unwrap_or(v);
            out.insert(k.trim().to_string(), v.to_string());
        }
    }
    out
}

/// Compose interpolation. Returns the text and the unknown variable names.
pub fn interpolate(s: &str, env: &BTreeMap<String, String>) -> (String, Vec<String>) {
    let mut out = String::with_capacity(s.len());
    let mut unknown = Vec::new();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c != '$' {
            out.push(c);
            i += 1;
            continue;
        }
        match chars.get(i + 1) {
            Some('$') => {
                out.push('$');
                i += 2;
            }
            Some('{') => {
                let Some(end) = chars[i + 2..].iter().position(|&x| x == '}') else {
                    out.extend(&chars[i..]);
                    break;
                };
                let inner: String = chars[i + 2..i + 2 + end].iter().collect();
                let n = inner
                    .bytes()
                    .take_while(|b| b.is_ascii_alphanumeric() || *b == b'_')
                    .count();
                let (name, m) = (&inner[..n], &inner[n..]);
                let v = env.get(name);
                let r = if let Some(d) = m.strip_prefix(":-") {
                    v.filter(|x| !x.is_empty())
                        .cloned()
                        .unwrap_or_else(|| d.to_string())
                } else if let Some(d) = m.strip_prefix('-') {
                    v.cloned().unwrap_or_else(|| d.to_string())
                } else if let Some(a) = m.strip_prefix(":+") {
                    if v.is_some_and(|x| !x.is_empty()) {
                        a.to_string()
                    } else {
                        String::new()
                    }
                } else if let Some(a) = m.strip_prefix('+') {
                    if v.is_some() {
                        a.to_string()
                    } else {
                        String::new()
                    }
                } else {
                    match v {
                        Some(x) => x.clone(),
                        None => {
                            unknown.push(name.to_string());
                            format!("${{{name}}}")
                        }
                    }
                };
                out.push_str(&r);
                i += 3 + end;
            }
            Some(&a) if a.is_ascii_alphabetic() || a == '_' => {
                let mut j = i + 1;
                while j < chars.len() && (chars[j].is_ascii_alphanumeric() || chars[j] == '_') {
                    j += 1;
                }
                let name: String = chars[i + 1..j].iter().collect();
                match env.get(&name) {
                    Some(x) => out.push_str(x),
                    None => {
                        out.push_str(&format!("${{{name}}}"));
                        unknown.push(name);
                    }
                }
                i = j;
            }
            _ => {
                out.push('$');
                i += 1;
            }
        }
    }
    (out, unknown)
}

fn scalar(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// The tasks of compose file `file`.
pub fn tasks(ws: &Workspace, file: &Path) -> Vec<DiscoveredTask> {
    let rel = ws.rel(file);
    let dir = file.parent().map(Path::to_path_buf).unwrap_or_default();
    let env_file = dir.join(".env");
    let env = fsx::read_to_string(&env_file)
        .map(|s| dotenv(&s))
        .unwrap_or_default();
    let mut file_task =
        DiscoveredTask::new(TaskId::docker(rel.clone(), None), rel.clone(), dir.clone());
    file_task.files.push(file.to_path_buf());
    file_task.spec.config(env_file.clone());
    let parsed: Option<Value> = fsx::read_to_string(file)
        .ok()
        .and_then(|s| serde_yaml_ng::from_str::<Value>(&s).ok())
        .map(|mut v| {
            let _ = v.apply_merge();
            v
        });
    let Some(root) = parsed else {
        file_task.spec.uncertain(
            UncertaintyKind::ParseIncomplete,
            format!("unreadable compose file {rel}"),
        );
        file_task
            .spec
            .membership(dir, "**", "unreadable compose file");
        return vec![file_task];
    };
    let mut out = Vec::new();
    if root.get("include").is_some() {
        file_task.spec.uncertain(
            UncertaintyKind::UnsupportedSyntax,
            "compose `include` not followed",
        );
        file_task
            .spec
            .membership(dir.clone(), "**", "compose include");
    }
    let Some(services) = root.get("services").and_then(Value::as_mapping) else {
        return Vec::new();
    };
    // Files of the repository the project is configured with: inputs of the whole
    // file.
    for key in ["configs", "secrets"] {
        for def in root
            .get(key)
            .and_then(Value::as_mapping)
            .into_iter()
            .flat_map(|m| m.values())
        {
            if let Some(f) = def.get("file").and_then(scalar) {
                host_path(
                    &f,
                    &dir,
                    &env,
                    &format!("compose {key}"),
                    &mut file_task.spec,
                );
            }
        }
    }
    for (name, svc) in services {
        let Some(name) = scalar(name) else { continue };
        service_files(svc, &dir, &env, &mut file_task.spec);
        let Some(build) = svc.get("build") else {
            continue;
        };
        let mut t = DiscoveredTask::new(
            TaskId::docker(rel.clone(), Some(name.clone())),
            name.clone(),
            dir.clone(),
        );
        t.files.push(file.to_path_buf());
        t.spec.config(env_file.clone());
        let mut spec = TaskSpec::default();
        let mut unknown: Vec<String> = Vec::new();
        let interp = |s: &str, u: &mut Vec<String>| {
            let (r, x) = interpolate(s, &env);
            u.extend(x);
            r
        };
        let (ctx_s, dockerfile, inline, target, args) = match build {
            Value::String(s) => (interp(s, &mut unknown), None, None, None, BuildVars::new()),
            Value::Mapping(_) => {
                let ctx_s = build
                    .get("context")
                    .and_then(scalar)
                    .map(|s| interp(&s, &mut unknown))
                    .unwrap_or_else(|| ".".into());
                let df = build
                    .get("dockerfile")
                    .and_then(scalar)
                    .map(|s| interp(&s, &mut unknown));
                let inline = build.get("dockerfile_inline").and_then(scalar);
                let target = build
                    .get("target")
                    .and_then(scalar)
                    .map(|s| interp(&s, &mut unknown));
                let mut args = BuildVars::new();
                match build.get("args") {
                    Some(Value::Mapping(m)) => {
                        for (k, v) in m {
                            if let Some(k) = scalar(k) {
                                let v = match scalar(v) {
                                    Some(v) => Some(interp(&v, &mut unknown)),
                                    // `KEY:` without a value: from the environment.
                                    None => env.get(&k).cloned(),
                                };
                                args.insert(k, v);
                            }
                        }
                    }
                    Some(Value::Sequence(s)) => {
                        for v in s.iter().filter_map(scalar) {
                            match v.split_once('=') {
                                Some((k, v)) => {
                                    args.insert(k.to_string(), Some(interp(v, &mut unknown)))
                                }
                                None => args.insert(v.clone(), env.get(&v).cloned()),
                            };
                        }
                    }
                    _ => {}
                }
                if let Some(Value::Mapping(m)) = build.get("additional_contexts") {
                    for v in m.values().filter_map(scalar) {
                        if v.contains("://")
                            || v.starts_with("docker-image:")
                            || v.starts_with("service:")
                        {
                            continue;
                        }
                        let v = interp(&v, &mut unknown);
                        spec.membership(fsx::join(&dir, &v), "**", "compose additional_contexts");
                    }
                }
                (ctx_s, df, inline, target, args)
            }
            _ => {
                spec.uncertain(
                    UncertaintyKind::ParseIncomplete,
                    format!("compose service `{name}`: unreadable build"),
                );
                spec.membership(dir.clone(), "**", "unreadable compose build");
                t.spec.merge(spec);
                out.push(t);
                continue;
            }
        };
        if svc.get("extends").is_some() {
            spec.uncertain(
                UncertaintyKind::UnsupportedSyntax,
                format!("compose service `{name}`: extends not followed"),
            );
            spec.membership(dir.clone(), "**", "compose extends");
        }
        if !unknown.is_empty() {
            unknown.sort();
            unknown.dedup();
            spec.uncertain(
                UncertaintyKind::EnvDependent,
                format!("compose service `{name}`: unknown {}", unknown.join(", ")),
            );
            spec.membership(dir.clone(), "**", "compose interpolation");
        }
        if ctx_s.contains("://") || ctx_s.starts_with("git@") {
            spec.tool("url", ctx_s);
            t.spec.merge(spec);
            out.push(t);
            continue;
        }
        let ctx: PathBuf = fsx::join(&dir, &ctx_s);
        let df_path = fsx::join(&ctx, dockerfile.as_deref().unwrap_or("Dockerfile"));
        let a = super::analyze(ws, &df_path, inline.as_deref(), Some(&ctx), &args);
        if a.missing {
            spec.input(df_path.clone());
            spec.uncertain(
                UncertaintyKind::UnresolvedSpecifier,
                format!("compose service `{name}`: missing {}", ws.rel(&df_path)),
            );
            spec.membership(ctx.clone(), "**", "missing Dockerfile");
        } else {
            match a.stage_index(target.as_deref()) {
                Some(idx) => spec.merge(a.closure_spec(idx)),
                None => {
                    spec.config(df_path.clone());
                    spec.uncertain(
                        UncertaintyKind::UnresolvedSpecifier,
                        format!(
                            "compose service `{name}`: target {} not found",
                            target.as_deref().unwrap_or("(final)")
                        ),
                    );
                    spec.membership(ctx.clone(), "**", "unknown target");
                }
            }
        }
        if inline.is_some() {
            spec.configs.retain(|c| c != &df_path);
        }
        t.spec.merge(spec);
        file_task.spec.dep(
            TaskRef::Id {
                id: TaskId::docker(rel.clone(), Some(name.clone())),
            },
            "compose service",
        );
        out.push(t);
    }
    // A project of third-party images only is still something a command starts
    // (`docker compose up`): its file, env file and mounted files define the
    // services.
    out.push(file_task);
    out
}

/// Adds the host path `raw` of a compose file in `dir` (a bind mount source, an
/// env file, a config or secret file) to `spec`: a file is an input, a
/// directory a membership. Named volumes, absolute and home-relative paths and
/// paths with unknown variables are not repository files.
fn host_path(
    raw: &str,
    dir: &Path,
    env: &BTreeMap<String, String>,
    reason: &str,
    spec: &mut TaskSpec,
) {
    let (s, unknown) = interpolate(raw, env);
    if !unknown.is_empty() {
        return;
    }
    if !(s == "." || s == ".." || s.starts_with("./") || s.starts_with("../")) {
        return;
    }
    let p = fsx::join(dir, &s);
    if p.fx_is_dir() {
        spec.membership(p, "**", reason);
    } else {
        spec.input(p);
    }
}

/// The repository files one service is configured with: bind mounts (`volumes`,
/// short `./host:/container[:ro]` and long `{type: bind, source: ./host}`
/// syntax) and `env_file`.
fn service_files(svc: &Value, dir: &Path, env: &BTreeMap<String, String>, spec: &mut TaskSpec) {
    for v in svc
        .get("volumes")
        .and_then(Value::as_sequence)
        .into_iter()
        .flatten()
    {
        match v {
            Value::String(s) => {
                // A name without a path separator is a named volume.
                if let Some(host) = s.split(':').next() {
                    host_path(host, dir, env, "compose bind mount", spec);
                }
            }
            Value::Mapping(_) => {
                let bind = v.get("type").and_then(scalar).is_none_or(|t| t == "bind");
                if let (true, Some(src)) = (bind, v.get("source").and_then(scalar)) {
                    host_path(&src, dir, env, "compose bind mount", spec);
                }
            }
            _ => {}
        }
    }
    let env_files: Vec<String> = match svc.get("env_file") {
        Some(Value::Sequence(xs)) => xs
            .iter()
            .filter_map(|x| scalar(x).or_else(|| x.get("path").and_then(scalar)))
            .collect(),
        Some(x) => scalar(x).into_iter().collect(),
        None => Vec::new(),
    };
    for f in env_files {
        // An env file is named relative to the compose file, with or without `./`.
        let f = if f.starts_with('/') || f.starts_with('~') || f.starts_with('.') {
            f
        } else {
            format!("./{f}")
        };
        host_path(&f, dir, env, "compose env_file", spec);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interpolation() {
        let mut env = BTreeMap::new();
        env.insert("APP".to_string(), "web".to_string());
        env.insert("EMPTY".to_string(), String::new());
        assert_eq!(interpolate("apps/${APP}", &env).0, "apps/web");
        assert_eq!(
            interpolate("${EMPTY:-d} ${EMPTY-d} ${NOPE:-x} $$HOME", &env).0,
            "d  x $HOME"
        );
        assert_eq!(interpolate("$NOPE", &env).1, ["NOPE"]);
    }

    #[test]
    fn dotenv_lines() {
        let e = dotenv("# c\nA=1\nexport B=\"two\"\nC='3'\n");
        assert_eq!(e["A"], "1");
        assert_eq!(e["B"], "two");
        assert_eq!(e["C"], "3");
    }
}
