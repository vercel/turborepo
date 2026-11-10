//! Vite plugins in a vitest config: which of them can change what a test loads.
//!
//! A vite plugin may do anything: alias imports (`resolveId`), serve virtual
//! modules (`load`), rewrite sources (`transform`). The tracer does not run
//! plugins, so a config with plugins used to be uncertain as a whole
//! (`conditional_resolution`), which selects every test of the project on every
//! change. Most projects list one or two well-known plugins whose effect is
//! known:
//!
//! | Plugin | What it does | Why the graph stays right |
//! |---|---|---|
//! | `@vitejs/plugin-react`, `@vitejs/plugin-react-swc`, `@vitejs/plugin-react-oxc` | JSX transform and fast refresh | Compiles each file on its own; resolves nothing but its own refresh runtime (dev server only) |
//! | `vite-tsconfig-paths` | Resolves imports through tsconfig `paths` | The tracer's resolver reads the same tsconfig `paths` |
//! | `@vgpu/wgsl/loader-vite` | Compiles `.wgsl` shader modules, inlining the shaders they import | Its only hook is `transform`, which returns `null` unless the module id ends with `.wgsl`: a *loader* ([`Loader`]). The shaders a module imports are written as ES import lines, which the `wgsl` language plugin reads (`turborepo_trace_lang_js::wgsl`): they are edges of the shader file |
//!
//! A loader plugin is known to touch only modules with certain suffixes. It
//! makes nothing uncertain by itself. What it reads for a module is traced when
//! a language plugin reads the module's file (a shader's import lines; a line
//! it cannot read or resolve is a record of the shader file). Without one, the
//! graph attaches the uncertainty to the files that import such a module, so a
//! test is uncertain only if it loads one.
//!
//! Only a call without options counts (`react()`), or with options that cannot
//! redirect an import (no `babel`, no `jsxImportSource`, no `projects`/`root`):
//! a babel plugin can rewrite imports, and `vite-tsconfig-paths` can be pointed
//! at other tsconfigs. The plugin package itself is an input of the config (the
//! config imports it), so an upgrade still selects the project's tests.
//! Everything else stays uncertain, named in the report: a plugin from another
//! package, an inline plugin object, a value that is not statically known.
//!
//! What such a plugin can change that the graph does not see is which module a
//! test loads: where an import of the project's code resolves (`resolveId`),
//! what is served for it (`load`), what its source becomes (`transform`), and
//! what the plugin's `config` hook adds to the runner's configuration (setup
//! files, aliases). So the record is that of a *module load with an unknown
//! target* of every test of the project
//! ([`crate::FrameworkUncertainty::is_load`]), and the bound for such loads
//! applies to it (`turborepo_trace_graph::loads`: the project's package and the
//! packages it declares, where packages resolve only what they declare); it is
//! open elsewhere and under `--strict`.

use serde::Serialize;
use turborepo_trace_lang_js::extract::config::ConfigVal;

/// Plugins whose effect on resolution and on the module graph is modelled or
/// nil, with the module suffixes a loader plugin handles (empty: the plugin
/// loads nothing the graph does not see).
const KNOWN: &[(&str, &[&str])] = &[
    ("@vitejs/plugin-react", &[]),
    ("@vitejs/plugin-react-swc", &[]),
    ("@vitejs/plugin-react-oxc", &[]),
    ("vite-tsconfig-paths", &[]),
    ("@vgpu/wgsl/loader-vite", &[".wgsl"]),
];

/// A plugin that only handles modules whose id ends with one of `suffixes`, and
/// may read files the graph does not see while doing so (a shader's imports).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Loader {
    /// The plugin's module (`@vgpu/wgsl/loader-vite`).
    pub plugin: String,
    /// Module id suffixes (`.wgsl`).
    pub suffixes: Vec<String>,
}

/// Option names that let a known plugin change what an import resolves to.
const REDIRECTING_OPTIONS: &[&str] = &["babel", "jsxImportSource", "projects", "root", "plugins"];

/// The suffix the config evaluator gives a call into a package it does not
/// evaluate (`react() (call into '@vitejs/plugin-react')`).
const CALL_INTO: &str = " (call into '";

/// The elements of a config's `plugins` value that are not modelled, described
/// for the uncertainty report. Empty when every plugin is known not to change
/// what tests load.
pub fn unmodelled(plugins: &ConfigVal) -> Vec<String> {
    let mut out = Vec::new();
    collect(plugins, &mut out, &mut Vec::new());
    out.sort();
    out.dedup();
    out
}

/// The [`Loader`] plugins among a config's `plugins`.
pub fn loaders(plugins: &ConfigVal) -> Vec<Loader> {
    let mut out = Vec::new();
    collect(plugins, &mut Vec::new(), &mut out);
    out.dedup();
    out
}

