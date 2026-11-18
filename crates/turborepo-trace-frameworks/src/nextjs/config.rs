//! Reading `next.config.*` with the lang-js config evaluator.
//!
//! The evaluator turns a config module into a value tree, but it only calls the
//! exported config function itself; `async redirects()` and `async rewrites()`
//! stay opaque function values that only the defining module can call. So the
//! loader evaluates a lightly rewritten copy of the source: the default export
//! (`export default x` / `module.exports = x`) is bound to a local, and
//! appended statements call the methods in the same module:
//!
//! ```js
//! const __dt_default = x;            // was: export default x
//! export const __dt_cfg = __dt_default;
//! export const __dt_redirects = __dt_cfg.redirects();
//! export const __dt_rewrites = __dt_cfg.rewrites();
//! ```
//!
//! Plugin wrappers (`withSentryConfig(nextConfig)`,
//! `bundleAnalyzer(withMicrofrontends(c))`) evaluate to unknown values. Then
//! the loader looks for the top-level variable that holds the plain config
//! object (most Next config keys) and reads that, with an uncertainty
//! saying that wrappers are assumed to keep `redirects`/`rewrites`/`basePath`.
//!
//! Rules that depend on the environment (`process.env.X ? [...] : []`) arrive
//! as alternatives: every alternative's rules are kept and marked
//! [`Rule::conditional`]. Rules that cannot be read make
//! [`NextConfig::rules_complete`] false.
//!
//! [`NextConfig::from_routes_manifest`] reads the evaluated rules of a build
//! (`.next/routes-manifest.json`) instead.

use std::{
    cell::RefCell,
    path::{Path, PathBuf},
};

use serde::Serialize;
use turborepo_trace_core::UncertaintyKind;
use turborepo_trace_lang_js::{
    Dialect,
    extract::config::{ConfigHost, ConfigModule, ConfigNote, ConfigVal, NoteKind, eval_config},
    lexer::{Kw, Lexer, TokKind},
};

use super::p2r::PathPattern;
use crate::{fsx, inputs::FrameworkUncertainty, jsconfig};

/// `next.config` file names, in Next's lookup order.
pub const CONFIG_NAMES: &[&str] = &[
    "next.config.js",
    "next.config.mjs",
    "next.config.ts",
    "next.config.mts",
    "next.config.cjs",
    "next.config.cts",
];

/// Keys that identify a Next config object.
const NEXT_KEYS: &[&str] = &[
    "redirects",
    "rewrites",
    "headers",
    "basePath",
    "i18n",
    "pageExtensions",
    "images",
    "experimental",
    "reactStrictMode",
    "trailingSlash",
    "transpilePackages",
    "env",
    "output",
    "turbopack",
    "webpack",
    "serverExternalPackages",
    "typescript",
    "eslint",
    "compiler",
    "poweredByHeader",
];

/// A `has` / `missing` condition.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Cond {
    /// `header`, `cookie`, `query` or `host`.
    #[serde(rename = "type")]
    pub kind: String,
    /// The key (absent for `host`).
    pub key: Option<String>,
    /// The value regex, if any.
    pub value: Option<String>,
}

/// One redirect or rewrite.
#[derive(Clone, Debug, Serialize)]
pub struct Rule {
    /// `source`.
    pub source: String,
    /// `destination`.
    pub destination: String,
    /// `has` conditions.
    pub has: Vec<Cond>,
    /// `missing` conditions.
    pub missing: Vec<Cond>,
    /// `basePath: false` (the source is not prefixed with the base path).
    pub no_base_path: bool,
    /// `locale: false`.
    pub no_locale: bool,
    /// The rule exists only in some environments (it came from one of several
    /// alternatives).
    pub conditional: bool,
    /// The compiled source, if supported.
    #[serde(skip)]
    pub pattern: Option<PathPattern>,
}

/// The rewrite phases.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Rewrites {
    /// `beforeFiles`.
    pub before_files: Vec<Rule>,
    /// `afterFiles` (also a plain array).
    pub after_files: Vec<Rule>,
    /// `fallback`.
    pub fallback: Vec<Rule>,
}

