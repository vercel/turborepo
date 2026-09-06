//! Turbo LSP server
//!
//! This is the main entry point for the LSP server. It is responsible for
//! handling all LSP requests and responses.
//!
//! For more, see the [LSP specification](https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/)
//! as well as the architecture documentation in `packages/turbo-vsc`.

#![feature(box_patterns)]
// miette's derive macro causes false positives for this lint
#![allow(unused_assignments)]
#![deny(clippy::all)]
#![warn(clippy::unwrap_used)]

use std::{
    borrow::Cow,
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex, MutexGuard},
};

use itertools::Itertools;
use jsonc_parser::{
    CollectOptions,
    ast::{ObjectPropName, StringLit},
};
use serde_json::Value;
use tokio::sync::watch::{Receiver, Sender};
use tower_lsp::{
    Client, LanguageServer, LspService, Server,
    jsonrpc::{Error, Result as LspResult},
    lsp_types::*,
};
use turbopath::AbsoluteSystemPathBuf;
use turborepo_daemon::{
    DaemonClient, DaemonConnector, DaemonConnectorError, DaemonError, DaemonPackageDiscovery,
    Paths as DaemonPaths,
};
use turborepo_repository::{
    discovery::{self, PackageDiscovery},
    inference::RepoState,
    native_tasks::observation_from_package_json,
    package_graph::{self, PackageGraph, PackageName},
    package_json::PackageJson,
};

const TURBO_EXTENDS: &str = "$TURBO_EXTENDS$";

/// Script source joined to identity and paths supplied by `PackageGraph`.
///
/// The parsed manifest remains a payload because LSP features need scripts and
/// source ranges. It is deliberately not consulted for package identity or
/// location after the graph has been built.
#[derive(Debug)]
struct LspPackage {
    /// `None` is a parsed compatibility source that repository knowledge did
    /// not represent (notably an unnamed workspace).
    identity: Option<PackageName>,
    source_path: AbsoluteSystemPathBuf,
    source: String,
    package_json: PackageJson,
}

#[derive(Debug)]
struct PackageSource {
    text: String,
    package_json: PackageJson,
}

#[derive(Debug, Default)]
struct LspPackages {
    packages: Vec<LspPackage>,
}

impl LspPackages {
    fn from_graph_or_unscoped(
        graph: Result<PackageGraph, package_graph::Error>,
        mut sources: HashMap<AbsoluteSystemPathBuf, PackageSource>,
        root_path: &AbsoluteSystemPathBuf,
    ) -> Self {
        match graph {
            Ok(graph) => Self::from_graph(&graph, sources),
            Err(_) => {
                let root = sources.remove(root_path).map(|source| LspPackage {
                    identity: Some(PackageName::Root),
                    source_path: root_path.clone(),
                    source: source.text,
                    package_json: source.package_json,
                });
                let mut packages = Self::unscoped(sources).packages;
                packages.extend(root);
                Self::new(packages)
            }
        }
    }

    fn from_graph(
        graph: &PackageGraph,
        mut sources: HashMap<AbsoluteSystemPathBuf, PackageSource>,
    ) -> Self {
        let mut packages = Vec::with_capacity(sources.len());
        for (identity, _) in graph.package_scope_directories() {
            let Some(definition_path) = graph.package_definition_path(&identity) else {
                continue;
            };
            let definition_path = graph.repo_root().resolve(definition_path);
            // Remove every graph-owned source before feature filtering. An
            // aggregate or native package must not reappear as an unscoped JS
            // compatibility package.
            let Some(source) = sources.remove(&definition_path) else {
                continue;
            };
            // Presence in `sources` proves this authoritative definition was
            // parsed as package.json; provenance does not determine LSP
            // package capabilities.
            if graph.is_aggregate_scope(&identity) {
                continue;
            }
            packages.push(LspPackage {
                identity: Some(identity),
                source_path: definition_path,
                source: source.text,
                package_json: source.package_json,
            });
        }

        packages.extend(sources.into_iter().map(|(source_path, source)| LspPackage {
            identity: None,
            source_path,
            source: source.text,
            package_json: source.package_json,
        }));
        Self::new(packages)
    }

    fn unscoped(sources: HashMap<AbsoluteSystemPathBuf, PackageSource>) -> Self {
        Self::new(
            sources
                .into_iter()
                .map(|(source_path, source)| LspPackage {
                    identity: None,
                    source_path,
                    source: source.text,
                    package_json: source.package_json,
                })
                .collect(),
        )
    }

    fn new(mut packages: Vec<LspPackage>) -> Self {
        packages.sort_by(|left, right| {
            package_sort_key(left)
                .cmp(&package_sort_key(right))
                .then_with(|| left.source_path.cmp(&right.source_path))
        });
        Self { packages }
    }

    /// Map an unsaved/in-memory package.json payload to the same native-task
    /// observation vocabulary used by the repository catalog.
    fn observed_task_names(package: &LspPackage) -> Vec<String> {
        let scope = package
            .identity
            .as_ref()
            .map(|identity| identity.as_str())
            .unwrap_or("//");
        observation_from_package_json(scope, &package.package_json)
            .tasks
            .into_iter()
            .filter(|task| task.script().is_some())
            .map(|task| task.name().to_string())
            .collect()
    }

    fn task_index(&self) -> HashMap<String, Vec<Option<String>>> {
        let mut tasks = HashMap::<String, Vec<Option<String>>>::new();
        for package in &self.packages {
            let identity = package
                .identity
                .as_ref()
                .map(|identity| identity.as_str().to_string());
            for script in Self::observed_task_names(package) {
                let identities = tasks.entry(script).or_default();
                if !identities.contains(&identity) {
                    identities.push(identity.clone());
                }
            }
        }
        tasks
    }

    fn completion_labels(&self) -> Vec<String> {
        let qualified = self.packages.iter().flat_map(|package| {
            package.identity.iter().flat_map(|identity| {
                Self::observed_task_names(package)
                    .into_iter()
                    .map(move |script| format!("{}#{script}", identity.as_str()))
            })
        });
        let tasks = self.packages.iter().flat_map(Self::observed_task_names);

        qualified.chain(tasks).unique().collect()
    }

