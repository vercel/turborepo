use std::{
    borrow::Cow,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context as TaskContext, Poll},
    thread::available_parallelism,
    time::Duration,
};

use anyhow::{anyhow, Context, Result};
use futures::Stream;
use futures_retry::{FutureRetry, RetryPolicy};
use indexmap::indexmap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value as JsonValue;
use turbo_tasks::{
    primitives::{JsonValueVc, StringVc},
    CompletionVc, TryJoinIterExt, Value, ValueToString,
};
use turbo_tasks_env::{ProcessEnv, ProcessEnvVc};
use turbo_tasks_fs::{
    glob::GlobVc, rope::Rope, to_sys_path, DirectoryEntry, File, FileSystemPathVc, ReadGlobResultVc,
};
use turbopack_core::{
    asset::{Asset, AssetVc},
    chunk::{ChunkGroupVc, ChunkingContext, ChunkingContextVc},
    context::{AssetContext, AssetContextVc},
    ident::AssetIdentVc,
    issue::{Issue, IssueSeverity, IssueSeverityVc, IssueVc},
    source_asset::SourceAssetVc,
    virtual_asset::VirtualAssetVc,
};
use turbopack_ecmascript::{
    chunk::EcmascriptChunkPlaceablesVc, EcmascriptInputTransform, EcmascriptInputTransformsVc,
    EcmascriptModuleAssetType, EcmascriptModuleAssetVc, InnerAssetsVc,
};

use crate::{
    bootstrap::NodeJsBootstrapAsset,
    embed_js::embed_file_path,
    emit, emit_package_json,
    pool::{NodeJsOperation, NodeJsPool, NodeJsPoolVc},
    StructuredError,
};

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum EvalJavaScriptOutgoingMessage<'a> {
    #[serde(rename_all = "camelCase")]
    Evaluate { args: Vec<&'a JsonValue> },
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum EvalJavaScriptIncomingMessage {
    FileDependency {
        path: String,
    },
    BuildDependency {
        path: String,
    },
    DirDependency {
        path: String,
        glob: String,
    },
    EmittedError {
        severity: IssueSeverity,
        error: StructuredError,
    },
    Value {
        data: String,
    },
    End {
        data: Option<String>,
    },
    Error(StructuredError),
}

#[turbo_tasks::value(shared)]
#[derive(Clone, Debug)]
pub enum JavaScriptValue {
    Error,
    Empty,
    Value(Rope),
}

impl From<String> for JavaScriptValue {
    fn from(value: String) -> Self {
        JavaScriptValue::Value(value.into())
    }
}

impl From<Option<String>> for JavaScriptValue {
    fn from(value: Option<String>) -> Self {
        match value {
            Some(v) => v.into(),
            None => JavaScriptValue::Empty,
        }
    }
}

#[turbo_tasks::value(shared)]
#[derive(Clone)]
pub enum JavaScriptEvaluation {
    Single(JavaScriptValue),
    Stream(JavaScriptStream),
}

#[turbo_tasks::value(shared, eq = "manual", serialization = "custom")]
#[derive(Clone)]
pub struct JavaScriptStream(#[turbo_tasks(trace_ignore, debug_ignore)] Arc<Mutex<JsStreamInner>>);

impl JavaScriptStream {
    fn new(data: Vec<JavaScriptValue>) -> Self {
        JavaScriptStream(Arc::new(Mutex::new(JsStreamInner { done: false, data })))
    }

    fn into_stream(self) -> JsStreamable {
        JsStreamable {
            inner: self,
            index: 0,
        }
    }
}

pub struct JsStreamable {
    inner: JavaScriptStream,
    index: usize,
}

impl Stream for JsStreamable {
    type Item = Result<Rope>;

    fn poll_next(self: Pin<&mut Self>, _cx: &mut TaskContext<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        let inner = this.inner.0.lock().unwrap();
        inner.data.get(this.index).map_or_else(
            || {
                if inner.done {
                    Poll::Ready(None)
                } else {
                    Poll::Pending
                }
            },
            |data| match data {
                JavaScriptValue::Empty => unreachable!(),
                JavaScriptValue::Value(v) => Poll::Ready(Some(Ok(v.clone()))),
                JavaScriptValue::Error => {
                    Poll::Ready(Some(Err(anyhow!("error during evaluation"))))
                }
            },
        )
    }
}

impl PartialEq for JavaScriptStream {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0) || {
            let inner = self.0.lock().unwrap();
            let other = other.0.lock().unwrap();
            *inner == *other
        }
    }
}
impl Eq for JavaScriptStream {}