/// `i18n`.
#[derive(Clone, Debug, Default, Serialize)]
pub struct I18n {
    /// Locales.
    pub locales: Vec<String>,
    /// Default locale.
    pub default_locale: Option<String>,
}

/// What the loader read from `next.config.*`.
#[derive(Clone, Debug, Default, Serialize)]
pub struct NextConfig {
    /// The config file.
    pub file: Option<PathBuf>,
    /// Every file read (config and local helpers).
    pub files: Vec<PathBuf>,
    /// The code files among them.
    pub code_files: Vec<PathBuf>,
    /// `basePath`.
    pub base_path: Option<String>,
    /// `i18n`.
    pub i18n: Option<I18n>,
    /// `pageExtensions` (all alternatives united).
    pub page_extensions: Vec<String>,
    /// `trailingSlash`.
    pub trailing_slash: bool,
    /// `redirects()`.
    pub redirects: Vec<Rule>,
    /// `rewrites()`.
    pub rewrites: Rewrites,
    /// Whether every redirect and rewrite rule was read.
    pub rules_complete: bool,
    /// Where the rules came from: `config`, `routes-manifest` or `none`.
    pub rules_source: &'static str,
    /// The config object was wrapped by these unknown calls.
    pub wrapped: bool,
    /// Problems.
    pub uncertainties: Vec<FrameworkUncertainty>,
}

/// The `next.config.*` in `dir`, if any.
pub fn find_config(dir: &Path) -> Option<PathBuf> {
    fsx::first_existing(dir, CONFIG_NAMES)
}

#[derive(Default)]
struct State {
    files: Vec<PathBuf>,
    code_files: Vec<PathBuf>,
    notes: Vec<(PathBuf, ConfigNote)>,
}

struct Host<'a> {
    dir: PathBuf,
    depth_left: u32,
    state: &'a RefCell<State>,
}

impl ConfigHost for Host<'_> {
    fn load(&mut self, spec: &str) -> Result<ConfigModule, String> {
        let base = fsx::join(&self.dir, spec);
        let file = jsconfig::resolve_local(&base).ok_or_else(|| "not found".to_string())?;
        self.state.borrow_mut().files.push(file.clone());
        if self.depth_left == 0 {
            return Err("helper depth limit".into());
        }
        let src = crate::fsx::read(&file).map_err(|e| format!("unreadable: {e}"))?;
        Ok(eval_file(&file, &src, self.depth_left - 1, self.state))
    }
}

fn eval_file(file: &Path, src: &[u8], depth_left: u32, state: &RefCell<State>) -> ConfigModule {
    let is_json = file.extension().is_some_and(|e| e == "json");
    if !is_json {
        state.borrow_mut().code_files.push(file.to_path_buf());
    }
    let mut host = Host {
        dir: file.parent().unwrap_or(Path::new("/")).to_path_buf(),
        depth_left,
        state,
    };
    let path = file.to_string_lossy().into_owned();
    let mut m = if is_json {
        let mut w = b"module.exports = ".to_vec();
        w.extend_from_slice(src);
        eval_config(&w, Dialect::JS, &path, &mut host)
    } else {
        eval_config(src, Dialect::from_path(file), &path, &mut host)
    };
    let notes = std::mem::take(&mut m.notes);
    state
        .borrow_mut()
        .notes
        .extend(notes.into_iter().map(|n| (file.to_path_buf(), n)));
    m
}

