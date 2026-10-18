//! Import tracing is an input-discovery aid, not a sandbox. Files read by user
//! code, preloads, environment and custom loaders still need declared inputs.

use std::{
    collections::{BTreeSet, HashMap},
    fs,
    path::{Path, PathBuf},
};

use serde_json::Value;
use turbo_trace::parse_runtime_imports;
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};
use unrs_resolver::{
    ResolveContext, ResolveError, ResolveOptions, Resolver, TsconfigOptions, TsconfigReferences,
};

use super::{invalid, trace_explicit_inputs};
use crate::{Error, raw::RawFileTasks};

fn diagnostic_message(error: Error) -> String {
    match error {
        Error::FileTask(message) => message,
        other => other.to_string(),
    }
}

#[derive(Debug, Clone, Default)]
struct Module {
    name: String,
    inputs: BTreeSet<String>,
    dependencies: Vec<AbsoluteSystemPathBuf>,
}

#[derive(Default)]
pub(super) struct TracedInputs {
    pub paths: BTreeSet<String>,
    pub modules: BTreeSet<AbsoluteSystemPathBuf>,
}

#[derive(Debug)]
pub(super) struct FileTracer {
    root: AbsoluteSystemPathBuf,
    physical_root: PathBuf,
    roots: Vec<AbsoluteSystemPathBuf>,
    bun: bool,
    resolver: Resolver,
    resolvers: HashMap<PathBuf, Resolver>,
    modules: HashMap<AbsoluteSystemPathBuf, Result<Module, String>>,
    configs: HashMap<PathBuf, Result<BTreeSet<String>, String>>,
}

impl FileTracer {
    pub(super) fn new(
        root: &AbsoluteSystemPath,
        directory: &AbsoluteSystemPath,
        options: &RawFileTasks,
    ) -> Result<Self, Error> {
        let resolution = options
            .resolution
            .as_ref()
            .map(|v| v.as_str())
            .unwrap_or("explicit");
        if !options.trace.unwrap_or(false)
            && (options.resolution.is_some()
                || options.trace_roots.is_some()
                || options.fallback_inputs.is_some())
        {
            return Err(invalid(
                "resolution, traceRoots and fallbackInputs require trace:true",
            ));
        }
        if let Some(fallback) = &options.fallback_inputs {
            if fallback.is_empty() || fallback.iter().any(|v| v.starts_with('!')) {
                return Err(invalid(
                    "fallbackInputs must contain positive complete input globs",
                ));
            }
            for pattern in fallback {
                pattern
                    .as_str()
                    .parse::<globwalk::ValidatedGlob>()
                    .map_err(|e| invalid(format!("invalid fallback input: {e}")))?;
            }
        }
        let roots = options
            .trace_roots
            .as_deref()
            .unwrap_or_default()
            .iter()
            .map(|path| {
                let path = if let Some(relative) = path.strip_prefix("$TURBO_ROOT$/") {
                    AbsoluteSystemPathBuf::from_unknown(root, relative)
                } else {
                    AbsoluteSystemPathBuf::from_unknown(directory, path.as_str())
                };
                if !path.as_std_path().is_file()
                    || !path.as_std_path().starts_with(root.as_std_path())
                {
                    return Err(invalid(format!(
                        "traceRoots must name existing repository files: {path}"
                    )));
                }
                Ok(path)
            })
            .collect::<Result<_, _>>()?;
        // Bun's local ESM extension order. Require is rejected below rather than
        // pretending that its conditions and extension order are the same.
        let resolver = Resolver::new(ResolveOptions {
            builtin_modules: true,
            extensions: [
                ".tsx", ".jsx", ".mts", ".ts", ".mjs", ".js", ".cts", ".cjs", ".json",
            ]
            .map(String::from)
            .to_vec(),
            extension_alias: vec![
                (
                    ".js".into(),
                    vec![".js".into(), ".ts".into(), ".tsx".into()],
                ),
                (".jsx".into(), vec![".jsx".into(), ".tsx".into()]),
                (".mjs".into(), vec![".mjs".into(), ".mts".into()]),
            ],
            condition_names: ["bun", "node-addons", "node", "import", "default"]
                .map(String::from)
                .to_vec(),
            main_fields: vec!["main".into(), "module".into()],
            ..ResolveOptions::default()
        });
        Ok(Self {
            root: root.to_owned(),
            physical_root: fs::canonicalize(root.as_std_path())?,
            roots,
            bun: resolution == "bun",
            resolver,
            resolvers: HashMap::new(),
            modules: HashMap::new(),
            configs: HashMap::new(),
        })
    }

