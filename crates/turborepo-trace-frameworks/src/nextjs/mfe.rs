//! Microfrontends routing (`microfrontends.json` / `microfrontends.jsonc`) and
//! the middleware (`middleware.ts` / `proxy.ts`) matcher.
//!
//! A microfrontends config lists, per application, the path patterns it serves.
//! A URL that matches another application's paths is not served by this app:
//! the visit maps to no local route. A group gated by a `flag` may or may not
//! apply, which is an ambiguity.

use std::path::{Path, PathBuf};

use serde::Serialize;

use super::p2r::PathPattern;
use crate::fsx::PathFsx;

/// Config file names.
pub const MFE_NAMES: &[&str] = &["microfrontends.json", "microfrontends.jsonc"];
/// Middleware file names (Next 16 calls it `proxy`).
pub const MIDDLEWARE_NAMES: &[&str] = &[
    "middleware.ts",
    "middleware.js",
    "middleware.mjs",
    "middleware.tsx",
    "middleware.jsx",
    "proxy.ts",
    "proxy.js",
    "proxy.mjs",
    "src/middleware.ts",
    "src/middleware.js",
    "src/proxy.ts",
    "src/proxy.js",
];

/// One path pattern of another application.
#[derive(Clone, Debug, Serialize)]
pub struct MfePath {
    /// The application that serves it.
    pub app: String,
    /// The pattern.
    pub source: String,
    /// The feature flag gating the group, if any.
    pub flag: Option<String>,
    /// Compiled pattern.
    #[serde(skip)]
    pub pattern: Option<PathPattern>,
}

/// The microfrontends config as it concerns this app.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Microfrontends {
    /// The file.
    pub file: Option<PathBuf>,
    /// This app's name in the config.
    pub this_app: Option<String>,
    /// Patterns served by other applications.
    pub others: Vec<MfePath>,
    /// When this app is a child: the patterns it serves (other URLs go
    /// elsewhere).
    pub own: Vec<MfePath>,
    /// Parse problems.
    pub errors: Vec<String>,
}

/// Strips `//` and `/* */` comments and trailing commas from JSONC.
pub fn strip_jsonc(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c == b'"' {
            let start = i;
            i += 1;
            while i < b.len() && b[i] != b'"' {
                if b[i] == b'\\' {
                    i += 1;
                }
                i += 1;
            }
            i = (i + 1).min(b.len());
            out.push_str(&s[start..i]);
            continue;
        }
        if c == b'/' && b.get(i + 1) == Some(&b'/') {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if c == b'/' && b.get(i + 1) == Some(&b'*') {
            i += 2;
            while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
                i += 1;
            }
            i += 2;
            continue;
        }
        if c == b',' {
            let mut j = i + 1;
            while j < b.len() && (b[j] as char).is_whitespace() {
                j += 1;
            }
            if matches!(b.get(j), Some(b'}' | b']')) {
                i += 1;
                continue;
            }
        }
        let ch = s[i..].chars().next().unwrap_or(' ');
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

impl Microfrontends {
    /// Loads the config next to the app (or in an ancestor up to `stop`), for
    /// the app whose `package.json` name is `pkg_name`.
    pub fn load(app_root: &Path, pkg_name: Option<&str>) -> Microfrontends {
        let mut m = Microfrontends::default();
        let Some(file) = crate::fsx::first_existing(app_root, MFE_NAMES) else {
            return m;
        };
        m.file = Some(file.clone());
        let text = match crate::fsx::read_to_string(&file) {
            Ok(t) => t,
            Err(e) => {
                m.errors.push(e.to_string());
                return m;
            }
        };
        let v: serde_json::Value = match serde_json::from_str(&strip_jsonc(&text)) {
            Ok(v) => v,
            Err(e) => {
                m.errors.push(format!("parse error: {e}"));
                return m;
            }
        };
        let Some(apps) = v.get("applications").and_then(|a| a.as_object()) else {
            return m;
        };
        // This app: by package name, else the default app (the one without routing).
        let this = apps
            .iter()
            .find(|(k, d)| {
                pkg_name.is_some_and(|n| {
                    *k == n || d.get("packageName").and_then(|x| x.as_str()) == Some(n)
                })
            })
            .or_else(|| apps.iter().find(|(_, d)| d.get("routing").is_none()))
            .map(|(k, _)| k.clone());
        m.this_app = this.clone();
        for (name, def) in apps {
            let Some(groups) = def.get("routing").and_then(|r| r.as_array()) else {
                continue;
            };
            for g in groups {
                let flag = g.get("flag").and_then(|f| f.as_str()).map(String::from);
                for p in g
                    .get("paths")
                    .and_then(|p| p.as_array())
                    .into_iter()
                    .flatten()
                {
                    let Some(src) = p.as_str() else { continue };
                    let pattern = PathPattern::new(src).ok();
                    if pattern.is_none() {
                        m.errors.push(format!("unsupported path pattern {src}"));
                    }
                    let mp = MfePath {
                        app: name.clone(),
                        source: src.to_string(),
                        flag: flag.clone(),
                        pattern,
                    };
                    if Some(name) == this.as_ref() {
                        m.own.push(mp);
                    } else {
                        m.others.push(mp);
                    }
                }
            }
        }
        m
    }
}

/// The middleware files of an app (existing ones), and its `config.matcher`
/// sources if they could be read.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Middleware {
    /// Files.
    pub files: Vec<PathBuf>,
    /// `config.matcher` entries (`None`: no matcher, or unreadable: every
    /// path).
    pub matcher: Option<Vec<String>>,
}

impl Middleware {
    /// Finds the middleware of the app at `root`.
    pub fn load(root: &Path) -> Middleware {
        let files: Vec<PathBuf> = MIDDLEWARE_NAMES
            .iter()
            .map(|n| root.join(n))
            .filter(|p| p.fx_is_file())
            .collect();
        let mut matcher = None;
        if let Some(f) = files.first() {
            use turborepo_trace_lang_js::extract::config::{ConfigVal, NoLoad, eval_config};
            let src = crate::fsx::read(f).unwrap_or_default();
            let m = eval_config(
                &src,
                turborepo_trace_lang_js::Dialect::from_path(f),
                &f.to_string_lossy(),
                &mut NoLoad,
            );
            let v = m.import_named("config");
            if !v.is_null() {
                match v.field("matcher") {
                    Some(ConfigVal::Arr(xs)) => {
                        let s: Option<Vec<String>> = xs
                            .iter()
                            .map(|x| {
                                x.as_str().map(String::from).or_else(|| {
                                    x.field("source").and_then(|s| s.as_str().map(String::from))
                                })
                            })
                            .collect();
                        matcher = s;
                    }
                    Some(x) => matcher = x.as_str().map(|s| vec![s.to_string()]),
                    None => {}
                }
            }
        }
        Middleware { files, matcher }
    }

    /// Whether the middleware may run for `path` (a rendered path).
    pub fn runs_for(&self, path: &str) -> bool {
        if self.files.is_empty() {
            return false;
        }
        match &self.matcher {
            None => true,
            Some(ms) => ms.iter().any(|m| match PathPattern::new(m) {
                Ok(p) => p.matches(path).is_some(),
                Err(_) => true,
            }),
        }
    }
}