/// The source with its default export bound to `__dt_default` instead, and the
/// names of top-level `const`/`let`/`var` bindings (candidates for the plain
/// config object).
fn rewrite_default(src: &[u8], dialect: Dialect) -> (Vec<u8>, Vec<String>, bool) {
    let mut lx = Lexer::new(src, dialect);
    let mut toks = Vec::new();
    loop {
        let t = lx.next_token();
        if t.kind == TokKind::Eof || toks.len() > 400_000 {
            break;
        }
        if t.kind == TokKind::TripleSlash {
            continue;
        }
        toks.push(t);
    }
    let text = |i: usize| -> &[u8] { toks.get(i).map(|t| t.text(src)).unwrap_or_default() };
    let mut names = Vec::new();
    let mut replace: Option<(usize, usize)> = None;
    for i in 0..toks.len() {
        let t = &toks[i];
        if t.depth != 0 {
            continue;
        }
        if matches!(t.kw, Kw::Const | Kw::Let | Kw::Var)
            && toks.get(i + 1).is_some_and(|n| n.kind == TokKind::Ident)
            && toks
                .get(i + 2)
                .is_some_and(|n| n.kind == TokKind::Assign || n.kind == TokKind::Colon)
        {
            let n = String::from_utf8_lossy(text(i + 1)).into_owned();
            if !names.contains(&n) {
                names.push(n);
            }
        }
        if replace.is_none() {
            if t.kw == Kw::Export && toks.get(i + 1).is_some_and(|n| n.kw == Kw::Default) {
                replace = Some((t.start as usize, toks[i + 1].end as usize));
            } else if text(i) == b"module"
                && toks.get(i + 1).is_some_and(|n| n.kind == TokKind::Dot)
                && text(i + 2) == b"exports"
                && toks.get(i + 3).is_some_and(|n| n.kind == TokKind::Assign)
            {
                replace = Some((t.start as usize, toks[i + 3].end as usize));
            }
        }
    }
    match replace {
        Some((a, b)) => {
            let mut out = src[..a].to_vec();
            out.extend_from_slice(b"const __dt_default =");
            out.extend_from_slice(&src[b..]);
            (out, names, true)
        }
        None => (src.to_vec(), names, false),
    }
}

fn next_key_score(v: &ConfigVal) -> usize {
    v.alternatives()
        .iter()
        .map(|a| match a {
            ConfigVal::Obj(o) => o
                .entries
                .iter()
                .filter(|(k, _)| NEXT_KEYS.contains(&k.as_str()))
                .count(),
            _ => 0,
        })
        .max()
        .unwrap_or(0)
}

fn named<'m>(m: &'m ConfigModule, n: &str) -> Option<&'m ConfigVal> {
    m.named.iter().find(|(k, _)| k == n).map(|(_, v)| v)
}

impl NextConfig {
    /// Loads `next.config.*` from `dir` (defaults if there is none).
    pub fn load(dir: &Path) -> NextConfig {
        match find_config(dir) {
            Some(f) => NextConfig::load_file(&f),
            None => NextConfig {
                rules_complete: true,
                rules_source: "none",
                ..NextConfig::default()
            },
        }
    }