    pub(super) fn inputs(&mut self, entry: &AbsoluteSystemPath) -> Result<TracedInputs, Error> {
        let mut pending = self.roots.clone();
        pending.push(entry.to_owned());
        let mut seen = BTreeSet::new();
        let mut inputs = BTreeSet::new();
        if !self.bun {
            for file in pending {
                inputs.extend(trace_explicit_inputs(&self.root, &file)?);
            }
            return Ok(TracedInputs {
                paths: inputs,
                modules: BTreeSet::new(),
            });
        }
        // Cross-workspace imports may use external dependencies that are absent
        // from the entry package's lockfile closure. Hash the full lockfiles.
        inputs.extend(
            [
                "bun.lock",
                "bun.lockb",
                "package-lock.json",
                "pnpm-lock.yaml",
                "yarn.lock",
            ]
            .map(String::from),
        );
        while let Some(file) = pending.pop() {
            if !seen.insert(file.clone()) {
                continue;
            }
            if seen.len() > 1024 {
                return Err(invalid(
                    "module trace exceeds 1024 files; declare complete fallbackInputs",
                ));
            }
            if !self.modules.contains_key(&file) {
                let module = self.read_module(&file).map_err(diagnostic_message);
                self.modules.insert(file.clone(), module);
            }
            let module = self
                .modules
                .get(&file)
                .ok_or_else(|| invalid("missing traced module"))?
                .as_ref()
                .map_err(invalid)?;
            pending.extend(module.dependencies.iter().cloned());
        }
        Ok(TracedInputs {
            paths: inputs,
            modules: seen,
        })
    }

    pub(super) fn module_inputs(
        &self,
        file: &AbsoluteSystemPath,
    ) -> Result<&BTreeSet<String>, Error> {
        self.modules
            .get(file)
            .ok_or_else(|| invalid("missing traced module"))?
            .as_ref()
            .map(|module| &module.inputs)
            .map_err(invalid)
    }

    pub(super) fn module_name(&self, file: &AbsoluteSystemPath) -> Result<&str, Error> {
        self.modules
            .get(file)
            .ok_or_else(|| invalid("missing traced module"))?
            .as_ref()
            .map(|module| module.name.as_str())
            .map_err(invalid)
    }

    fn add_path(&self, module: &mut Module, path: &Path) -> Result<(), Error> {
        // Installed dependencies are represented by the lockfile. Workspace
        // symlinks resolve to their repository sources and are traced normally.
        if path.components().any(|c| c.as_os_str() == "node_modules") {
            return Ok(());
        }
        let relative = path.strip_prefix(&self.physical_root).map_err(|_| {
            invalid(format!(
                "traced input escapes repository: {}",
                path.display()
            ))
        })?;
        module.inputs.insert(
            relative
                .to_str()
                .ok_or_else(|| invalid("non-UTF-8 traced path"))?
                .replace('\\', "/"),
        );
        Ok(())
    }

    fn resolver_for(
        &mut self,
        file: &AbsoluteSystemPath,
        module: &mut Module,
    ) -> Result<PathBuf, Error> {
        let parent = file
            .as_std_path()
            .parent()
            .ok_or_else(|| invalid("module has no parent"))?;
        let mut nearest_config = None;
        for ancestor in parent
            .ancestors()
            .take_while(|p| p.starts_with(&self.physical_root))
        {
            for name in [
                "package.json",
                "tsconfig.json",
                "jsconfig.json",
                "bunfig.toml",
            ] {
                let path = ancestor.join(name);
                self.add_path(module, &path)?;
                if nearest_config.is_none()
                    && matches!(name, "tsconfig.json" | "jsconfig.json")
                    && path.is_file()
                {
                    nearest_config = Some(path);
                }
            }
        }
        if let Some(config) = &nearest_config {
            if !self.configs.contains_key(config) {
                let mut configuration = Module::default();
                let result = self
                    .config_inputs(config, &mut configuration, &mut BTreeSet::new())
                    .map(|()| configuration.inputs)
                    .map_err(diagnostic_message);
                self.configs.insert(config.clone(), result);
            }
            module.inputs.extend(
                self.configs
                    .get(config)
                    .ok_or_else(|| invalid("missing traced config"))?
                    .as_ref()
                    .map_err(invalid)?
                    .iter()
                    .cloned(),
            );
        }
        let key = nearest_config
            .clone()
            .unwrap_or_else(|| self.physical_root.clone());
        if !self.resolvers.contains_key(&key) {
            let mut options = self.resolver.options().clone();
            options.tsconfig = nearest_config.map(|config_file| TsconfigOptions {
                config_file,
                references: TsconfigReferences::Disabled,
            });
            self.resolvers
                .insert(key.clone(), self.resolver.clone_with_options(options));
        }
        Ok(key)
    }