    fn references(&self, referenced_task: &str) -> Vec<Location> {
        let (requested_package, task) = referenced_task
            .rsplit_once('#')
            .map(|(package, task)| (Some(package), task))
            .unwrap_or((None, referenced_task));

        self.packages
            .iter()
            .filter(|package| {
                requested_package.is_none_or(|requested| {
                    package
                        .identity
                        .as_ref()
                        .is_some_and(|identity| requested == identity.as_str())
                }) && Self::observed_task_names(package)
                    .iter()
                    .any(|name| name == task)
            })
            .filter_map(|package| {
                // TODO: use jsonc_ast instead of text search.
                let start = package.source.find(&format!("\"{task}\""))?;
                let end = start + task.len() + 2;
                let rope = crop::Rope::from(package.source.as_str());
                let start_line = rope.line_of_byte(start);
                let end_line = rope.line_of_byte(end);
                let range = Range {
                    start: Position {
                        line: start_line as u32,
                        character: (start - rope.byte_of_line(start_line)) as u32,
                    },
                    end: Position {
                        line: end_line as u32,
                        character: (end - rope.byte_of_line(end_line)) as u32,
                    },
                };
                let uri = Url::from_file_path(&package.source_path).ok()?;
                Some(Location::new(uri, range))
            })
            .collect()
    }
}

#[derive(Debug, Default)]
struct LspPackageCache(Mutex<Option<Arc<LspPackages>>>);

impl LspPackageCache {
    fn get(&self) -> Option<Arc<LspPackages>> {
        lock_or_recover(&self.0).clone()
    }

    fn set(&self, packages: Arc<LspPackages>) {
        *lock_or_recover(&self.0) = Some(packages);
    }

    fn invalidate(&self) {
        lock_or_recover(&self.0).take();
    }
}

fn retain_unique_named(package_jsons: &mut HashMap<AbsoluteSystemPathBuf, PackageJson>) {
    let counts = package_jsons
        .values()
        .filter_map(|package_json| {
            package_json
                .name
                .as_ref()
                .map(|name| name.as_str().to_string())
        })
        .counts();
    package_jsons.retain(|_, package_json| {
        package_json
            .name
            .as_ref()
            .is_some_and(|name| counts.get(name.as_str()) == Some(&1))
    });
}

fn package_sort_key(package: &LspPackage) -> (u8, &str) {
    match package.identity.as_ref() {
        Some(PackageName::Root) => (0, ""),
        Some(PackageName::Other(identity)) => (1, identity),
        None => (2, ""),
    }
}

pub struct Backend {
    client: Client,
    repo_root: Arc<Mutex<Option<AbsoluteSystemPathBuf>>>,
    files: Mutex<HashMap<Url, crop::Rope>>,
    initializer: Sender<Option<DaemonClient<DaemonConnector>>>,
    daemon: Receiver<Option<DaemonClient<DaemonConnector>>>,
    packages: LspPackageCache,

    // this is only used for turbo optimize
    pidlock: Mutex<Option<pidlock::Pidlock>>,
}

pub fn run_lsp_server() {
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(err) => {
            eprintln!("failed to build tokio runtime: {err}");
            return;
        }
    };

    runtime.block_on(run_lsp());
}