    /// Loads one config file.
    pub fn load_file(file: &Path) -> NextConfig {
        let mut out = NextConfig {
            file: Some(file.to_path_buf()),
            files: vec![file.to_path_buf()],
            rules_complete: true,
            rules_source: "config",
            ..NextConfig::default()
        };
        let src = match crate::fsx::read(file) {
            Ok(s) => s,
            Err(e) => {
                out.rules_complete = false;
                out.unc(
                    file,
                    UncertaintyKind::ParseIncomplete,
                    None,
                    format!("unreadable: {e}"),
                );
                return out;
            }
        };
        let dialect = Dialect::from_path(file);
        let (rewritten, names, found) = rewrite_default(&src, dialect);
        if !found {
            out.rules_complete = false;
            out.unc(
                file,
                UncertaintyKind::UnsupportedSyntax,
                None,
                "no default export found".into(),
            );
            return out;
        }
        // Pass 1: the exported value and the candidates.
        let mut probe = rewritten.clone();
        probe.extend_from_slice(b"\n;export const __dt_cfg = __dt_default;\n");
        for n in &names {
            probe.extend_from_slice(format!("export const __dt_cand_{n} = {n};\n").as_bytes());
        }
        let state = RefCell::new(State::default());
        let m = eval_file(file, &probe, 2, &state);
        let cfg = named(&m, "__dt_cfg").cloned().unwrap_or(ConfigVal::Null);
        let mut target = "__dt_default".to_string();
        if matches!(cfg, ConfigVal::Function(_)) {
            target = "__dt_default(\"phase-production-build\", { defaultConfig: {} })".into();
        } else if next_key_score(&cfg) == 0 {
            let best = names
                .iter()
                .filter_map(|n| {
                    let v = named(&m, &format!("__dt_cand_{n}"))?;
                    let s = next_key_score(v);
                    (s > 0).then_some((s, n))
                })
                .max_by_key(|(s, _)| *s);
            match best {
                Some((_, n)) => {
                    out.wrapped = true;
                    out.unc(
                        file,
                        UncertaintyKind::UnsupportedSyntax,
                        None,
                        format!(
                            "the exported config is wrapped by plugin calls; read `{n}` and \
                             assumed the wrappers keep basePath, i18n, redirects and rewrites"
                        ),
                    );
                    target = n.clone();
                }
                None if cfg.is_null() => {}
                None => {
                    out.rules_complete = false;
                    out.unc(
                        file,
                        UncertaintyKind::UnsupportedSyntax,
                        None,
                        "the exported config could not be evaluated".into(),
                    );
                }
            }
        }
        // Pass 2: call the methods.
        let mut full = rewritten;
        full.extend_from_slice(
            format!(
                "\n;const __dt_c0 = {target};\nexport const __dt_cfg = __dt_c0;\nexport const \
                 __dt_redirects = __dt_c0.redirects();\nexport const __dt_rewrites = \
                 __dt_c0.rewrites();\n"
            )
            .as_bytes(),
        );
        let state = RefCell::new(State::default());
        let m = eval_file(file, &full, 2, &state);
        let st = state.into_inner();
        for f in st.files.iter().chain(&st.code_files) {
            if !out.files.contains(f) {
                out.files.push(f.clone());
            }
        }
        out.code_files = st.code_files.clone();
        out.code_files.dedup();
        for (f, n) in st.notes {
            if matches!(n.kind, NoteKind::NotFollowed | NoteKind::Truncated) {
                out.unc(&f, UncertaintyKind::UnresolvedSpecifier, None, n.detail);
            }
        }
        let cfg = named(&m, "__dt_cfg").cloned().unwrap_or(ConfigVal::Null);
        out.read_fields(file, &cfg);
        let has = |k: &str| {
            cfg.alternatives()
                .iter()
                .any(|a| a.field(k).is_some_and(|v| !v.is_null()))
        };
        if has("redirects") {
            let v = named(&m, "__dt_redirects")
                .cloned()
                .unwrap_or(ConfigVal::Null);
            out.redirects = out.rules(file, "redirects", &v, false);
        }
        if has("rewrites") {
            let v = named(&m, "__dt_rewrites")
                .cloned()
                .unwrap_or(ConfigVal::Null);
            out.read_rewrites(file, &v);
        }
        out
    }

    fn unc(&mut self, file: &Path, kind: UncertaintyKind, field: Option<&str>, detail: String) {
        self.uncertainties.push(FrameworkUncertainty {
            kind,
            file: file.to_path_buf(),
            line: 0,
            field: field.map(String::from),
            detail,
        });
    }

