use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    time::Duration,
};

use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};
use turborepo_cache::test_results::TestResult;
use turborepo_process::{ChildExit, Command, ProcessManager};
use turborepo_task_id::TaskId;

const HELPER: &str = include_str!("jest.cjs");

#[derive(Deserialize)]
struct Discovery {
    tests: Vec<PathBuf>,
    extensions: Vec<String>,
    #[serde(flatten)]
    context: BTreeMap<String, Value>,
}

pub struct TestPlan {
    units: BTreeMap<PathBuf, String>,
    inputs: BTreeMap<PathBuf, Option<Vec<u8>>>,
    directory: tempfile::TempDir,
    command: Command,
    executable: Option<PathBuf>,
}

#[derive(Deserialize)]
struct BatchOutcome {
    success: bool,
    files: Vec<FileOutcome>,
}

#[derive(Deserialize)]
struct FileOutcome {
    path: PathBuf,
    passed: u64,
    failed: u64,
    pending: u64,
    error: bool,
}

fn helper_command(command: &Command, helper: &Path, input: Value) -> Command {
    let mut command = command.clone();
    command.replace_program("node");
    command.args([helper.as_os_str().to_owned(), input.to_string().into()]);
    command
}

impl TestPlan {
    pub async fn discover(
        command: &Command,
        root: &AbsoluteSystemPath,
        task: &str,
        context: &str,
        manager: &ProcessManager,
        task_id: &TaskId<'static>,
    ) -> Option<Self> {
        let argv = command.javascript_test_command.as_ref()?;
        if !matches!(
            Path::new(argv.first()?).file_name()?.to_str()?,
            "jest" | "jest.js"
        ) || argv.iter().skip(1).any(|arg| arg != "--runInBand")
        {
            return None;
        }
        let standard: std::process::Command = command.clone().into();
        if standard.get_envs().any(|(name, value)| {
            matches!(
                name.to_str(),
                Some(
                    "NODE_V8_COVERAGE"
                        | "npm_config_node_options"
                        | "npm_config_script_shell"
                        | "BASH_ENV"
                        | "ENV"
                )
            ) || (name == "NODE_OPTIONS"
                && value
                    .is_some_and(|value| !value.is_empty() && value != "--experimental-vm-modules"))
        }) {
            return None;
        }
        let cwd = command.working_directory()?;
        let executable = if command.program() == std::ffi::OsStr::new(&argv[0]) {
            let path = standard
                .get_envs()
                .find(|(name, _)| *name == "PATH")
                .and_then(|(_, value)| value);
            Some(which::which_in(&argv[0], path, cwd.as_std_path()).ok()?)
        } else {
            None
        };
        let temporary = tempfile::tempdir().ok()?;
        let discovery_path = temporary.path().join("discovery.json");
        let helper = temporary.path().join("jest.cjs");
        std::fs::write(&helper, HELPER).ok()?;
        let probe = helper_command(
            command,
            &helper,
            json!({"mode": "discover", "task": task, "executable": executable,
                "root": root.as_str(), "result": discovery_path}),
        );
        probe_output(manager, probe, task_id, Duration::from_secs(10)).await?;
        let file = std::fs::File::open(discovery_path).ok()?;
        if file.metadata().ok()?.len() > 4 * 1024 * 1024 {
            return None;
        }
        let mut discovery: Discovery = serde_json::from_reader(file).ok()?;
        if discovery.tests.is_empty() {
            return None;
        }
        let root = root.as_std_path().canonicalize().ok()?;
        let mut common = BTreeMap::new();
        let cwd_path = cwd.as_std_path().canonicalize().ok()?;
        let mut directory = Some(cwd_path.as_path());
        while let Some(path) = directory {
            if !path.starts_with(&root) {
                break;
            }
            if [
                ".npmrc",
                ".yarnrc",
                ".yarnrc.yml",
                ".pnpmfile.cjs",
                ".pnp.cjs",
            ]
            .iter()
            .any(|name| path.join(name).exists())
            {
                return None;
            }
            let workspace_config = path.join("pnpm-workspace.yaml");
            read_input(&workspace_config, &mut common)?;
            if common
                .get(&workspace_config)?
                .as_ref()
                .is_some_and(|bytes| {
                    let config = String::from_utf8_lossy(bytes);
                    ["scriptShell", "shellEmulator", "nodeOptions", "pnpmfile"]
                        .iter()
                        .any(|key| config.contains(key))
                })
            {
                return None;
            }
            read_input(&path.join("package.json"), &mut common)?;
            if path == root {
                break;
            }
            directory = path.parent();
        }
        if let Some(Value::Object(config)) = discovery.context.get_mut("config") {
            config.remove("id");
            config.remove("cacheDirectory");
        }
        for value in discovery.context.values_mut() {
            portable_context(value, root.to_str()?);
        }
        let mut base = Sha256::new();
        base.update(HELPER);
        base.update(context);
        base.update(serde_json::to_vec(&discovery.context).ok()?);
        base.update(serde_json::to_vec(argv).ok()?);
        // Declared globals/env are in the shared context. These also affect Node
        // or Jest itself even though test code cannot access process/global.
        for (name, value) in standard.get_envs() {
            if matches!(
                name.to_str(),
                Some("NODE_OPTIONS" | "NODE_PATH" | "NODE_ENV" | "TZ" | "LANG" | "LC_ALL")
            ) {
                base.update(name.as_encoded_bytes());
                base.update([0]);
                if let Some(value) = value {
                    base.update(value.as_encoded_bytes());
                }
                base.update([0]);
            }
        }
        let mut units = BTreeMap::new();
        let mut inputs = common.clone();
        for path in discovery.tests {
            let mut files = common.clone();
            trace(
                &path,
                &root,
                &discovery.extensions,
                &mut files,
                &mut BTreeSet::new(),
            )?;
            let mut hash = base.clone();
            hash.update(
                path.strip_prefix(&root)
                    .ok()?
                    .as_os_str()
                    .as_encoded_bytes(),
            );
            for (file, bytes) in &files {
                hash.update(
                    file.strip_prefix(&root)
                        .ok()?
                        .as_os_str()
                        .as_encoded_bytes(),
                );
                hash.update([0]);
                match bytes {
                    Some(bytes) => {
                        hash.update([1]);
                        hash.update(bytes);
                    }
                    None => hash.update([0]),
                }
                hash.update([0]);
            }
            units.insert(path, format!("{:x}", hash.finalize()));
            inputs.extend(files);
        }
        Some(Self {
            units,
            inputs,
            directory: temporary,
            command: command.clone(),
            executable,
        })
    }