    fn config_inputs(
        &self,
        path: &Path,
        module: &mut Module,
        seen: &mut BTreeSet<PathBuf>,
    ) -> Result<(), Error> {
        if !seen.insert(path.to_owned()) {
            return Ok(());
        }
        self.add_path(module, path)?;
        let mut contents = fs::read_to_string(path)?;
        let config = unrs_resolver::TsConfig::parse(true, path, &mut contents)
            .map_err(|e| invalid(format!("{}: {e}", path.display())))?;
        let json: Value = serde_json::from_str(&contents).map_err(|e| invalid(e.to_string()))?;
        if json
            .get("compilerOptions")
            .and_then(|v| v.get("jsxImportSource"))
            .is_some()
        {
            return Err(invalid(
                "custom JSX import sources require explicit inputs or fallbackInputs",
            ));
        }
        let mut options = self.resolver.options().clone();
        options.extensions = vec![".json".into()];
        options.main_fields = vec!["tsconfig".into(), "main".into()];
        options.extension_alias.clear();
        let resolver = self.resolver.clone_with_options(options);
        for extends in config.extends() {
            let mut context = ResolveContext::default();
            let resolved = resolver
                .resolve_with_context(config.directory(), extends, &mut context)
                .map_err(|e| invalid(format!("cannot trace tsconfig extension {extends}: {e}")))?;
            for dependency in context
                .file_dependencies
                .iter()
                .chain(&context.missing_dependencies)
            {
                if !dependency.is_dir() && dependency.starts_with(&self.physical_root) {
                    self.add_path(module, dependency)?;
                }
            }
            self.config_inputs(resolved.path(), module, seen)?;
        }
        Ok(())
    }

    fn read_module(&mut self, file: &AbsoluteSystemPath) -> Result<Module, Error> {
        let physical = fs::canonicalize(file.as_std_path())?;
        let logical = self.root.anchor(file).map_err(|e| invalid(e.to_string()))?;
        if self.physical_root.join(logical.as_str()) != physical {
            return Err(invalid(format!(
                "symlinked traced inputs are not supported: {file}"
            )));
        }
        let mut module = Module {
            // Compute the stable identity once per source, not once per importer.
            name: super::encode_name(
                &physical
                    .strip_prefix(&self.physical_root)
                    .map_err(|e| invalid(e.to_string()))?
                    .to_string_lossy()
                    .replace('\\', "/"),
            ),
            ..Default::default()
        };
        self.add_path(&mut module, &physical)?;
        let physical_file = AbsoluteSystemPathBuf::new(physical.to_string_lossy())
            .map_err(|e| invalid(e.to_string()))?;
        let key = self.resolver_for(&physical_file, &mut module)?;
        let extension = file.extension().unwrap_or_default();
        if matches!(
            extension,
            "json"
                | "jsonc"
                | "toml"
                | "txt"
                | "svg"
                | "png"
                | "jpg"
                | "jpeg"
                | "gif"
                | "webp"
                | "avif"
                | "ico"
        ) {
            return Ok(module);
        }
        if !matches!(extension, "ts" | "tsx" | "mts" | "js" | "jsx" | "mjs") {
            return Err(invalid(format!("unsupported traced source: {file}")));
        }
        let text = fs::read_to_string(&physical)?;
        let mut imports =
            parse_runtime_imports(file, &text).map_err(|e| invalid(format!("{file}: {e}")))?;
        if imports.uses_jsx {
            // JSX emits imports absent from the written source. Track both
            // default runtimes conservatively; aliases/workspace exports can
            // resolve them to mutable repository files. Custom sources fail
            // closed through the configuration, pragma and command guards.
            imports
                .specifiers
                .extend(["react/jsx-runtime".into(), "react/jsx-dev-runtime".into()]);
        }
        let resolver = self
            .resolvers
            .get(&key)
            .ok_or_else(|| invalid("missing resolver"))?;
        for specifier in imports.specifiers {
            if specifier.starts_with("bun:") || specifier == "bun" {
                continue;
            }
            let mut context = ResolveContext::default();
            let resolved = resolver.resolve_with_context(
                physical
                    .parent()
                    .ok_or_else(|| invalid("module has no parent"))?,
                &specifier,
                &mut context,
            );
            for path in context
                .file_dependencies
                .iter()
                .chain(&context.missing_dependencies)
            {
                if path.starts_with(&self.physical_root) && !path.is_dir() {
                    self.add_path(&mut module, path)?;
                }
            }
            let resolved = match resolved {
                Ok(resolved) => resolved,
                Err(ResolveError::Builtin { .. }) => continue,
                Err(error) => {
                    return Err(invalid(format!(
                        "{file}: cannot resolve {specifier}: {error}"
                    )));
                }
            };
            let path = resolved.path();
            if path.components().any(|c| c.as_os_str() == "node_modules") {
                continue;
            }
            self.add_path(&mut module, path)?;
            module.dependencies.push(
                AbsoluteSystemPathBuf::new(path.to_string_lossy())
                    .map_err(|e| invalid(e.to_string()))?,
            );
        }
        Ok(module)
    }
}