    fn read_fields(&mut self, file: &Path, cfg: &ConfigVal) {
        if let Some(bp) = cfg.field("basePath") {
            let alts: Vec<&ConfigVal> = bp.alternatives();
            let known: Vec<&str> = alts.iter().filter_map(|a| a.as_str()).collect();
            if known.len() == 1 && alts.len() == 1 {
                if !known[0].is_empty() {
                    self.base_path = Some(known[0].to_string());
                }
            } else if !bp.is_null() {
                self.unc(
                    file,
                    UncertaintyKind::EnvDependent,
                    Some("basePath"),
                    "basePath is not a single known string".into(),
                );
                self.base_path = known.first().map(|s| s.to_string());
            }
        }
        if let Some(i) = cfg.field("i18n").filter(|v| !v.is_null()) {
            let locales: Vec<String> = match i.field("locales") {
                Some(ConfigVal::Arr(xs)) => xs
                    .iter()
                    .filter_map(|x| x.as_str().map(String::from))
                    .collect(),
                _ => Vec::new(),
            };
            let default_locale = i
                .field("defaultLocale")
                .and_then(|v| v.as_str().map(String::from));
            self.i18n = Some(I18n {
                locales,
                default_locale,
            });
        }
        if let Some(pe) = cfg.field("pageExtensions") {
            for alt in pe.alternatives() {
                if let ConfigVal::Arr(xs) = alt {
                    for x in xs {
                        for a in x.alternatives() {
                            if let Some(s) = a.as_str()
                                && !self.page_extensions.iter().any(|e| e == s)
                            {
                                self.page_extensions.push(s.to_string());
                            }
                        }
                    }
                }
            }
        }
        if let Some(ConfigVal::Bool(b)) = cfg.field("trailingSlash") {
            self.trailing_slash = b;
        }
    }

    fn read_rewrites(&mut self, file: &Path, v: &ConfigVal) {
        for alt in v.alternatives() {
            let conditional = v.alternatives().len() > 1;
            match alt {
                ConfigVal::Arr(_) => {
                    let r = self.rules(file, "rewrites", alt, conditional);
                    self.rewrites.after_files.extend(r);
                }
                ConfigVal::Obj(_) => {
                    for (k, phase) in [("beforeFiles", 0), ("afterFiles", 1), ("fallback", 2)] {
                        if let Some(x) = alt.field(k) {
                            let r = self.rules(file, &format!("rewrites.{k}"), &x, conditional);
                            match phase {
                                0 => self.rewrites.before_files.extend(r),
                                1 => self.rewrites.after_files.extend(r),
                                _ => self.rewrites.fallback.extend(r),
                            }
                        }
                    }
                }
                ConfigVal::Null => {}
                other => {
                    self.rules_complete = false;
                    let mut rs = Vec::new();
                    other.unknown_reasons(&mut rs);
                    self.unc(
                        file,
                        UncertaintyKind::UnsupportedSyntax,
                        Some("rewrites"),
                        format!("rewrites not readable: {}", rs.join("; ")),
                    );
                }
            }
        }
    }

    fn rules(&mut self, file: &Path, field: &str, v: &ConfigVal, conditional: bool) -> Vec<Rule> {
        let mut out = Vec::new();
        let alts = v.alternatives();
        let cond = conditional || alts.len() > 1;
        for alt in alts {
            match alt {
                ConfigVal::Arr(xs) => {
                    for x in xs {
                        let xa = x.alternatives();
                        let c2 = cond || xa.len() > 1;
                        for el in xa {
                            match rule_of(el, c2) {
                                Some(r) => out.push(r),
                                None if el.is_null() => {}
                                None => {
                                    self.rules_complete = false;
                                    let mut rs = Vec::new();
                                    el.unknown_reasons(&mut rs);
                                    let detail = if rs.is_empty() {
                                        "unreadable rule".to_string()
                                    } else {
                                        rs.join("; ")
                                    };
                                    self.unc(
                                        file,
                                        UncertaintyKind::UnsupportedSyntax,
                                        Some(field),
                                        detail,
                                    );
                                }
                            }
                        }
                    }
                }
                ConfigVal::Null => {}
                other => {
                    self.rules_complete = false;
                    let mut rs = Vec::new();
                    other.unknown_reasons(&mut rs);
                    self.unc(
                        file,
                        UncertaintyKind::UnsupportedSyntax,
                        Some(field),
                        format!("not an array: {}", rs.join("; ")),
                    );
                }
            }
        }
        for r in &out {
            if r.pattern.is_none() {
                self.rules_complete = false;
            }
        }
        out
    }