    pub fn keys(&self) -> Vec<String> {
        self.units.values().cloned().collect()
    }

    pub fn retain_misses(&mut self, hits: &BTreeMap<String, TestResult>) -> usize {
        let before = self.units.len();
        self.units.retain(|_, key| !hits.contains_key(key));
        before - self.units.len()
    }

    pub fn misses(&self) -> usize {
        self.units.len()
    }

    pub fn command(&self) -> Command {
        helper_command(
            &self.command,
            &self.directory.path().join("jest.cjs"),
            json!({
                "mode": "run", "tests": self.units.keys().collect::<Vec<_>>(),
                "executable": self.executable,
                "result": self.directory.path().join("result.json")
            }),
        )
    }

    pub fn inputs_unchanged(&self) -> bool {
        self.inputs
            .iter()
            .all(|(path, before)| std::fs::read(path).ok() == *before)
    }

    pub fn successful_results(&self) -> Option<BTreeMap<String, TestResult>> {
        if !self.inputs_unchanged() {
            return None;
        }
        let outcome: BatchOutcome =
            serde_json::from_slice(&std::fs::read(self.directory.path().join("result.json")).ok()?)
                .ok()?;
        if !outcome.success || outcome.files.len() != self.units.len() {
            return None;
        }
        let mut results = BTreeMap::new();
        for file in outcome.files {
            if file.error || file.failed != 0 || file.pending != 0 || file.passed == 0 {
                return None;
            }
            let key = self.units.get(&file.path)?;
            if results
                .insert(
                    key.clone(),
                    TestResult {
                        passed: file.passed,
                    },
                )
                .is_some()
            {
                return None;
            }
        }
        Some(results)
    }
}

async fn probe_output(
    manager: &ProcessManager,
    command: Command,
    task_id: &TaskId<'static>,
    timeout: Duration,
) -> Option<()> {
    let mut child = manager
        .spawn(command, Duration::from_millis(500), task_id.clone())?
        .ok()?;
    let mut output = ProbeOutput(0);
    match tokio::time::timeout(timeout, child.wait_with_piped_outputs(&mut output)).await {
        Ok(Ok(Some(ChildExit::Finished(Some(0))))) => Some(()),
        _ => {
            child.kill().await;
            None
        }
    }
}

struct ProbeOutput(usize);