pub async fn run_lsp() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = LspService::new(Backend::new);
    Server::new(stdin, stdout, socket).serve(service).await;
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, params: InitializeParams) -> LspResult<InitializeResult> {
        if let Some(uri) = params.root_uri {
            if uri.scheme() != "file" {
                return Err(Error::invalid_params("root is not a local file"));
            }

            let repo_root = uri
                .to_file_path()
                .map_err(|_| Error::invalid_params("root is not a local file"))?;
            let repo_root = repo_root
                .as_os_str()
                .to_str()
                .ok_or(Error::invalid_params("root is not a valid utf-8 path"))?;

            // convert uri file:///absolute-path to AbsoluteSystemPathBuf
            let repo_root = AbsoluteSystemPathBuf::new(repo_root)
                .map_err(|_| Error::invalid_params("root is not an absolute path"))?;

            // Walk up from the editor's root_uri to find the actual monorepo
            // root. In multi-root VSCode workspaces the editor can send a
            // subdirectory, which would cause the daemon to start with the
            // wrong root and write cookie files in the wrong location.
            let repo_root = match RepoState::infer(&repo_root) {
                Ok(state) => state.root,
                Err(_) => repo_root,
            };

            self.packages.invalidate();
            lock_or_recover(&self.repo_root).replace(repo_root.clone());

            let paths = DaemonPaths::from_repo_root(&repo_root).map_err(|err| {
                let mut error = Error::internal_error();
                error.message = format!("failed to construct daemon paths: {err}").into();
                error
            })?;

            let (_, daemon) = tokio::join!(
                self.client
                    .log_message(MessageType::INFO, format!("root uri: {}", paths.sock_file),),
                tokio_retry::Retry::spawn(
                    tokio_retry::strategy::FixedInterval::from_millis(100).take(5),
                    || async {
                        let can_start_server = true;
                        let can_kill_server = true;
                        let connector = DaemonConnector::new(
                            can_start_server,
                            can_kill_server,
                            &repo_root,
                            None,
                        )?;
                        connector.connect().await
                    },
                )
            );

            let daemon = match daemon {
                Ok(daemon) => daemon,
                Err(DaemonConnectorError::Handshake(box DaemonError::VersionMismatch(message))) => {
                    self.client
                        .show_message(
                            MessageType::ERROR,
                            "Pre-2.0 versions of turborepo are not compatible with 2.0 or later \
                             of the extension. If you do not plan to update to turbo 2.0, please \
                             ensure you install the latest 1.0 version of the extension in this \
                             workspace.",
                        )
                        .await;
                    self.client
                        .log_message(
                            MessageType::ERROR,
                            format!("version mismatch when connecting to daemon: {message}"),
                        )
                        .await;

                    // in this case, just say we don't support any features
                    return Ok(InitializeResult {
                        ..Default::default()
                    });
                }
                Err(e) => {
                    self.client
                        .show_message(
                            MessageType::ERROR,
                            "Turborepo language server failed to connect to the daemon. See the \
                             output for details.",
                        )
                        .await;
                    self.client
                        .log_message(
                            MessageType::ERROR,
                            format!("failed to connect to daemon: {e}"),
                        )
                        .await;
                    return Ok(InitializeResult {
                        ..Default::default()
                    });
                }
            };

            self.initializer
                .send(Some(daemon))
                .map_err(|_| Error::internal_error())?;

            let mut lock = pidlock::Pidlock::new(paths.lsp_pid_file.as_std_path().to_owned());

            match lock.acquire() {
                Ok(()) => {
                    *lock_or_recover(&self.pidlock) = Some(lock);
                }
                Err(pidlock::PidlockError::AlreadyOwned) => {
                    // Another LSP instance (e.g., another VSCode window) already holds the lock.
                    // This lock is only used for exclusive optimize features, so proceed without
                    // it.
                    self.client
                        .log_message(
                            MessageType::INFO,
                            "another LSP instance holds lock; continuing without exclusive \
                             optimize features.",
                        )
                        .await;
                }
                Err(e) => {
                    self.client
                        .show_message(
                            MessageType::ERROR,
                            "Turborepo language server failed to initialize. See the output for \
                             details.",
                        )
                        .await;
                    self.client
                        .log_message(
                            MessageType::ERROR,
                            format!("Failed to acquire pidlock: {e}"),
                        )
                        .await;
                    return Ok(InitializeResult {
                        ..Default::default()
                    });
                }
            }
        }

        Ok(InitializeResult {
            server_info: None,
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::INCREMENTAL,
                )),
                completion_provider: Some(CompletionOptions {
                    resolve_provider: Some(false),
                    trigger_characters: Some(vec![".".to_string()]),
                    work_done_progress_options: Default::default(),
                    all_commit_characters: None,
                    ..Default::default()
                }),
                execute_command_provider: Some(ExecuteCommandOptions {
                    commands: vec![
                        // "turbo.run".to_string(),
                        // todo: port these from JS land
                        // "turbo.daemon.start".to_string(),
                        // "turbo.daemon.status".to_string(),
                        // "turbo.daemon.stop".to_string(),
                    ],
                    work_done_progress_options: Default::default(),
                }),
                workspace: Some(WorkspaceServerCapabilities {
                    workspace_folders: Some(WorkspaceFoldersServerCapabilities {
                        supported: Some(true),
                        change_notifications: Some(OneOf::Left(true)),
                    }),
                    file_operations: None,
                }),
                code_lens_provider: Some(CodeLensOptions {
                    resolve_provider: None,
                }),
                code_action_provider: Some(CodeActionProviderCapability::Options(
                    CodeActionOptions {
                        code_action_kinds: Some(vec![CodeActionKind::QUICKFIX]),
                        resolve_provider: None,
                        work_done_progress_options: WorkDoneProgressOptions {
                            work_done_progress: None,
                        },
                    },
                )),
                references_provider: Some(OneOf::Right(ReferencesOptions {
                    work_done_progress_options: WorkDoneProgressOptions {
                        work_done_progress: None,
                    },
                })),
                ..ServerCapabilities::default()
            },
        })
    }

    /// Find which projects / scripts are affected by a given pipeline item
    async fn references(&self, params: ReferenceParams) -> LspResult<Option<Vec<Location>>> {
        self.client
            .log_message(MessageType::INFO, "references!")
            .await;

        let Some(referenced_task) = ({
            let rope = {
                let map = lock_or_recover(&self.files);
                match map.get(&params.text_document_position.text_document.uri) {
                    Some(files) => files,
                    None => return Ok(None),
                }
                .to_owned() // cloning is cheap
            };

            let text = rope.chunks().join("");
            let parse = jsonc_parser::parse_to_ast(
                &text,
                &CollectOptions {
                    comments: true,
                    tokens: true,
                },
                &Default::default(),
            );

            // iterate pipeline items, and see if any of their ranges intersect
            // with the reference request's position
            let parse = match parse {
                Ok(parse) => parse,
                Err(_err) => {
                    // if it is not a valid json, then there are no references
                    return Ok(None);
                }
            };

            parse
                .value
                .as_ref()
                .and_then(|v| v.as_object())
                .and_then(|o| o.get_object("tasks"))
                .map(|p| p.properties.iter())
                .into_iter()
                .flatten()
                .find_map(|task| {
                    let mut range = task.range;
                    range.start += 1; // account for quote
                    let key_range = range.start + task.name.as_str().len();
                    range.end = key_range;

                    // convert ast range to lsp range
                    let lsp_range = convert_ranges(&rope, range);

                    if lsp_range.start < params.text_document_position.position
                        && lsp_range.end > params.text_document_position.position
                    {
                        Some(task.name.as_str().to_string())
                    } else {
                        None
                    }
                })
        }) else {
            // no overlap with any task definitions, exit
            return Ok(None);
        };

        self.client
            .log_message(
                MessageType::INFO,
                format!("finding references for {referenced_task:?}"),
            )
            .await;

        let packages = match self.package_discovery().await {
            Ok(packages) => packages,
            Err(e) => {
                self.client
                    .log_message(MessageType::WARNING, e.to_string())
                    .await;

                // there aren't really any other errors we can return here, other than
                // an internal error
                let mut error = Error::internal_error();
                error.message = "failed to get package list from the daemon".into();
                return Err(error);
            }
        };

        Ok(Some(packages.references(&referenced_task)))
    }

    /// Add code lens items for running a particular task in the turbo.json
    async fn code_lens(&self, params: CodeLensParams) -> LspResult<Option<Vec<CodeLens>>> {
        self.client
            .log_message(MessageType::INFO, "code lens!")
            .await;

        let rope = {
            let map = lock_or_recover(&self.files);
            match map.get(&params.text_document.uri) {
                Some(files) => files,
                None => return Ok(None),
            }
            .to_owned() // cloning is cheap
        };

        let text = rope.chunks().join("");
        let parse = jsonc_parser::parse_to_ast(
            &text,
            &CollectOptions {
                comments: true,
                tokens: true,
            },
            &Default::default(),
        );

        let parse = match parse {
            Ok(parse) => parse,
            Err(_err) => {
                // todo: do we error here?
                // self.client
                //     .log_message(MessageType::ERROR, format!("{:?}", err))
                //     .await;
                return Ok(None);
            }
        };

        let pipeline = parse
            .value
            .as_ref()
            .and_then(|v| v.as_object())
            .and_then(|o| o.get_object("tasks"))
            .map(|p| p.properties.iter())
            .into_iter()
            .flatten();

        let mut tasks = vec![];
        for task in pipeline {
            let mut range = task.range;
            range.start += 1; // account for quote
            let key_range = range.start + task.name.as_str().len();
            range.end = key_range;

            tasks.push(CodeLens {
                command: Some(Command {
                    title: format!("Run {}", task.name.as_str()),
                    command: "turbo.run".to_string(),
                    arguments: Some(vec![Value::String(task.name.as_str().to_string())]),
                }),
                range: convert_ranges(&rope, range),
                data: None,
            });
        }

        Ok(Some(tasks))
    }

    /// Given a list of diagnistics that we previously reported, produce code
    /// actions that the user can run
    async fn code_action(&self, params: CodeActionParams) -> LspResult<Option<CodeActionResponse>> {
        self.client
            .log_message(MessageType::INFO, format!("{params:#?}"))
            .await;

        let mut code_actions = vec![];

        for diag in params.context.diagnostics {
            match &diag.code {
                Some(NumberOrString::String(s)) if s == "deprecated:env-var" => {
                    code_actions.push(CodeActionOrCommand::CodeAction(CodeAction {
                        title: "Apply codemod".to_string(),
                        command: Some(Command {
                            title: "Apply codemod".to_string(),
                            command: "turbo.codemod".to_string(),
                            arguments: Some(vec![Value::String(
                                "migrate-env-var-dependencies".to_string(),
                            )]),
                        }),
                        diagnostics: Some(vec![diag]),
                        kind: Some(CodeActionKind::QUICKFIX),
                        is_preferred: Some(true),
                        ..Default::default()
                    }))
                }
                _ => {}
            }
        }

        Ok(Some(code_actions))
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "initialized!")
            .await;
    }

    async fn shutdown(&self) -> LspResult<()> {
        Ok(())
    }

    async fn did_change_workspace_folders(&self, _: DidChangeWorkspaceFoldersParams) {
        self.packages.invalidate();
        self.client
            .log_message(MessageType::INFO, "workspace folders changed!")
            .await;
    }

    async fn did_change_configuration(&self, _: DidChangeConfigurationParams) {
        self.packages.invalidate();
        self.client
            .log_message(MessageType::INFO, "configuration changed!")
            .await;
    }

    async fn did_change_watched_files(&self, _: DidChangeWatchedFilesParams) {
        self.packages.invalidate();
        self.client
            .log_message(MessageType::INFO, "watched files have changed!")
            .await;
    }

    async fn execute_command(&self, _: ExecuteCommandParams) -> LspResult<Option<Value>> {
        self.client
            .log_message(MessageType::INFO, "command executed!")
            .await;

        match self.client.apply_edit(WorkspaceEdit::default()).await {
            Ok(res) if res.applied => self.client.log_message(MessageType::INFO, "applied").await,
            Ok(_) => self.client.log_message(MessageType::INFO, "rejected").await,
            Err(err) => self.client.log_message(MessageType::ERROR, err).await,
        }

        Ok(None)
    }

    /// Add an entry to the list of ropes for a newly opened file
    async fn did_open(&self, document: DidOpenTextDocumentParams) {
        self.client
            .log_message(
                MessageType::INFO,
                format!("file opened: {}", document.text_document.uri),
            )
            .await;

        let rope = crop::Rope::from(document.text_document.text);

        {
            let mut map = lock_or_recover(&self.files);
            map.insert(document.text_document.uri.clone(), rope.clone());
        }

        self.handle_file_update(document.text_document.uri, Some(rope), None)
            .await;
    }

    /// Modify the rope that we have in memory to reflect the changes that were
    /// made to the buffer in the editor
    async fn did_change(&self, document: DidChangeTextDocumentParams) {
        self.client
            .log_message(
                MessageType::INFO,
                format!("file changed: {}", document.text_document.uri),
            )
            .await;

        let updated_rope = {
            let mut map = lock_or_recover(&self.files);
            let rope = map.entry(document.text_document.uri.clone()).or_default();

            for change in document.content_changes {
                match change.range {
                    Some(range) => {
                        let start_offset = rope.byte_of_line(range.start.line as usize)
                            + range.start.character as usize;
                        let end_offset = rope.byte_of_line(range.end.line as usize)
                            + range.end.character as usize;

                        rope.replace(start_offset..end_offset, change.text);
                    }
                    None => *rope = crop::Rope::from(change.text),
                }
            }

            rope.clone()
        };

        self.handle_file_update(
            document.text_document.uri,
            Some(updated_rope),
            Some(document.text_document.version),
        )
        .await;
    }

    async fn did_save(&self, document: DidSaveTextDocumentParams) {
        if document.text_document.uri.path().ends_with("/package.json") {
            self.packages.invalidate();
        }
        self.client
            .log_message(
                MessageType::INFO,
                format!("file saved: {}", document.text_document.uri),
            )
            .await;
    }

    async fn did_close(&self, document: DidCloseTextDocumentParams) {
        self.client
            .log_message(
                MessageType::INFO,
                format!("file closed: {}", document.text_document.uri),
            )
            .await;
    }

    /// Provide intellisense completions for package / task names
    ///
    /// - get all packages
    /// - get all package jsons
    /// - produce all unique script names
    /// - flatmap producing all package#script combos
    /// - chain them
    async fn completion(&self, _: CompletionParams) -> LspResult<Option<CompletionResponse>> {
        let packages = self
            .package_discovery()
            .await
            .map_err(|_e| Error::internal_error())?;

        let items = packages
            .completion_labels()
            .into_iter()
            .map(|label| CompletionItem {
                label,
                kind: Some(CompletionItemKind::FIELD),
                ..Default::default()
            })
            .collect();

        Ok(Some(CompletionResponse::Array(items)))
    }
}