fn collect(v: &ConfigVal, out: &mut Vec<String>, loaders: &mut Vec<Loader>) {
    match v {
        // `cond && plugin()` when the condition is false, holes.
        ConfigVal::Null | ConfigVal::Bool(false) => {}
        // Vite flattens nested arrays (presets); alternatives may each be present.
        ConfigVal::Arr(xs) | ConfigVal::Alts(xs) => {
            xs.iter().for_each(|x| collect(x, out, loaders))
        }
        ConfigVal::Unknown(reason) => match known_call(reason) {
            Some(Some((plugin, suffixes))) => {
                if !suffixes.is_empty() {
                    loaders.push(Loader {
                        plugin: plugin.to_string(),
                        suffixes: suffixes.iter().map(|s| s.to_string()).collect(),
                    });
                }
            }
            Some(None) => out.push(format!("plugin {reason}")),
            None => out.push(format!("not statically known: {reason}")),
        },
        ConfigVal::Obj(o) => {
            let name = o.get("name").and_then(|n| n.as_str()).unwrap_or("?");
            out.push(format!("inline plugin '{name}'"));
        }
        ConfigVal::External { module, .. } => out.push(format!("value from package '{module}'")),
        ConfigVal::Function(f) => out.push(format!("function (line {})", f.line)),
        other => {
            let mut rs = Vec::new();
            other.unknown_reasons(&mut rs);
            out.push(match rs.first() {
                Some(r) => format!("not statically known: {r}"),
                None => "not a plugin".to_string(),
            });
        }
    }
}

/// For the evaluator's description of a call into a package: the [`KNOWN`]
/// plugin it calls (module and loader suffixes) with options that cannot
/// redirect imports, or `Some(None)` for any other call. `None`: not a call
/// into a package.
fn known_call(reason: &str) -> Option<Option<(&'static str, &'static [&'static str])>> {
    let (call, rest) = reason.rsplit_once(CALL_INTO)?;
    let module = rest.strip_suffix("')")?;
    let Some(&known) = KNOWN.iter().find(|(m, _)| *m == module) else {
        return Some(None);
    };
    // The call as written: `react()`, `react({ include: /\.tsx$/ })`. A truncated
    // text (`…`) may hide options.
    let args = call
        .split_once('(')
        .and_then(|(_, a)| a.rsplit_once(')'))
        .map(|(a, _)| a.trim());
    let harmless = match args {
        Some("") => true,
        Some(a) => !call.contains('…') && !REDIRECTING_OPTIONS.iter().any(|o| a.contains(o)),
        None => false,
    };
    Some(harmless.then_some(known))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(text: &str, module: &str) -> ConfigVal {
        ConfigVal::Unknown(format!("{text}{CALL_INTO}{module}')").into())
    }

    #[test]
    fn known_plugins_are_modelled() {
        let react = call("react()", "@vitejs/plugin-react");
        assert!(unmodelled(&ConfigVal::Arr(vec![react.clone()])).is_empty());
        assert!(unmodelled(&ConfigVal::Arr(vec![])).is_empty());
        // Conditional plugins, nested arrays, harmless options.
        let v = ConfigVal::Arr(vec![
            ConfigVal::Bool(false),
            ConfigVal::Null,
            ConfigVal::Arr(vec![call("tsconfigPaths()", "vite-tsconfig-paths")]),
            call("react({ include: /\\.tsx$/ })", "@vitejs/plugin-react-swc"),
        ]);
        assert!(unmodelled(&v).is_empty(), "{:?}", unmodelled(&v));
    }

    /// A loader plugin is modelled: it makes no test uncertain by itself and is
    /// reported with the module suffixes it compiles.
    #[test]
    fn loader_plugins_are_scoped_to_their_modules() {
        let v = ConfigVal::Arr(vec![
            call("react()", "@vitejs/plugin-react"),
            call("wgslVitePlugin({ minify: true })", "@vgpu/wgsl/loader-vite"),
        ]);
        assert!(unmodelled(&v).is_empty(), "{:?}", unmodelled(&v));
        assert_eq!(
            loaders(&v),
            [Loader {
                plugin: "@vgpu/wgsl/loader-vite".into(),
                suffixes: vec![".wgsl".into()],
            }]
        );
        assert!(
            loaders(&ConfigVal::Arr(vec![call(
                "react()",
                "@vitejs/plugin-react"
            )]))
            .is_empty()
        );
    }

    #[test]
    fn other_plugins_stay_uncertain() {
        let react = call("react()", "@vitejs/plugin-react");
        // A plugin of another package, next to a known one.
        let loader = call("wgsl({ minify: true })", "@acme/wgsl/loader-vite");
        let got = unmodelled(&ConfigVal::Arr(vec![react, loader]));
        assert_eq!(
            got,
            ["plugin wgsl({ minify: true }) (call into '@acme/wgsl/loader-vite')"]
        );
        // Options that can redirect imports.
        for text in [
            "react({ babel: { plugins: ['./rewrite-imports'] } })",
            "react({ jsxImportSource: '@emotion/react' })",
            "react({ a: 1, …",
        ] {
            let v = ConfigVal::Arr(vec![call(text, "@vitejs/plugin-react")]);
            assert_eq!(unmodelled(&v).len(), 1, "{text}");
        }
        let v = ConfigVal::Arr(vec![call(
            "tsconfigPaths({ projects: ['../other'] })",
            "vite-tsconfig-paths",
        )]);
        assert_eq!(unmodelled(&v).len(), 1);
        // Not a call into a package: an unknown value, a spread of one.
        let v = ConfigVal::Arr(vec![ConfigVal::Unknown("makePlugins()".into())]);
        assert_eq!(unmodelled(&v), ["not statically known: makePlugins()"]);
        let v = ConfigVal::Unknown("plugins".into());
        assert_eq!(unmodelled(&v), ["not statically known: plugins"]);
    }
}
