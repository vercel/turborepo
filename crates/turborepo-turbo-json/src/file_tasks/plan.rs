//! Discovery registers file tasks, but imports are traced only when the engine
//! reaches a task. The cache belongs to this configuration load, never to disk:
//! each invocation sees edits, new resolution candidates and changed configs.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use turbopath::AbsoluteSystemPathBuf;
use turborepo_errors::Spanned;
use turborepo_task_id::TaskName;
use turborepo_unescape::UnescapedString;

use super::{
    input, invalid, literal_glob,
    trace::{FileTracer, TracedInputs},
};
use crate::{
    Error, Pipeline, RawTaskDefinition,
    raw::{RawCommand, RawFileTasks},
};

#[derive(Debug, Clone, PartialEq)]
struct Entry {
    file: AbsoluteSystemPathBuf,
    template: usize,
}

#[derive(Debug, Clone, PartialEq)]
struct Template {
    name: TaskName<'static>,
    options: RawFileTasks,
}

#[derive(Debug, Default)]
struct TraceState {
    tracers: Vec<FileTracer>,
    tasks: HashMap<TaskName<'static>, RawTaskDefinition>,
}

#[derive(Debug, Default, Clone)]
pub(crate) struct FileTasks {
    entries: HashMap<TaskName<'static>, Entry>,
    templates: Vec<Template>,
    state: Arc<Mutex<TraceState>>,
}

impl PartialEq for FileTasks {
    fn eq(&self, other: &Self) -> bool {
        // Memoized resolution does not change the configuration's identity.
        self.entries == other.entries && self.templates == other.templates
    }
}

impl FileTasks {
    pub(super) fn register(
        &mut self,
        name: TaskName<'static>,
        options: RawFileTasks,
        entries: Vec<(TaskName<'static>, AbsoluteSystemPathBuf)>,
        tracer: FileTracer,
    ) -> Result<(), Error> {
        let template = self.templates.len();
        self.templates.push(Template { name, options });
        self.entries.extend(
            entries
                .into_iter()
                .map(|(name, file)| (name, Entry { file, template })),
        );
        self.state
            .lock()
            .map_err(|_| invalid("file trace cache poisoned"))?
            .tracers
            .push(tracer);
        Ok(())
    }

    pub(crate) fn contains_source(&self, name: &TaskName) -> bool {
        !self.templates.is_empty()
            && !self.entries.contains_key(name)
            && self
                .state
                .lock()
                .is_ok_and(|state| state.tasks.contains_key(name))
    }

    pub(crate) fn task(
        &self,
        name: &TaskName,
        definitions: &Pipeline,
    ) -> Result<Option<RawTaskDefinition>, Error> {
        if self.templates.is_empty() {
            return Ok(definitions.get(name).map(|task| task.value.clone()));
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| invalid("file trace cache poisoned"))?;
        if let Some(task) = state.tasks.get(name) {
            return Ok(Some(task.clone()));
        }
        let Some(mut task) = definitions.get(name).map(|task| task.value.clone()) else {
            return Ok(None);
        };
        let Some(entry) = self.entries.get(name) else {
            return Ok(Some(task));
        };
        let template = &self.templates[entry.template];
        let TraceState { tracers, tasks } = &mut *state;
        let tracer = &mut tracers[entry.template];
        let dependencies = match tracer.inputs(&entry.file) {
            Ok(dependencies) => dependencies,
            Err(error)
                if template
                    .options
                    .fallback_inputs
                    .as_ref()
                    .is_some_and(|v| !v.is_empty()) =>
            {
                tracing::warn!(task = %name, %error, "using declared fallbackInputs for incomplete trace");
                task.inputs.get_or_insert_with(Vec::new).extend(
                    template
                        .options
                        .fallback_inputs
                        .as_deref()
                        .unwrap_or_default()
                        .iter()
                        .map(|path| input(path.as_str())),
                );
                TracedInputs::default()
            }
            Err(error) => return Err(error),
        };
        task.inputs.get_or_insert_with(Vec::new).extend(
            dependencies
                .paths
                .iter()
                .map(|path| input(&format!("$TURBO_ROOT$/{}", literal_glob(path)))),
        );
        // Shared graph-only nodes retain the same identity and hash whether
        // reached through one child, an ordinary task dependency, or the family.
        for module in dependencies.modules {
            let source_name = TaskName::from(format!(
                "{}@source@{}",
                template.name,
                tracer.module_name(&module)?
            ));
            if definitions.contains_key(&source_name) {
                return Err(invalid(format!(
                    "generated task collides with {source_name}"
                )));
            }
            if !tasks.contains_key(&source_name) {
                let inputs = tracer
                    .module_inputs(&module)?
                    .iter()
                    .map(|path| input(&format!("$TURBO_ROOT$/{}", literal_glob(path))))
                    .collect();
                tasks.insert(
                    source_name.clone(),
                    RawTaskDefinition {
                        command: Some(Spanned::new(RawCommand::OptOut)),
                        inputs: Some(inputs),
                        ..Default::default()
                    },
                );
            }
            task.depends_on
                .get_or_insert_with(|| Spanned::new(Vec::new()))
                .value
                .push(Spanned::new(UnescapedString::from(source_name.to_string())));
        }
        tasks.insert(name.clone().into_owned(), task.clone());
        Ok(Some(task))
    }
}