impl std::io::Write for ProbeOutput {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0 += bytes.len();
        if self.0 > 64 * 1024 {
            return Err(std::io::Error::other(
                "test discovery output limit exceeded",
            ));
        }
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn portable_context(value: &mut Value, root: &str) {
    match value {
        Value::String(string) => *string = string.replace(root, "<root>"),
        Value::Array(values) => values
            .iter_mut()
            .for_each(|value| portable_context(value, root)),
        Value::Object(values) => values
            .values_mut()
            .for_each(|value| portable_context(value, root)),
        _ => {}
    }
}

fn read_input(path: &Path, inputs: &mut BTreeMap<PathBuf, Option<Vec<u8>>>) -> Option<()> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => Some(bytes),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => return None,
    };
    inputs.insert(path.to_owned(), bytes);
    Some(())
}

fn trace(
    path: &Path,
    root: &Path,
    extensions: &[String],
    inputs: &mut BTreeMap<PathBuf, Option<Vec<u8>>>,
    seen: &mut BTreeSet<PathBuf>,
) -> Option<()> {
    let path = path.canonicalize().ok()?;
    if !path.starts_with(root) || path.components().any(|p| p.as_os_str() == "node_modules") {
        return None;
    }
    if !seen.insert(path.clone()) {
        return Some(());
    }
    let mut ancestor = path.parent();
    while let Some(directory) = ancestor {
        if !directory.starts_with(root) {
            return None;
        }
        read_input(&directory.join("package.json"), inputs)?;
        if directory == root {
            break;
        }
        ancestor = directory.parent();
    }
    read_input(&path, inputs)?;
    let source = std::str::from_utf8(inputs.get(&path)?.as_deref()?).ok()?;
    if source.contains("@jest-environment") {
        return None;
    }
    if path.extension()?.to_str()? == "json" {
        serde_json::from_str::<Value>(source).ok()?;
        return Some(());
    }
    if !matches!(path.extension()?.to_str()?, "js" | "cjs" | "mjs") {
        return None;
    }
    let absolute = AbsoluteSystemPathBuf::try_from(path.clone()).ok()?;
    let (_, ast) =
        turbo_trace::parse_file(&absolute, source, turbo_trace::ImportTraceType::All, true).ok()?;
    let mut imports = BTreeSet::new();
    static_imports(&ast?, &mut imports)?;
    for import in imports {
        if !import.starts_with("./") && !import.starts_with("../") {
            return None;
        }
        let base = path.parent()?.join(import);
        let mut candidates = vec![base.clone()];
        candidates.extend(
            extensions
                .iter()
                .map(|extension| PathBuf::from(format!("{}.{}", base.display(), extension))),
        );
        // Package-directory resolution can depend on exports/main. Decline it.
        if base.is_dir() {
            return None;
        }
        let selected = candidates.iter().find(|p| p.is_file())?.clone();
        for candidate in &candidates {
            read_input(candidate, inputs)?;
        }
        trace(&selected, root, extensions, inputs, seen)?;
    }
    Some(())
}

