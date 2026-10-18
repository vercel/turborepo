//! Prototype file-task expansion. The resulting tasks use the ordinary engine,
//! task hasher, executor, and local/remote cache without a child-tool protocol.

use std::{collections::BTreeSet, fs};

mod plan;
pub(crate) use plan::FileTasks;

mod trace;

use globwalk::{ValidatedGlob, WalkType};
use turbo_trace::parse_explicit_imports;
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};
use turborepo_errors::Spanned;
use turborepo_task_id::TaskName;
use turborepo_unescape::UnescapedString;

use crate::{
    Error, RawTaskDefinition, TurboJson,
    raw::{RawCommand, RawTaskInput},
};

const FILE: &str = "$TURBO_FILE$";

fn invalid(message: impl Into<String>) -> Error {
    Error::FileTask(message.into())
}

pub(crate) fn expand(
    config: &mut TurboJson,
    repo_root: &AbsoluteSystemPath,
    config_path: &AbsoluteSystemPath,
) -> Result<(), Error> {
    let templates: Vec<_> = config
        .tasks
        .iter()
        .filter(|(_, task)| task.experimental_files.is_some())
        .map(|(name, task)| (name.clone(), task.as_inner().clone()))
        .collect();
    if templates.is_empty() {
        return Ok(());
    }
    let directory = config_path
        .parent()
        .ok_or_else(|| invalid("config has no parent"))?;
    let physical_directory = fs::canonicalize(directory.as_std_path())?;
    let physical_root = fs::canonicalize(repo_root.as_std_path())?;
    if !physical_directory.starts_with(&physical_root)
        || (config.extends.is_empty() && physical_directory != physical_root)
    {
        return Err(invalid(
            "file task configuration must be in its executing package directory; root templates \
             must be in the repository root",
        ));
    }
    for (name, template) in templates {
        // Root templates must explicitly address the root. Otherwise generated
        // paths would be inherited by packages with a different working directory.
        if config.extends.is_empty() && name.package() != Some("//") {
            return Err(invalid(format!(
                "{name}: root file tasks must use //#task syntax"
            )));
        }
        let options = template
            .experimental_files
            .as_ref()
            .ok_or_else(|| invalid("missing file options"))?;
        let tracer = trace::FileTracer::new(repo_root, directory, options)?;
        if options.include.is_empty() {
            return Err(invalid(format!("{name}: include must not be empty")));
        }
        let Some(command) = template.command.as_ref() else {
            return Err(invalid(format!("{name}: command argv is required")));
        };
        let RawCommand::Argv(argv) = command.as_inner() else {
            return Err(invalid(format!("{name}: command must be an argv array")));
        };
        if options
            .resolution
            .as_ref()
            .is_some_and(|v| v.as_str() == "bun")
            && argv.iter().any(|arg| {
                [
                    "--conditions",
                    "--extension-order",
                    "--tsconfig-override",
                    "--define",
                    "--loader",
                    "--target",
                    "--no-addons",
                    "--main-fields",
                    "--preserve-symlinks",
                    "--preserve-symlinks-main",
                    "--jsx-import-source",
                ]
                .iter()
                .any(|flag| arg.as_str() == *flag || arg.starts_with(&format!("{flag}=")))
                    || (!arg.starts_with("--") && (arg.starts_with("-d") || arg.starts_with("-l")))
            })
        {
            return Err(invalid(format!(
                "{name}: custom module resolution flags require trace:false and explicit inputs"
            )));
        }
        if argv.first().is_none_or(|arg| arg.as_str() == FILE)
            || !argv.iter().skip(1).any(|arg| arg.as_str() == FILE)
        {
            return Err(invalid(format!(
                "{name}: command needs a {FILE} argument after the executable"
            )));
        }
        let Some(shared_inputs) = template.inputs.as_ref() else {
            return Err(invalid(format!(
                "{name}: declare shared inputs explicitly (use [] if none)"
            )));
        };
        if shared_inputs
            .iter()
            .any(|input| !matches!(input.as_inner(), RawTaskInput::String(_)))
        {
            return Err(invalid(format!(
                "{name}: structured inputs are not supported by this prototype"
            )));
        }
        if shared_inputs.iter().any(|input| {
            matches!(input.as_inner(), RawTaskInput::String(value) if value.starts_with('!') || value.as_str() == "$TURBO_EXTENDS$")
        }) {
            return Err(invalid(format!("{name}: input exclusions could hide mandatory file dependencies")));
        }
        if template
            .outputs
            .as_ref()
            .is_some_and(|items| !items.is_empty())
            || template.persistent.as_ref().is_some_and(|v| **v)
            || template.interactive.as_ref().is_some_and(|v| **v)
            || template.with.is_some()
        {
            return Err(invalid(format!(
                "{name}: this prototype supports finite, noninteractive tasks with logs only"
            )));
        }
        let includes: Vec<ValidatedGlob> = options
            .include
            .iter()
            .map(|glob| {
                let value = glob.as_str();
                if value.starts_with('/')
                    || value.split('/').any(|part| part == "..")
                    || value.starts_with('!')
                {
                    return Err(invalid(format!(
                        "{name}: include must stay within its config directory"
                    )));
                }
                value
                    .parse()
                    .map_err(|error| invalid(format!("{name}: invalid include: {error}")))
            })
            .collect::<Result<_, _>>()?;
        let excludes: Vec<ValidatedGlob> = ["**/node_modules/**", "**/.git/**", "**/.turbo/**"]
            .into_iter()
            .chain(
                options
                    .exclude
                    .as_deref()
                    .unwrap_or_default()
                    .iter()
                    .map(|glob| glob.as_str()),
            )
            .map(|s| s.parse().map_err(|error| invalid(format!("{error}"))))
            .collect::<Result<_, _>>()?;
        let mut files: Vec<_> =
            globwalk::globwalk(directory, &includes, &excludes, WalkType::Files)
                .map_err(|error| invalid(format!("{name}: {error}")))?
                .into_iter()
                .collect();
        files.sort();
        if files.is_empty() {
            return Err(invalid(format!("{name}: include matched no files")));
        }
        let mut children = Vec::new();
        let mut entries = Vec::new();
        for file in files {
            let physical = fs::canonicalize(file.as_std_path())?;
            if !physical.starts_with(&physical_directory) {
                return Err(invalid(format!(
                    "discovered file escapes config directory: {file}"
                )));
            }
            let relative = directory
                .anchor(&file)
                .map_err(|e| invalid(e.to_string()))?
                .to_unix()
                .to_string();
            if physical_directory.join(&relative) != physical {
                return Err(invalid(format!(
                    "symlinked file tasks are not supported: {file}"
                )));
            }
            let child_name = TaskName::from(format!("{name}@{}", encode_name(&relative)));
            if config.tasks.contains_key(&child_name) {
                return Err(invalid(format!(
                    "generated task collides with {child_name}"
                )));
            }
            let mut child = template.clone();
            child.experimental_files = None;
            child.command = Some(Spanned::new(RawCommand::Argv(
                argv.iter()
                    .map(|arg| {
                        let value = if arg.as_str() == FILE {
                            format!("./{relative}")
                        } else {
                            arg.to_string()
                        };
                        Spanned::new(UnescapedString::from(value))
                    })
                    .collect(),
            )));
            let mut inputs = shared_inputs.clone();
            // Escape literal paths before adding them to the glob-based hasher.
            inputs.push(input(&literal_glob(&relative)));
            if options.trace.unwrap_or(false) {
                entries.push((child_name.clone(), file));
            }
            child.inputs = Some(inputs);
            config.tasks.insert(child_name.clone(), Spanned::new(child));
            children.push(Spanned::new(UnescapedString::from(child_name.to_string())));
        }
        if options.trace.unwrap_or(false) {
            config
                .file_tasks
                .register(name.clone(), options.clone(), entries, tracer)?;
        }
        // A graph-only aggregate never executes the package's original script.
        // Selecting a child directly therefore uses exactly the same cache key.
        config.tasks.insert(
            name,
            Spanned::new(RawTaskDefinition {
                experimental_files: Some(options.clone()),
                command: Some(Spanned::new(RawCommand::OptOut)),
                depends_on: Some(Spanned::new(children)),
                ..Default::default()
            }),
        );
    }
    Ok(())
}