impl Serialize for JavaScriptStream {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::Error;
        let lock = self.0.lock().map_err(Error::custom)?;
        if !lock.done {
            return Err(Error::custom("cannot serialize unfinished stream"));
        }
        lock.data.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for JavaScriptStream {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let data = <Vec<JavaScriptValue>>::deserialize(deserializer)?;
        Ok(JavaScriptStream::new(data))
    }
}

#[derive(Eq, PartialEq)]
pub struct JsStreamInner {
    done: bool,
    data: Vec<JavaScriptValue>,
}

#[turbo_tasks::function]
/// Pass the file you cared as `runtime_entries` to invalidate and reload the
/// evaluated result automatically.
pub async fn get_evaluate_pool(
    module_asset: AssetVc,
    cwd: FileSystemPathVc,
    env: ProcessEnvVc,
    context: AssetContextVc,
    chunking_context: ChunkingContextVc,
    runtime_entries: Option<EcmascriptChunkPlaceablesVc>,
    additional_invalidation: CompletionVc,
    debug: bool,
) -> Result<NodeJsPoolVc> {
    let runtime_asset = EcmascriptModuleAssetVc::new(
        SourceAssetVc::new(embed_file_path("ipc/evaluate.ts")).into(),
        context,
        Value::new(EcmascriptModuleAssetType::Typescript),
        EcmascriptInputTransformsVc::cell(vec![EcmascriptInputTransform::TypeScript {
            use_define_for_class_fields: false,
        }]),
        context.compile_time_info(),
    )
    .as_asset();

    let module_path = module_asset.ident().path().await?;
    let file_name = module_path.file_name();
    let file_name = if file_name.ends_with(".js") {
        Cow::Borrowed(file_name)
    } else if let Some(file_name) = file_name.strip_suffix(".ts") {
        Cow::Owned(format!("{file_name}.js"))
    } else {
        Cow::Owned(format!("{file_name}.js"))
    };
    let path = chunking_context.output_root().join(file_name.as_ref());
    let entry_module = EcmascriptModuleAssetVc::new_with_inner_assets(
        VirtualAssetVc::new(
            runtime_asset.ident().path().join("evaluate.js"),
            File::from(
                "import { run } from 'RUNTIME'; run((...args) => \
                 (require('INNER').default(...args)))",
            )
            .into(),
        )
        .into(),
        context,
        Value::new(EcmascriptModuleAssetType::Typescript),
        EcmascriptInputTransformsVc::cell(vec![EcmascriptInputTransform::TypeScript {
            use_define_for_class_fields: false,
        }]),
        context.compile_time_info(),
        InnerAssetsVc::cell(indexmap! {
            "INNER".to_string() => module_asset,
            "RUNTIME".to_string() => runtime_asset
        }),
    );

    let (Some(cwd), Some(entrypoint)) = (to_sys_path(cwd).await?, to_sys_path(path).await?) else {
        panic!("can only evaluate from a disk filesystem");
    };

    let runtime_entries = {
        let globals_module = EcmascriptModuleAssetVc::new(
            SourceAssetVc::new(embed_file_path("globals.ts")).into(),
            context,
            Value::new(EcmascriptModuleAssetType::Typescript),
            EcmascriptInputTransformsVc::cell(vec![EcmascriptInputTransform::TypeScript {
                use_define_for_class_fields: false,
            }]),
            context.compile_time_info(),
        )
        .as_ecmascript_chunk_placeable();

        let mut entries = vec![globals_module];
        if let Some(other_entries) = runtime_entries {
            for entry in &*other_entries.await? {
                entries.push(*entry)
            }
        };

        Some(EcmascriptChunkPlaceablesVc::cell(entries))
    };

    let bootstrap = NodeJsBootstrapAsset {
        path,
        chunk_group: ChunkGroupVc::from_chunk(
            entry_module.as_evaluated_chunk(chunking_context, runtime_entries),
        ),
    };
    emit_package_json(chunking_context.output_root()).await?;
    emit(bootstrap.cell().into(), chunking_context.output_root()).await?;
    let pool = NodeJsPool::new(
        cwd,
        entrypoint,
        env.read_all()
            .await?
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
        available_parallelism().map_or(1, |v| v.get()),
        debug,
    );
    additional_invalidation.await?;
    Ok(pool.cell())
}