fn static_imports(node: &Value, imports: &mut BTreeSet<String>) -> Option<()> {
    match node {
        Value::Object(object) => {
            match object.get("type").and_then(Value::as_str) {
                Some("ImportExpression" | "MetaProperty" | "WithStatement" | "ThisExpression") => {
                    return None;
                }
                Some("MemberExpression")
                    if object.get("object")?.get("name").and_then(Value::as_str)
                        == Some("module") =>
                {
                    return (object.get("computed") == Some(&Value::Bool(false))
                        && object.get("property")?.get("name").and_then(Value::as_str)
                            == Some("exports"))
                    .then_some(());
                }
                Some("ImportDeclaration" | "ExportNamedDeclaration" | "ExportAllDeclaration") => {
                    if let Some(source) = object.get("source").filter(|v| !v.is_null()) {
                        imports.insert(source.get("value")?.as_str()?.to_owned());
                    }
                }
                Some("CallExpression")
                    if object.get("callee")?.get("name").and_then(Value::as_str)
                        == Some("require") =>
                {
                    let args = object.get("arguments")?.as_array()?;
                    if args.len() != 1 {
                        return None;
                    }
                    imports.insert(args[0].get("value")?.as_str()?.to_owned());
                    return Some(());
                }
                Some("Literal")
                    if object
                        .get("value")
                        .and_then(Value::as_str)
                        .is_some_and(|value| {
                            matches!(
                                value,
                                "toMatchSnapshot"
                                    | "toMatchInlineSnapshot"
                                    | "toThrowErrorMatchingSnapshot"
                                    | "toThrowErrorMatchingInlineSnapshot"
                                    | "constructor"
                            )
                        }) =>
                {
                    return None;
                }
                Some("Identifier") => {
                    if matches!(
                        object.get("name")?.as_str()?,
                        "require"
                            | "module"
                            | "process"
                            | "global"
                            | "globalThis"
                            | "eval"
                            | "Function"
                            | "__dirname"
                            | "__filename"
                            | "beforeAll"
                            | "afterAll"
                            | "beforeEach"
                            | "afterEach"
                            | "jest"
                            | "toMatchSnapshot"
                            | "toMatchInlineSnapshot"
                            | "toThrowErrorMatchingSnapshot"
                            | "toThrowErrorMatchingInlineSnapshot"
                            | "constructor"
                            | "fetch"
                    ) {
                        return None;
                    }
                }
                _ => {}
            }
            for value in object.values() {
                static_imports(value, imports)?;
            }
        }
        Value::Array(values) => {
            for value in values {
                static_imports(value, imports)?;
            }
        }
        _ => {}
    }
    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn discovery_timeout_and_output_limit_reap_children() {
        for (script, timeout) in [
            ("setInterval(() => {}, 1000)", Duration::from_millis(200)),
            (
                "process.stdout.write('x'.repeat(128 * 1024)); setInterval(() => {}, 1000)",
                Duration::from_secs(5),
            ),
        ] {
            let manager = ProcessManager::new(false);
            let mut command = Command::new("node");
            command.args(["-e", script]);
            assert!(
                probe_output(&manager, command, &TaskId::new("pkg", "test"), timeout)
                    .await
                    .is_none()
            );
            assert!(manager.running_task_ids().is_empty());
        }
    }

    #[tokio::test]
    async fn discovery_is_cancelled_with_the_task_manager() {
        let manager = ProcessManager::new(false);
        let directory = tempfile::tempdir().unwrap();
        let ready = directory.path().join("ready");
        let mut command = Command::new("node");
        command.args([
            "-e",
            &format!(
                "require('fs').writeFileSync({}, 'ready'); setInterval(() => {{}}, 1000)",
                serde_json::to_string(&ready).unwrap()
            ),
        ]);
        let task_id = TaskId::new("pkg", "test");
        let probe = probe_output(&manager, command, &task_id, Duration::from_secs(30));
        let cancel = async {
            tokio::time::timeout(Duration::from_secs(5), async {
                while !ready.exists() {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap();
            manager.stop().await;
        };
        let (result, ()) = tokio::join!(probe, cancel);
        assert!(result.is_none());
        assert!(manager.running_task_ids().is_empty());
    }

    #[test]
    fn docblock_options_decline_and_absent_package_boundaries_are_tracked() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().canonicalize().unwrap();
        let nested = root.join("nested");
        std::fs::create_dir(&nested).unwrap();
        let entry = nested.join("entry.test.js");
        std::fs::write(
            &entry,
            "/** @jest-environment-options {\"url\":\"https://example.test\"} */\ntest('x', () => \
             expect(1).toBe(1));",
        )
        .unwrap();
        let mut inputs = BTreeMap::new();
        assert!(trace(&entry, &root, &[], &mut inputs, &mut BTreeSet::new()).is_none());
        std::fs::write(&entry, "test('x', () => expect(1).toBe(1));").unwrap();
        trace(&entry, &root, &[], &mut inputs, &mut BTreeSet::new()).unwrap();
        assert_eq!(inputs.get(&nested.join("package.json")), Some(&None));
        assert_eq!(inputs.get(&root.join("package.json")), Some(&None));
    }

    #[test]
    fn opaque_dependencies_and_snapshots_decline() {
        for source in [
            "require(name)",
            "module.require('./x')",
            "const m = module; m.require('./x')",
            "const r = require; r('./x')",
            "import('./x')",
            "jest.mock('./x')",
            "expect(1).toMatchSnapshot()",
            "expect(1)['toMatchSnapshot']()",
            "expect(1).toThrowErrorMatchingSnapshot()",
            "fetch('https://example.test')",
            "process.env.VALUE",
        ] {
            let path = AbsoluteSystemPathBuf::try_from(PathBuf::from("/test.js")).unwrap();
            let (_, ast) =
                turbo_trace::parse_file(&path, source, turbo_trace::ImportTraceType::All, true)
                    .unwrap();
            assert!(
                static_imports(&ast.unwrap(), &mut BTreeSet::new()).is_none(),
                "{source}"
            );
        }
    }

    #[test]
    fn nested_static_require_is_traced() {
        let path = AbsoluteSystemPathBuf::try_from(PathBuf::from("/test.js")).unwrap();
        let (_, ast) = turbo_trace::parse_file(
            &path,
            "test('x', () => expect(require('./x')).toBe(1))",
            turbo_trace::ImportTraceType::All,
            true,
        )
        .unwrap();
        let mut imports = BTreeSet::new();
        static_imports(&ast.unwrap(), &mut imports).unwrap();
        assert_eq!(imports, BTreeSet::from(["./x".into()]));
    }
}
