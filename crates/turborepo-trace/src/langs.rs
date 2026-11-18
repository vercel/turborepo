//! Compiled-in language registration.
//!
//! Languages are an explicit list. No `inventory` or linker tricks: to add a
//! language, add its crate as a dependency and one line here.
//!
//! Language-specific flags live in [`LangArgs`] (Go `--tags`, Rust
//! `--features`, `--all-features`, `--no-default-features`, Python
//! `--python-path`, `--venv`). Concepts several languages share are core
//! [`turborepo_trace_core::TraceOptions`] (`--target`, `--all-targets`).

use std::{path::PathBuf, sync::Arc};

use turborepo_trace_core::Language;

/// Language-specific options, shared by `trace`, `why`, `extract` and `graph
/// build`.
///
/// `graph build` records them (paths made absolute) in `<graph
/// dir>/lang-args.json`, so the refresh before `graph query`/`export` uses the
/// same languages as the build.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct LangArgs {
    /// Go build tags, comma separated (`go build -tags`).
    pub tags: Vec<String>,
    /// Cargo features of the workspace members, comma separated (`name` or
    /// `pkg/name`).
    pub features: Vec<String>,
    /// Enable every feature of the workspace members.
    pub all_features: bool,
    /// Do not enable the members' `default` features.
    pub no_default_features: bool,
    /// Extra Python module search roots (like `PYTHONPATH`). Repeatable.
    pub python_path: Vec<PathBuf>,
    /// The Python virtual environment [default: $VIRTUAL_ENV, else .venv/ or
    /// venv/ in the base].
    pub venv: Option<PathBuf>,
}

/// Constructs one language from the options.
pub type LangCtor = fn(&LangArgs) -> Arc<dyn Language>;

/// Every compiled-in language, in priority order (earlier wins extension
/// conflicts). Python and shell scripts are not compiled in yet.
pub const LANGUAGES: &[LangCtor] = &[
    js,
    wgsl,
    #[cfg(feature = "go")]
    go,
    #[cfg(feature = "rust")]
    rust,
];

fn js(_: &LangArgs) -> Arc<dyn Language> {
    Arc::new(turborepo_trace_lang_js::JsLanguage)
}

/// Shader modules that JS/TS modules import (`.wgsl` with ES import lines).
/// They resolve with the JS resolver.
fn wgsl(_: &LangArgs) -> Arc<dyn Language> {
    Arc::new(turborepo_trace_lang_js::wgsl::WgslLanguage)
}

#[cfg(feature = "go")]
fn go(a: &LangArgs) -> Arc<dyn Language> {
    use turborepo_trace_lang_go::{GoLanguage, GoOptions};
    Arc::new(GoLanguage::new(GoOptions {
        tags: a.tags.clone(),
        ..GoOptions::default()
    }))
}

#[cfg(feature = "rust")]
fn rust(a: &LangArgs) -> Arc<dyn Language> {
    use turborepo_trace_lang_rust::{FeatureSelection, RustLanguage, RustOptions};
    Arc::new(RustLanguage::with_options(RustOptions {
        cargo_home: None,
        features: FeatureSelection {
            features: a.features.clone(),
            all_features: a.all_features,
            no_default_features: a.no_default_features,
        },
    }))
}

impl LangArgs {
    /// The file `graph build` records the arguments in, below the graph
    /// directory.
    pub const FILE: &str = "lang-args.json";

    /// These arguments with paths made absolute (against the current
    /// directory).
    pub fn absolutized(&self) -> LangArgs {
        LangArgs {
            python_path: self.python_path.iter().map(absolute).collect(),
            venv: self.venv.as_ref().map(absolute),
            ..self.clone()
        }
    }

    /// Records the arguments (absolutized) below the graph directory `dir`.
    pub fn save(&self, dir: &std::path::Path) -> std::io::Result<()> {
        let json = serde_json::to_vec_pretty(&self.absolutized()).map_err(std::io::Error::other)?;
        // Left alone when it already says the same: a refresh that changes nothing
        // writes nothing below the graph directory.
        turborepo_trace_store::write_if_changed(&dir.join(Self::FILE), &json).map(|_| ())
    }

    /// The arguments recorded below `dir`, else the defaults.
    pub fn load(dir: &std::path::Path) -> LangArgs {
        std::fs::read(dir.join(Self::FILE))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }
}

fn absolute(p: &PathBuf) -> PathBuf {
    match std::env::current_dir() {
        Ok(cwd) => turborepo_trace_core::fs::normalize(&cwd.join(p)),
        Err(_) => p.clone(),
    }
}

/// Instantiates all compiled-in languages with `args`.
pub fn all(args: &LangArgs) -> Vec<Arc<dyn Language>> {
    LANGUAGES.iter().map(|ctor| ctor(args)).collect()
}