impl Backend {
    pub fn new(client: Client) -> Self {
        let (rx, tx) = tokio::sync::watch::channel(None);

        Self {
            client,
            repo_root: Arc::new(Mutex::new(None)),
            files: Mutex::new(HashMap::new()),
            initializer: rx,
            daemon: tx,
            packages: LspPackageCache::default(),

            pidlock: Mutex::new(None),
        }
    }

    #[allow(clippy::result_large_err)]
    async fn package_discovery(&self) -> Result<Arc<LspPackages>, package_graph::Error> {
        if let Some(packages) = self.packages.get() {
            return Ok(packages);
        }
        let repo_root =
            lock_or_recover(&self.repo_root)
                .clone()
                .ok_or(package_graph::Error::Discovery(
                    discovery::Error::Unavailable,
                ))?;
        let root_manifest_path = repo_root.join_component("package.json");

        // LSP has no Cargo package features today. Still construct the same
        // repository view so a pure Cargo repository cannot gain a synthetic
        // JavaScript manifest or root script scope.
        if !root_manifest_path.exists() {
            let graph = PackageGraph::builder_optional(&repo_root, None)
                .build()
                .await?;
            let packages = Arc::new(LspPackages::from_graph(&graph, HashMap::new()));
            self.packages.set(packages.clone());
            return Ok(packages);
        }

        let daemon = {
            let mut daemon = self.daemon.clone();
            let daemon = daemon
                .wait_for(|d| d.is_some())
                .await
                .map_err(|_| package_graph::Error::Discovery(discovery::Error::Unavailable))?;
            let Some(daemon) = daemon.as_ref() else {
                return Err(package_graph::Error::Discovery(
                    discovery::Error::Unavailable,
                ));
            };
            daemon.clone()
        };

        let response = DaemonPackageDiscovery::new(daemon, repo_root.clone())
            .discover_packages_blocking()
            .await?;

        // Parse each manifest once. The parsed map seeds graph construction;
        // the parallel source map is retained only for scripts and ranges.
        let mut package_jsons = HashMap::with_capacity(response.workspaces.len());
        let mut sources = HashMap::with_capacity(response.workspaces.len() + 1);
        for workspace in response.workspaces {
            let (package_json_path, _) = workspace.into_paths();
            let Ok(text) = std::fs::read_to_string(&package_json_path) else {
                continue;
            };
            let Ok(package_json) = PackageJson::load_from_str(&text, package_json_path.as_str())
            else {
                continue;
            };
            package_jsons.insert(package_json_path.clone(), package_json.clone());
            sources.insert(package_json_path, PackageSource { text, package_json });
        }
        retain_unique_named(&mut package_jsons);

        let root_text = std::fs::read_to_string(&root_manifest_path).ok();
        let parsed_root = root_text
            .as_deref()
            .and_then(|text| PackageJson::load_from_str(text, root_manifest_path.as_str()).ok());
        let root_package_json = parsed_root.clone().unwrap_or_default();
        if let (Some(text), Some(package_json)) = (root_text, parsed_root) {
            sources.insert(
                root_manifest_path.clone(),
                PackageSource { text, package_json },
            );
        }

        let graph = PackageGraph::builder(&repo_root, root_package_json)
            .with_package_manager(response.package_manager)
            .with_package_jsons(Some(package_jsons))
            .build()
            .await;

        // Discovery succeeded, so repository validation errors should not
        // disable every LSP script feature. Sources that cannot be joined to
        // authoritative knowledge remain deliberately unscoped.
        let packages = Arc::new(LspPackages::from_graph_or_unscoped(
            graph,
            sources,
            &root_manifest_path,
        ));
        self.packages.set(packages.clone());
        Ok(packages)
    }