struct PoolErrorHandler;

/// Number of attempts before we start slowing down the retry.
const MAX_FAST_ATTEMPTS: usize = 5;
/// Total number of attempts.
const MAX_ATTEMPTS: usize = MAX_FAST_ATTEMPTS * 2;

impl futures_retry::ErrorHandler<anyhow::Error> for PoolErrorHandler {
    type OutError = anyhow::Error;

    fn handle(&mut self, attempt: usize, err: anyhow::Error) -> RetryPolicy<Self::OutError> {
        if attempt >= MAX_ATTEMPTS {
            RetryPolicy::ForwardError(err)
        } else if attempt >= MAX_FAST_ATTEMPTS {
            RetryPolicy::WaitRetry(Duration::from_secs(1))
        } else {
            RetryPolicy::Repeat
        }
    }
}

/// Pass the file you cared as `runtime_entries` to invalidate and reload the
/// evaluated result automatically.
#[turbo_tasks::function]
pub async fn evaluate(
    module_asset: AssetVc,
    cwd: FileSystemPathVc,
    env: ProcessEnvVc,
    context_ident_for_issue: AssetIdentVc,
    context: AssetContextVc,
    chunking_context: ChunkingContextVc,
    runtime_entries: Option<EcmascriptChunkPlaceablesVc>,
    args: Vec<JsonValueVc>,
    additional_invalidation: CompletionVc,
    debug: bool,
) -> Result<JavaScriptEvaluationVc> {
    let pool = get_evaluate_pool(
        module_asset,
        cwd,
        env,
        context,
        chunking_context,
        runtime_entries,
        additional_invalidation,
        debug,
    )
    .await?;

    let args = args.into_iter().try_join().await?;
    // Assume this is a one-off operation, so we can kill the process
    // TODO use a better way to decide that.
    let kill = args.is_empty();

    // Workers in the pool could be in a bad state that we didn't detect yet.
    // The bad state might even be unnoticable until we actually send the job to the
    // worker. So we retry picking workers from the pools until we succeed
    // sending the job.

    let (mut operation, _) = FutureRetry::new(
        || async {
            let mut operation = pool.operation().await?;
            operation
                .send(EvalJavaScriptOutgoingMessage::Evaluate {
                    args: args.iter().map(|v| &**v).collect(),
                })
                .await?;
            Ok(operation)
        },
        PoolErrorHandler,
    )
    .await
    .map_err(|(e, _)| e)?;

    let output = loop_operation(&mut operation, cwd, context_ident_for_issue).await?;

    let data = match output {
        EvalJavaScriptIncomingMessage::End { data } => {
            if kill {
                operation.wait_or_kill().await?;
            }
            return Ok(JavaScriptEvaluation::Single(data.into()).cell());
        }
        EvalJavaScriptIncomingMessage::Error(_) => {
            return Ok(JavaScriptEvaluation::Single(JavaScriptValue::Error).cell());
        }
        EvalJavaScriptIncomingMessage::Value { data } => data,
        _ => unreachable!("file dep messages cannot leak out of loop"),
    };

    let stream = JavaScriptStream::new(vec![data.into()]);
    let inner = stream.0.clone();
    tokio::spawn(async move {
        let inner = inner.clone();
        loop {
            let output = loop_operation(&mut operation, cwd, context_ident_for_issue).await?;
            let mut lock = inner.lock().unwrap();

            match output {
                EvalJavaScriptIncomingMessage::End { data } => {
                    lock.done = true;
                    if let Some(data) = data {
                        lock.data.push(data.into());
                    };
                    break;
                }
                EvalJavaScriptIncomingMessage::Error(_) => {
                    lock.done = true;
                    lock.data.push(JavaScriptValue::Error);
                    break;
                }
                EvalJavaScriptIncomingMessage::Value { data } => {
                    lock.data.push(data.into());
                }
                _ => unreachable!("file dep messages cannot leak out of loop"),
            }
        }

        if kill {
            operation.wait_or_kill().await?;
        }
        Result::<()>::Ok(())
    });

    Ok(JavaScriptEvaluation::Stream(stream).cell())
}