    /// Reads the evaluated rules of a build (`routes-manifest.json`) into
    /// `self`, replacing the rules from the config file. Dev-only rules
    /// that proxy to `localhost` are dropped.
    pub fn use_routes_manifest(&mut self, manifest: &Path) -> Result<(), String> {
        let bytes = crate::fsx::read(manifest).map_err(|e| e.to_string())?;
        let v: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        let conv = |list: Option<&serde_json::Value>| -> Vec<Rule> {
            list.and_then(|l| l.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|r| {
                            let source = r.get("source")?.as_str()?.to_string();
                            let destination = r.get("destination")?.as_str()?.to_string();
                            if destination.starts_with("http://localhost")
                                || destination.starts_with("http://127.0.0.1")
                            {
                                return None;
                            }
                            // Internal rules Next adds for trailing slashes and basePath.
                            if r.get("internal").and_then(|x| x.as_bool()) == Some(true) {
                                return None;
                            }
                            let conds = |k: &str| -> Vec<Cond> {
                                r.get(k)
                                    .and_then(|x| x.as_array())
                                    .map(|a| {
                                        a.iter()
                                            .filter_map(|c| {
                                                Some(Cond {
                                                    kind: c.get("type")?.as_str()?.to_string(),
                                                    key: c
                                                        .get("key")
                                                        .and_then(|x| x.as_str())
                                                        .map(String::from),
                                                    value: c
                                                        .get("value")
                                                        .and_then(|x| x.as_str())
                                                        .map(String::from),
                                                })
                                            })
                                            .collect()
                                    })
                                    .unwrap_or_default()
                            };
                            Some(Rule {
                                pattern: PathPattern::new(&source).ok(),
                                source,
                                destination,
                                has: conds("has"),
                                missing: conds("missing"),
                                no_base_path: r.get("basePath").and_then(|x| x.as_bool())
                                    == Some(false),
                                no_locale: r.get("locale").and_then(|x| x.as_bool()) == Some(false),
                                conditional: false,
                            })
                        })
                        .collect()
                })
                .unwrap_or_default()
        };
        self.redirects = conv(v.get("redirects"));
        let rw = v.get("rewrites");
        self.rewrites = match rw {
            Some(serde_json::Value::Array(_)) => Rewrites {
                after_files: conv(rw),
                ..Rewrites::default()
            },
            Some(o) => Rewrites {
                before_files: conv(o.get("beforeFiles")),
                after_files: conv(o.get("afterFiles")),
                fallback: conv(o.get("fallback")),
            },
            None => Rewrites::default(),
        };
        self.rules_complete = true;
        self.rules_source = "routes-manifest";
        Ok(())
    }

    /// Every rule.
    pub fn all_rules(&self) -> impl Iterator<Item = &Rule> {
        self.redirects
            .iter()
            .chain(&self.rewrites.before_files)
            .chain(&self.rewrites.after_files)
            .chain(&self.rewrites.fallback)
    }
}

fn conds_of(v: Option<ConfigVal>) -> Option<Vec<Cond>> {
    let Some(v) = v else {
        return Some(Vec::new());
    };
    if v.is_null() {
        return Some(Vec::new());
    }
    let ConfigVal::Arr(xs) = v else {
        return None;
    };
    xs.iter()
        .map(|c| {
            Some(Cond {
                kind: c.field("type")?.as_str()?.to_string(),
                key: c.field("key").and_then(|k| k.as_str().map(String::from)),
                value: match c.field("value") {
                    None | Some(ConfigVal::Null) => None,
                    Some(v) => Some(v.as_str()?.to_string()),
                },
            })
        })
        .collect()
}

fn rule_of(v: &ConfigVal, conditional: bool) -> Option<Rule> {
    let ConfigVal::Obj(_) = v else {
        return None;
    };
    let source = v.field("source")?.as_str()?.to_string();
    let destination = v.field("destination")?.as_str()?.to_string();
    let has = conds_of(v.field("has"))?;
    let missing = conds_of(v.field("missing"))?;
    Some(Rule {
        pattern: PathPattern::new(&source).ok(),
        source,
        destination,
        has,
        missing,
        no_base_path: matches!(v.field("basePath"), Some(ConfigVal::Bool(false))),
        no_locale: matches!(v.field("locale"), Some(ConfigVal::Bool(false))),
        conditional,
    })
}