    /// Handle a file update to a rope, emitting diagnostics if necessary.
    async fn handle_file_update(&self, uri: Url, rope: Option<crop::Rope>, version: Option<i32>) {
        let rope = match rope {
            Some(rope) => rope,
            None => match lock_or_recover(&self.files).get(&uri) {
                Some(files) => files,
                None => return,
            }
            .clone(),
        };

        let contents = rope.chunks().join("");

        let packages = self.package_discovery().await;
        let tasks = packages.map(|packages| packages.task_index());

        // we still want to emit diagnostics if we can't infer tasks
        let tasks_and_packages = tasks.as_ref().map(|tasks| {
            (
                tasks,
                tasks
                    .values()
                    .flatten()
                    .flatten()
                    .map(|s| s.as_str())
                    .unique()
                    .collect::<HashSet<_>>(),
            )
        });

        let mut diagnostics = vec![];

        // ParseResult cannot be sent across threads, so we must ensure it is dropped
        // before we send the diagnostics. easiest way is just to scope it
        {
            let parse =
                jsonc_parser::parse_to_ast(&contents, &Default::default(), &Default::default());

            let parse = match parse {
                Ok(parse) => parse,
                Err(_) => return, // if it is not a valid json, then there are no diagnostics
            };

            let object = parse.value.as_ref().and_then(|v| v.as_object());

            let mut globs = vec![];

            globs.extend(
                object
                    .and_then(|o| o.get_array("globalDependencies"))
                    .map(|d| &d.elements)
                    .into_iter()
                    .flatten(),
            );

            let tasks_object = object.and_then(|o| o.get_object("tasks"));

            let pipeline = tasks_object.map(|p| p.properties.iter());

            let transit_nodes = tasks_object
                .map(collect_transit_node_tasks)
                .unwrap_or_default();

            for property in pipeline.into_iter().flatten() {
                let mut object_range = property.range;
                object_range.start += 1; // account for quote
                let object_key_range = object_range.start + property.name.as_str().len();
                object_range.end = object_key_range;

                if let (Ok((tasks, packages)), ObjectPropName::String(name)) =
                    (&tasks_and_packages, &property.name)
                {
                    report_invalid_packages_and_tasks(
                        tasks,
                        &transit_nodes,
                        packages,
                        &rope,
                        &mut diagnostics,
                        name,
                    );
                }

                // inputs, outputs
                globs.extend(
                    ["inputs", "outputs"]
                        .iter()
                        .filter_map(|s| {
                            property
                                .value
                                .as_object()
                                .and_then(|o| o.get_array(s))
                                .map(|a| &a.elements)
                        })
                        .flatten(),
                );

                // dependsOn
                if let Some(array) = property
                    .value
                    .as_object()
                    .and_then(|o| o.get_array("dependsOn"))
                {
                    for depends_on in &array.elements {
                        if let Some(string) = depends_on.as_string_lit().cloned() {
                            if is_turbo_extends_sentinel(&string) {
                                continue;
                            }

                            let suffix = if let Some(suffix) = strip_lit_prefix(&string, "^") {
                                diagnostics.push(Diagnostic {
                                    message: format!(
                                        "The '^' means \"run the `{}` task in the package's \
                                         dependencies before this one\"",
                                        suffix.value,
                                    ),
                                    range: convert_ranges(
                                        &rope,
                                        collapse_string_range(string.range),
                                    ),
                                    severity: Some(DiagnosticSeverity::HINT),
                                    ..Default::default()
                                });
                                suffix
                            } else {
                                // prevent task from depending on itself, if it is not a '^' task
                                if string.value == property.name.as_str() {
                                    diagnostics.push(Diagnostic {
                                        message: "A task cannot depend on itself.".to_string(),
                                        range: convert_ranges(&rope, string.range),
                                        severity: Some(DiagnosticSeverity::ERROR),
                                        code: Some(NumberOrString::String(
                                            "turbo:self-dependency".to_string(),
                                        )),
                                        ..Default::default()
                                    });
                                    continue;
                                }

                                string
                            };

                            let suffix = if let Some(suffix) = strip_lit_prefix(&suffix, "$") {
                                diagnostics.push(Diagnostic {
                                    message: "The $ syntax is deprecated. Please apply the \
                                              codemod."
                                        .to_string(),
                                    range: convert_ranges(
                                        &rope,
                                        collapse_string_range(suffix.range),
                                    ),
                                    severity: Some(DiagnosticSeverity::ERROR),
                                    code: Some(NumberOrString::String(
                                        "deprecated:env-var".to_string(),
                                    )),
                                    ..Default::default()
                                });
                                suffix
                            } else {
                                suffix
                            };

                            if let Ok((tasks, packages)) = &tasks_and_packages {
                                report_invalid_packages_and_tasks(
                                    tasks,
                                    &transit_nodes,
                                    packages,
                                    &rope,
                                    &mut diagnostics,
                                    &suffix,
                                );
                            }
                        }
                    }
                }
            }

            for glob in globs {
                // read string and parse glob
                if let Some(string) = glob.as_string_lit() {
                    let expression = string.value.strip_prefix('!').unwrap_or(&string.value); // strip the negation
                    if let Err(glob) = wax::Glob::new(expression) {
                        diagnostics.push(Diagnostic {
                            message: format!("Invalid glob: {glob}"),
                            range: convert_ranges(&rope, collapse_string_range(string.range)),
                            severity: Some(DiagnosticSeverity::ERROR),
                            ..Default::default()
                        });
                    }
                }
            }
        }

        self.client
            .publish_diagnostics(uri, diagnostics, version)
            .await;
    }
}