async fn loop_operation(
    operation: &mut NodeJsOperation,
    cwd: FileSystemPathVc,
    context_ident_for_issue: AssetIdentVc,
) -> Result<EvalJavaScriptIncomingMessage> {
    let mut file_dependencies = Vec::new();
    let mut dir_dependencies = Vec::new();

    let output = loop {
        match operation.recv().await? {
            EvalJavaScriptIncomingMessage::Error(error) => {
                EvaluationIssue {
                    error: error.clone(),
                    context_ident: context_ident_for_issue,
                    cwd,
                }
                .cell()
                .as_issue()
                .emit();
                // Do not reuse the process in case of error
                operation.disallow_reuse();
                break EvalJavaScriptIncomingMessage::Error(error);
            }
            value @ EvalJavaScriptIncomingMessage::Value { .. } => {
                break value;
            }
            value @ EvalJavaScriptIncomingMessage::End { .. } => {
                break value;
            }
            EvalJavaScriptIncomingMessage::FileDependency { path } => {
                // TODO We might miss some changes that happened during execution
                file_dependencies.push(cwd.join(&path).read());
            }
            EvalJavaScriptIncomingMessage::BuildDependency { path } => {
                // TODO We might miss some changes that happened during execution
                BuildDependencyIssue {
                    context_ident: context_ident_for_issue,
                    path: cwd.join(&path),
                }
                .cell()
                .as_issue()
                .emit();
            }
            EvalJavaScriptIncomingMessage::DirDependency { path, glob } => {
                // TODO We might miss some changes that happened during execution
                dir_dependencies.push(dir_dependency(
                    cwd.join(&path).read_glob(GlobVc::new(&glob), false),
                ));
            }
            EvalJavaScriptIncomingMessage::EmittedError { error, severity } => {
                EvaluateEmittedErrorIssue {
                    context: context_ident_for_issue.path(),
                    cwd,
                    error,
                    severity: severity.cell(),
                }
                .cell()
                .as_issue()
                .emit();
            }
        }
    };

    // Read dependencies to make them a dependencies of this task. This task will
    // execute again when they change.
    for dep in file_dependencies {
        dep.await?;
    }
    for dep in dir_dependencies {
        dep.await?;
    }

    Ok(output)
}

/// An issue that occurred while evaluating node code.
#[turbo_tasks::value(shared)]
pub struct EvaluationIssue {
    pub context_ident: AssetIdentVc,
    pub cwd: FileSystemPathVc,
    pub error: StructuredError,
}

#[turbo_tasks::value_impl]
impl Issue for EvaluationIssue {
    #[turbo_tasks::function]
    fn title(&self) -> StringVc {
        StringVc::cell("Error evaluating Node.js code".to_string())
    }

    #[turbo_tasks::function]
    fn category(&self) -> StringVc {
        StringVc::cell("build".to_string())
    }