fn input(value: &str) -> Spanned<RawTaskInput> {
    Spanned::new(RawTaskInput::String(UnescapedString::from(
        value.to_owned(),
    )))
}

fn encode_name(path: &str) -> String {
    path.bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-') {
                (byte as char).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect()
}

fn literal_glob(path: &str) -> String {
    path.chars()
        .flat_map(|ch| {
            if matches!(
                ch,
                '*' | '?' | '[' | ']' | '{' | '}' | '(' | ')' | '!' | '\\'
            ) {
                vec!['\\', ch]
            } else {
                vec![ch]
            }
        })
        .collect()
}

/// Deliberately limited to explicit relative ESM imports. Guessing a runtime's
/// extension order, conditional exports, aliases, or computed imports can yield
/// false hits. Unsupported imports fail closed; authors can instead declare a
/// complete input set with trace:false. Non-import inputs remain explicit.
fn trace_explicit_inputs(
    repo_root: &AbsoluteSystemPath,
    entry: &AbsoluteSystemPath,
) -> Result<BTreeSet<String>, Error> {
    let physical_root = fs::canonicalize(repo_root.as_std_path())?;
    let mut pending = vec![entry.to_owned()];
    let mut seen = BTreeSet::new();
    while let Some(file) = pending.pop() {
        let physical = fs::canonicalize(file.as_std_path())?;
        let logical = repo_root
            .anchor(&file)
            .map_err(|e| invalid(e.to_string()))?;
        if physical_root.join(logical.as_str()) != physical {
            return Err(invalid(format!(
                "symlinked traced inputs are not supported: {file}"
            )));
        }
        let relative = physical
            .strip_prefix(&physical_root)
            .map_err(|_| invalid(format!("traced input escapes repository: {file}")))?;
        let relative = relative
            .to_str()
            .ok_or_else(|| invalid("non-UTF-8 input path"))?
            .replace('\\', "/");
        if !seen.insert(relative) {
            continue;
        }
        // Module interpretation can change when package or compiler configuration
        // changes. Include absent paths too so adding one invalidates the result.
        for ancestor in physical
            .ancestors()
            .skip(1)
            .take_while(|p| p.starts_with(&physical_root))
        {
            for config in [
                "package.json",
                "tsconfig.json",
                "jsconfig.json",
                "bunfig.toml",
            ] {
                let path = ancestor.join(config);
                let path = path
                    .strip_prefix(&physical_root)
                    .map_err(|e| invalid(e.to_string()))?;
                seen.insert(
                    path.to_str()
                        .ok_or_else(|| invalid("non-UTF-8 config path"))?
                        .replace('\\', "/"),
                );
            }
        }
        let extension = physical
            .extension()
            .and_then(|v| v.to_str())
            .unwrap_or_default();
        if extension == "json" {
            continue;
        }
        if !matches!(extension, "js" | "jsx" | "ts" | "tsx" | "mjs" | "mts") {
            return Err(invalid(format!("unsupported traced source: {file}")));
        }
        let text = fs::read_to_string(&physical)?;
        let imports = parse_explicit_imports(&file, &text)
            .map_err(|error| invalid(format!("{file}: {error}")))?;
        for specifier in imports {
            if specifier.starts_with("node:") || specifier == "bun:test" {
                continue;
            }
            if !(specifier.starts_with("./") || specifier.starts_with("../")) {
                return Err(invalid(format!(
                    "{file}: import {} needs explicit inputs and trace:false",
                    specifier
                )));
            }
            let parent = file
                .parent()
                .ok_or_else(|| invalid("input has no parent"))?;
            let dependency = AbsoluteSystemPathBuf::from_unknown(parent, &specifier);
            if !dependency.as_std_path().is_file() || dependency.as_std_path().extension().is_none()
            {
                return Err(invalid(format!(
                    "{file}: import {} must name an existing file with an explicit extension",
                    specifier
                )));
            }
            pending.push(dependency);
        }
    }
    Ok(seen)
}