fn lock_or_recover<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn convert_ranges(rope: &crop::Rope, range: jsonc_parser::common::Range) -> Range {
    let start_line = rope.line_of_byte(range.start);
    let end_line = rope.line_of_byte(range.end);

    Range {
        start: Position {
            line: start_line as u32,
            character: (range.start - rope.byte_of_line(start_line)) as u32,
        },
        end: Position {
            line: end_line as u32,
            character: (range.end - rope.byte_of_line(end_line)) as u32,
        },
    }
}

fn strip_lit_prefix<'a>(s: &'a StringLit<'a>, prefix: &str) -> Option<StringLit<'a>> {
    s.value
        .strip_prefix(prefix)
        .map(Cow::Borrowed)
        .map(|stripped| StringLit {
            value: stripped,
            range: jsonc_parser::common::Range {
                start: s.range.start + prefix.len(),
                end: s.range.end,
            },
        })
}

fn is_turbo_extends_sentinel(string: &StringLit<'_>) -> bool {
    string.value == TURBO_EXTENDS
}

fn collect_transit_node_tasks(tasks: &jsonc_parser::ast::Object<'_>) -> HashSet<String> {
    tasks
        .properties
        .iter()
        .filter_map(|property| {
            let task = property.name.as_str();
            let dependency = format!("^{task}");
            let depends_on = property.value.as_object()?.get_array("dependsOn")?;

            depends_on
                .elements
                .iter()
                .filter_map(|value| value.as_string_lit())
                .any(|value| value.value == dependency)
                .then(|| task.to_string())
        })
        .collect()
}

/// remove quotes from a string range
fn collapse_string_range(range: jsonc_parser::common::Range) -> jsonc_parser::common::Range {
    jsonc_parser::common::Range {
        start: range.start + 1,
        end: range.end - 1,
    }
}