    #[turbo_tasks::function]
    fn context(&self) -> FileSystemPathVc {
        self.context_ident.path()
    }

    #[turbo_tasks::function]
    async fn description(&self) -> Result<StringVc> {
        let cwd = to_sys_path(self.cwd.root())
            .await?
            .context("Must have path on disk")?;

        Ok(StringVc::cell(
            self.error
                .print(Default::default(), &cwd.to_string_lossy())
                .await?,
        ))
    }
}

/// An issue that occurred while evaluating node code.
#[turbo_tasks::value(shared)]
pub struct BuildDependencyIssue {
    pub context_ident: AssetIdentVc,
    pub path: FileSystemPathVc,
}

#[turbo_tasks::value_impl]
impl Issue for BuildDependencyIssue {
    #[turbo_tasks::function]
    fn severity(&self) -> IssueSeverityVc {
        IssueSeverity::Warning.into()
    }

    #[turbo_tasks::function]
    fn title(&self) -> StringVc {
        StringVc::cell("Build dependencies are not yet supported".to_string())
    }

    #[turbo_tasks::function]
    fn category(&self) -> StringVc {
        StringVc::cell("build".to_string())
    }

    #[turbo_tasks::function]
    fn context(&self) -> FileSystemPathVc {
        self.context_ident.path()
    }

    #[turbo_tasks::function]
    async fn description(&self) -> Result<StringVc> {
        Ok(StringVc::cell(
            format!("The file at {} is a build dependency, which is not yet implemented.
Changing this file or any dependency will not be recognized and might require restarting the server", self.path.to_string().await?)
        ))
    }
}

/// A hack to invalidate when any file in a directory changes. Need to be
/// awaited before files are accessed.
#[turbo_tasks::function]
async fn dir_dependency(glob: ReadGlobResultVc) -> Result<CompletionVc> {
    let shallow = dir_dependency_shallow(glob);
    let glob = glob.await?;
    glob.inner
        .values()
        .map(|&inner| dir_dependency(inner))
        .try_join()
        .await?;
    shallow.await?;
    Ok(CompletionVc::new())
}

#[turbo_tasks::function]
async fn dir_dependency_shallow(glob: ReadGlobResultVc) -> Result<CompletionVc> {
    let glob = glob.await?;
    for item in glob.results.values() {
        // Reading all files to add itself as dependency
        match *item {
            DirectoryEntry::File(file) => {
                file.track().await?;
            }
            DirectoryEntry::Directory(dir) => {
                dir_dependency(dir.read_glob(GlobVc::new("**"), false)).await?;
            }
            DirectoryEntry::Symlink(symlink) => {
                symlink.read_link().await?;
            }
            DirectoryEntry::Other(other) => {
                other.get_type().await?;
            }
            DirectoryEntry::Error => {}
        }
    }
    Ok(CompletionVc::new())
}

#[turbo_tasks::value(shared)]
pub struct EvaluateEmittedErrorIssue {
    pub context: FileSystemPathVc,
    pub cwd: FileSystemPathVc,
    pub severity: IssueSeverityVc,
    pub error: StructuredError,
}

#[turbo_tasks::value_impl]
impl Issue for EvaluateEmittedErrorIssue {
    #[turbo_tasks::function]
    fn context(&self) -> FileSystemPathVc {
        self.context
    }

    #[turbo_tasks::function]
    fn severity(&self) -> IssueSeverityVc {
        self.severity
    }

    #[turbo_tasks::function]
    fn category(&self) -> StringVc {
        StringVc::cell("loaders".to_string())
    }

    #[turbo_tasks::function]
    fn title(&self) -> StringVc {
        StringVc::cell("Issue while running loader".to_string())
    }

    #[turbo_tasks::function]
    async fn description(&self) -> Result<StringVc> {
        let root = to_sys_path(self.cwd.root())
            .await?
            .context("Must have path on disk")?;

        Ok(StringVc::cell(
            self.error
                .print(Default::default(), &root.to_string_lossy())
                .await?,
        ))
    }
}