fn report_invalid_packages_and_tasks(
    tasks: &HashMap<String, Vec<Option<String>>>,
    transit_nodes: &HashSet<String>,
    packages: &HashSet<&str>,
    rope: &crop::Rope,
    diagnostics: &mut Vec<Diagnostic>,
    package_task: &StringLit,
) {
    let (package, task) = package_task
        .value
        .split_once('#') // turbo packages may not have # in them
        .map(|(p, t)| (Some(p), t))
        .unwrap_or((None, &package_task.value));

    let range = convert_ranges(rope, collapse_string_range(package_task.range));

    match (tasks.get(task), package) {
        // we specified a package, but that package doesn't exist
        (_, Some(package)) if !packages.contains(&package) => {
            diagnostics.push(Diagnostic {
                message: format!("The package `{package}` does not exist in {packages:?}"),
                range,
                severity: Some(DiagnosticSeverity::ERROR),
                code: Some(NumberOrString::String("turbo:no-such-package".to_string())),
                ..Default::default()
            });
        }
        // that task exists, and we have a package defined, but the task
        // doesn't exist in that
        // package
        (Some(list), Some(package))
            if !list
                .iter()
                .filter_map(|s| s.as_ref().map(|s| s.as_str()))
                .contains(&package) =>
        {
            diagnostics.push(Diagnostic {
                message: format!("The task `{task}` does not exist in the package `{package}`."),
                range,
                severity: Some(DiagnosticSeverity::ERROR),
                code: Some(NumberOrString::String(
                    "turbo:no-such-task-in-package".to_string(),
                )),
                ..Default::default()
            });
        }
        // transit nodes are configured tasks that intentionally do not have a
        // matching package script.
        (None, None) if transit_nodes.contains(task) => {}
        // the task doesn't exist anywhere, so we have a problem
        (None, None) => {
            diagnostics.push(Diagnostic {
                message: format!("The task `{task}` does not exist."),
                range,
                severity: Some(DiagnosticSeverity::ERROR),
                code: Some(NumberOrString::String("turbo:no-such-task".to_string())),
                ..Default::default()
            });
        }
        // we have specified a package, but the task doesn't exist at
        // all
        (None, Some(package)) => {
            diagnostics.push(Diagnostic {
                message: format!("The task `{task}` does not exist in the package `{package}`."),
                range,
                severity: Some(DiagnosticSeverity::ERROR),
                code: Some(NumberOrString::String("turbo:no-such-task".to_string())),
                ..Default::default()
            });
        }
        // task exists in a given package, so we're good
        (Some(_), Some(_)) => {}
        // the task exists and we haven't specified a package, so we're
        // good
        (Some(_), None) => {}
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use std::{
        borrow::Cow,
        collections::{HashMap, HashSet},
        sync::Arc,
    };

    use jsonc_parser::{ast::StringLit, common::Range};
    use serde_json::json;
    use tower_lsp::lsp_types::{Diagnostic, NumberOrString};
    use turbopath::AbsoluteSystemPathBuf;
    use turborepo_repository::{
        package_graph::{PackageGraph, PackageName},
        package_json::PackageJson,
        package_manager::PackageManager,
    };

    use super::{
        LspPackageCache, LspPackages, PackageSource, collect_transit_node_tasks,
        is_turbo_extends_sentinel, report_invalid_packages_and_tasks, retain_unique_named,
    };

    struct TestRepository {
        _temp_dir: tempfile::TempDir,
        root: AbsoluteSystemPathBuf,
    }

    impl TestRepository {
        fn new() -> Self {
            let temp_dir = tempfile::tempdir().expect("temp directory");
            let root =
                AbsoluteSystemPathBuf::new(temp_dir.path().to_str().expect("UTF-8 temp directory"))
                    .expect("absolute temp directory");
            Self {
                _temp_dir: temp_dir,
                root,
            }
        }

        async fn lsp_packages(
            &self,
            root_source: &str,
            workspaces: &[(&str, &str)],
        ) -> LspPackages {
            let root_path = self.root.join_component("package.json");
            let root_package_json = parse_package_json(root_source, &root_path);
            let mut package_jsons = HashMap::new();
            let mut sources = HashMap::from([(
                root_path,
                PackageSource {
                    text: root_source.to_string(),
                    package_json: root_package_json.clone(),
                },
            )]);

            for (relative_path, source) in workspaces {
                let components = relative_path.split('/').collect::<Vec<_>>();
                let path = self.root.join_components(&components);
                let package_json = parse_package_json(source, &path);
                package_jsons.insert(path.clone(), package_json.clone());
                sources.insert(
                    path,
                    PackageSource {
                        text: source.to_string(),
                        package_json,
                    },
                );
            }
            retain_unique_named(&mut package_jsons);

            let graph = PackageGraph::builder(&self.root, root_package_json)
                .with_package_manager(PackageManager::Npm)
                .with_package_jsons(Some(package_jsons))
                .build()
                .await
                .expect("package graph");
            LspPackages::from_graph(&graph, sources)
        }
    }

    fn parse_package_json(source: &str, path: &AbsoluteSystemPathBuf) -> PackageJson {
        PackageJson::load_from_str(source, path.as_str()).expect("valid package.json")
    }

    fn string_lit(value: &'static str) -> StringLit<'static> {
        StringLit {
            value: Cow::Borrowed(value),
            range: Range {
                start: 0,
                end: value.len() + 2,
            },
        }
    }

    #[test]
    fn detects_turbo_extends_sentinel() {
        assert!(is_turbo_extends_sentinel(&string_lit("$TURBO_EXTENDS$")));
    }

    #[test]
    fn does_not_treat_other_dollar_syntax_as_turbo_extends_sentinel() {
        assert!(!is_turbo_extends_sentinel(&string_lit("$FOO")));
        assert!(!is_turbo_extends_sentinel(&string_lit("^$TURBO_EXTENDS$")));
    }

    #[test]
    fn detects_transit_node_tasks() {
        let parsed = jsonc_parser::parse_to_ast(
            r#"{
              "tasks": {
                "topology": { "dependsOn": ["^topology"] },
                "build": { "dependsOn": ["^topology"] }
              }
            }"#,
            &Default::default(),
            &Default::default(),
        )
        .expect("valid json");

        let tasks = parsed
            .value
            .as_ref()
            .and_then(|value| value.as_object())
            .and_then(|object| object.get_object("tasks"))
            .expect("tasks object");

        let transit_nodes = collect_transit_node_tasks(tasks);

        assert!(transit_nodes.contains("topology"));
        assert!(!transit_nodes.contains("build"));
    }

    #[test]
    fn does_not_report_missing_task_for_transit_node() {
        let rope = crop::Rope::from(r#""topology""#);
        let mut diagnostics = Vec::new();
        let transit_nodes = HashSet::from(["topology".to_string()]);
        let tasks = HashMap::new();
        let packages = Default::default();

        report_invalid_packages_and_tasks(
            &tasks,
            &transit_nodes,
            &packages,
            &rope,
            &mut diagnostics,
            &string_lit("topology"),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_missing_task_when_not_a_transit_node() {
        let rope = crop::Rope::from(r#""topology""#);
        let mut diagnostics = Vec::new();

        report_invalid_packages_and_tasks(
            &HashMap::new(),
            &Default::default(),
            &Default::default(),
            &rope,
            &mut diagnostics,
            &string_lit("topology"),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostic_code(&diagnostics[0]), Some("turbo:no-such-task"));
    }

    #[tokio::test]
    async fn root_scope_uses_graph_identity_when_named_or_unnamed() {
        for root_source in [
            r#"{"name":"repository","scripts":{"build":"named"}}"#,
            r#"{"scripts":{"build":"unnamed"}}"#,
        ] {
            let repository = TestRepository::new();
            let packages = repository.lsp_packages(root_source, &[]).await;

            assert_eq!(packages.packages.len(), 1);
            assert_eq!(packages.packages[0].identity, Some(PackageName::Root));
            assert!(
                packages
                    .completion_labels()
                    .contains(&"//#build".to_string())
            );
        }
    }

    #[tokio::test]
    async fn nested_package_definition_path_is_graph_backed() {
        let repository = TestRepository::new();
        let packages = repository
            .lsp_packages(
                r#"{"scripts":{}}"#,
                &[(
                    "apps/nested/web/package.json",
                    r#"{"name":"web","scripts":{"dev":"next dev"}}"#,
                )],
            )
            .await;
        let web = packages
            .packages
            .iter()
            .find(|package| {
                package
                    .identity
                    .as_ref()
                    .is_some_and(|identity| identity.as_str() == "web")
            })
            .expect("web package");

        assert_eq!(
            web.source_path,
            repository
                .root
                .join_components(&["apps", "nested", "web", "package.json"])
        );
    }

    #[tokio::test]
    async fn stale_source_name_cannot_change_identity_or_reference_path() {
        let repository = TestRepository::new();
        let definition_path = repository
            .root
            .join_components(&["packages", "app", "package.json"]);
        let authoritative = PackageJson::from_value(json!({
            "name": "authoritative",
            "scripts": { "stale-task": "old command" }
        }))
        .expect("package json");
        let graph = PackageGraph::builder(
            &repository.root,
            PackageJson::from_value(json!({})).expect("root package json"),
        )
        .with_package_manager(PackageManager::Npm)
        .with_package_jsons(Some(HashMap::from([(
            definition_path.clone(),
            authoritative,
        )])))
        .build()
        .await
        .expect("package graph");
        // Identity and definition path belong to graph knowledge, while the
        // current source payload owns scripts until native task APIs exist.
        let stale_source = r#"{"name":"stale","scripts":{"source-task":"new command"}}"#;
        let packages = LspPackages::from_graph(
            &graph,
            HashMap::from([(
                definition_path.clone(),
                PackageSource {
                    text: stale_source.to_string(),
                    package_json: parse_package_json(stale_source, &definition_path),
                },
            )]),
        );

        assert!(
            packages
                .completion_labels()
                .contains(&"authoritative#source-task".to_string())
        );
        assert!(
            !packages
                .completion_labels()
                .contains(&"stale#source-task".to_string())
        );
        assert!(
            !packages
                .completion_labels()
                .contains(&"authoritative#stale-task".to_string())
        );
        let references = packages.references("authoritative#source-task");
        assert_eq!(references.len(), 1);
        assert_eq!(
            references[0].uri.to_file_path().ok(),
            Some(definition_path.into())
        );
        assert!(packages.references("stale#source-task").is_empty());
    }

    #[tokio::test]
    async fn unnamed_workspace_is_unscoped_compatibility_payload() {
        let repository = TestRepository::new();
        let packages = repository
            .lsp_packages(
                r#"{"scripts":{}}"#,
                &[(
                    "packages/unnamed/package.json",
                    r#"{"scripts":{"build":"build unnamed"}}"#,
                )],
            )
            .await;
        let unnamed = packages
            .packages
            .iter()
            .find(|package| package.identity.is_none())
            .expect("unscoped package source");

        assert!(
            unnamed
                .source_path
                .ends_with("packages/unnamed/package.json")
        );
        assert_eq!(packages.completion_labels(), vec!["build"]);
        assert_eq!(packages.references("build").len(), 1);
        assert!(packages.references("#build").is_empty());
        assert_eq!(packages.task_index().get("build"), Some(&vec![None]));
    }

    #[tokio::test]
    async fn duplicate_names_remain_unscoped_without_hiding_valid_packages() {
        let repository = TestRepository::new();
        let packages = repository
            .lsp_packages(
                r#"{"scripts":{"root":"root"}}"#,
                &[
                    (
                        "unique/package.json",
                        r#"{"name":"unique","scripts":{"unique":"unique"}}"#,
                    ),
                    (
                        "first/package.json",
                        r#"{"name":"duplicate","scripts":{"shared":"first"}}"#,
                    ),
                    (
                        "second/package.json",
                        r#"{"name":"duplicate","scripts":{"shared":"second"}}"#,
                    ),
                ],
            )
            .await;

        let labels = packages.completion_labels();
        assert!(labels.contains(&"//#root".to_string()));
        assert!(labels.contains(&"unique#unique".to_string()));
        assert!(!labels.iter().any(|label| label == "duplicate#shared"));
        assert_eq!(packages.references("shared").len(), 2);
        assert_eq!(packages.task_index().get("shared"), Some(&vec![None]));
    }

    #[test]
    fn package_cache_survives_changes_until_save_invalidation() {
        let cache = LspPackageCache::default();
        let packages = Arc::new(LspPackages::default());
        cache.set(packages.clone());
        assert!(Arc::ptr_eq(
            &cache.get().expect("cached packages"),
            &packages
        ));
        assert!(Arc::ptr_eq(
            &cache.get().expect("cache survives another document change"),
            &packages
        ));
        cache.invalidate();
        assert!(cache.get().is_none());
    }

    #[test]
    fn source_insertion_order_does_not_affect_lsp_results() {
        let repository = TestRepository::new();
        let first_path = repository.root.join_component("a-package.json");
        let second_path = repository.root.join_component("b-package.json");
        let first_source = r#"{"scripts":{"build":"first","alpha":"alpha"}}"#;
        let second_source = r#"{"scripts":{"build":"second","beta":"beta"}}"#;
        let source = |text: &str, path: &AbsoluteSystemPathBuf| PackageSource {
            text: text.to_string(),
            package_json: parse_package_json(text, path),
        };
        let forward = LspPackages::unscoped(HashMap::from([
            (first_path.clone(), source(first_source, &first_path)),
            (second_path.clone(), source(second_source, &second_path)),
        ]));
        let reverse = LspPackages::unscoped(HashMap::from([
            (second_path.clone(), source(second_source, &second_path)),
            (first_path.clone(), source(first_source, &first_path)),
        ]));

        assert_eq!(forward.completion_labels(), reverse.completion_labels());
        assert_eq!(forward.task_index(), reverse.task_index());
        assert_eq!(
            forward
                .references("build")
                .into_iter()
                .map(|reference| reference.uri)
                .collect::<Vec<_>>(),
            reverse
                .references("build")
                .into_iter()
                .map(|reference| reference.uri)
                .collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    async fn completion_references_and_file_update_index_include_root_and_packages() {
        let repository = TestRepository::new();
        let packages = repository
            .lsp_packages(
                r#"{"name":"ignored-root-name","scripts":{"lint":"root lint"}}"#,
                &[(
                    "packages/ui/package.json",
                    r#"{"name":"@repo/ui","scripts":{"lint":"eslint","test":"vitest"}}"#,
                )],
            )
            .await;

        let labels = packages.completion_labels();
        assert!(labels.contains(&"//#lint".to_string()));
        assert!(labels.contains(&"@repo/ui#test".to_string()));
        assert_eq!(labels.iter().filter(|label| *label == "lint").count(), 1);
        assert_eq!(packages.references("lint").len(), 2);
        assert_eq!(packages.references("//#lint").len(), 1);

        let tasks = packages.task_index();
        assert_eq!(
            tasks.get("lint"),
            Some(&vec![Some("//".to_string()), Some("@repo/ui".to_string())])
        );
        assert_eq!(tasks.get("test"), Some(&vec![Some("@repo/ui".to_string())]));
    }

    #[tokio::test]
    async fn pure_cargo_graph_does_not_invent_javascript_packages() {
        let repository = TestRepository::new();
        let graph = PackageGraph::builder_optional(&repository.root, None)
            .build()
            .await
            .expect("pure Cargo graph");
        let packages = LspPackages::from_graph(&graph, HashMap::new());

        assert!(packages.packages.is_empty());
        assert!(packages.completion_labels().is_empty());
        assert!(packages.task_index().is_empty());
    }

    fn diagnostic_code(diagnostic: &Diagnostic) -> Option<&str> {
        match diagnostic.code.as_ref()? {
            NumberOrString::String(code) => Some(code.as_str()),
            NumberOrString::Number(_) => None,
        }
    }
}
