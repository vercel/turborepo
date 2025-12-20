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
    iter,
    sync::{Arc, Mutex},
};

use itertools::{Itertools, chain};
use jsonc_parser::{
    CollectOptions,
    ast::{ObjectPropName, StringLit},
};
use serde_json::Value;
use tokio::sync::watch::{Receiver, Sender};
use tower_lsp::{
    Client, LanguageServer,
    jsonrpc::{Error, Result as LspResult},
    lsp_types::*,
};
use turbopath::AbsoluteSystemPathBuf;
use turborepo_lib::{
    DaemonClient, DaemonConnector, DaemonConnectorError, DaemonError, DaemonPackageDiscovery,
    DaemonPaths,
};
use turborepo_repository::{
    discovery::{self, DiscoveryResponse, PackageDiscovery, WorkspaceData},
    package_json::PackageJson,
};

pub struct Backend {
    client: Client,
    repo_root: Arc<Mutex<Option<AbsoluteSystemPathBuf>>>,
    files: Mutex<HashMap<Url, crop::Rope>>,
    initializer: Sender<Option<DaemonClient<DaemonConnector>>>,
    daemon: Receiver<Option<DaemonClient<DaemonConnector>>>,

    // this is only used for turbo optimize
    pidlock: Mutex<Option<pidlock::Pidlock>>,
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
            let repo_root =
                AbsoluteSystemPathBuf::new(repo_root).expect("file is always an absolute path");

            self.repo_root
                .lock()
                .expect("only fails if poisoned")
                .replace(repo_root.clone());

            let paths = DaemonPaths::from_repo_root(&repo_root);

            let (_, daemon) = tokio::join!(
                self.client
                    .log_message(MessageType::INFO, format!("root uri: {}", paths.sock_file),),
                tokio_retry::Retry::spawn(
                    tokio_retry::strategy::FixedInterval::from_millis(100).take(5),
                    || {
                        let can_start_server = true;
                        let can_kill_server = false;
                        let connector = DaemonConnector::new(
                            can_start_server,
                            can_kill_server,
                            &repo_root,
                            None,
                        );
                        connector.connect()
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
                        .log_message(
                            MessageType::ERROR,
                            format!("failed to connect to daemon: {e}"),
                        )
                        .await;
                    return Err(Error::internal_error());
                }
            };

            self.initializer
                .send(Some(daemon))
                .expect("there is a receiver");

            let mut lock = pidlock::Pidlock::new(paths.lsp_pid_file.as_std_path().to_owned());

            match lock.acquire() {
                Ok(()) => {
                    *self.pidlock.lock().expect("only fails if poisoned") = Some(lock);
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
                        .log_message(
                            MessageType::ERROR,
                            format!("Failed to acquire pidlock: {e}"),
                        )
                        .await;
                    return Err(Error::internal_error());
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
                let map = self.files.lock().expect("only fails if poisoned");
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

        let repo_root = self
            .repo_root
            .lock()
            .expect("only fails if poisoned")
            .clone();

        let repo_root = match repo_root {
            Some(repo_root) => repo_root,
            None => {
                self.client
                    .log_message(MessageType::INFO, "received request before initialization")
                    .await;
                return Ok(None);
            }
        };

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

        let mut locations = vec![];
        for wd in packages.workspaces {
            let data = match std::fs::read_to_string(&wd.package_json) {
                Ok(data) => data,
                // if we can't read a package.json, then we can't set up references to it
                // so we just skip it and do a best effort
                Err(_) => continue,
            };
            let package_json = match PackageJson::load_from_str(&data, wd.package_json.as_str()) {
                Ok(package_json) => package_json,
                // if we can't parse a package.json, then we can't set up references to it
                // so we just skip it and do a best effort
                Err(_) => continue,
            };
            let scripts = package_json.scripts.into_keys().collect::<HashSet<_>>();

            // if in the root, the name should be '//'
            let package_json_name = if repo_root.contains(&wd.package_json) {
                Some("//")
            } else {
                package_json.name.as_ref().map(|name| name.as_str())
            };

            // todo: use jsonc_ast instead of text search
            let rope = crop::Rope::from(data.clone());

            let (package, task) = referenced_task
                .rsplit_once('#')
                .map(|(p, t)| (Some(p), t))
                .unwrap_or((None, &referenced_task));

            if let (Some(package), Some(package_name)) = (package, package_json_name)
                && package_name != package
            {
                continue;
            };

            let Some(start) = data.find(&format!("\"{task}\"")) else {
                continue;
            };
            let end = start + task.len() + 2;

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

            if scripts.contains(task) {
                let location = Location::new(
                    Url::from_file_path(&wd.package_json).expect("only fails if path is relative"),
                    range,
                );
                locations.push(location);
            }
        }

        Ok(Some(locations))
    }

    /// Add code lens items for running a particular task in the turbo.json
    async fn code_lens(&self, params: CodeLensParams) -> LspResult<Option<Vec<CodeLens>>> {
        self.client
            .log_message(MessageType::INFO, "code lens!")
            .await;

        let rope = {
            let map = self.files.lock().expect("only fails if poisoned");
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
        self.client
            .log_message(MessageType::INFO, "workspace folders changed!")
            .await;
    }

    async fn did_change_configuration(&self, _: DidChangeConfigurationParams) {
        self.client
            .log_message(MessageType::INFO, "configuration changed!")
            .await;
    }

    async fn did_change_watched_files(&self, _: DidChangeWatchedFilesParams) {
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
            .log_message(MessageType::INFO, "file opened!")
            .await;

        let rope = crop::Rope::from(document.text_document.text);

        {
            let mut map = self.files.lock().expect("only fails if poisoned");
            map.insert(document.text_document.uri.clone(), rope.clone());
        }

        self.handle_file_update(document.text_document.uri, Some(rope), None)
            .await;
    }

    /// Modify the rope that we have in memory to reflect the changes that were
    /// made to the buffer in the editor
    async fn did_change(&self, document: DidChangeTextDocumentParams) {
        self.client
            .log_message(MessageType::INFO, "file changed!")
            .await;

        let updated_rope = {
            let mut map = self.files.lock().expect("only fails if poisoned");
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

    async fn did_save(&self, _: DidSaveTextDocumentParams) {
        self.client
            .log_message(MessageType::INFO, "file saved!")
            .await;
    }

    async fn did_close(&self, _: DidCloseTextDocumentParams) {
        self.client
            .log_message(MessageType::INFO, "file closed!")
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

        let package_jsons = packages
            .workspaces
            .into_iter()
            .flat_map(|wd| PackageJson::load(&wd.package_json).ok()) // if we can't parse a package.json, then we can't infer its tasks
            .collect::<Vec<_>>();

        let tasks = package_jsons
            .iter()
            .flat_map(|p| p.scripts.keys())
            .unique()
            .map(|s| CompletionItem {
                label: s.to_owned(),
                kind: Some(CompletionItemKind::FIELD),
                ..Default::default()
            });

        let keys = package_jsons
            .iter()
            .flat_map(|p| p.scripts.keys().map(move |k| (p.name.clone(), k)))
            .map(|(package, s)| CompletionItem {
                label: format!("{}#{}", package.unwrap_or_default().into_inner(), s),
                kind: Some(CompletionItemKind::FIELD),
                ..Default::default()
            });

        Ok(Some(CompletionResponse::Array(keys.chain(tasks).collect())))
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

            pidlock: Mutex::new(None),
        }
    }

    pub async fn package_discovery(&self) -> Result<DiscoveryResponse, discovery::Error> {
        let daemon = {
            let mut daemon = self.daemon.clone();
            let daemon = daemon.wait_for(|d| d.is_some()).await;
            let daemon = daemon.as_ref().expect("only fails if self is dropped");
            daemon
                .as_ref()
                .expect("guaranteed to be some above")
                .clone()
        };

        DaemonPackageDiscovery::new(daemon)
            .discover_packages_blocking()
            .await
    }

    /// Handle a file update to a rope, emitting diagnostics if necessary.
    async fn handle_file_update(&self, uri: Url, rope: Option<crop::Rope>, version: Option<i32>) {
        let rope = match rope {
            Some(rope) => rope,
            None => match self.files.lock().expect("only fails if poisoned").get(&uri) {
                Some(files) => files,
                None => return,
            }
            .clone(),
        };

        let contents = rope.chunks().join("");

        let repo_root = self
            .repo_root
            .lock()
            .expect("only fails if poisoned")
            .clone();

        let repo_root = match repo_root {
            Some(repo_root) => repo_root,
            None => {
                self.client
                    .log_message(MessageType::INFO, "received request before initialization")
                    .await;
                return;
            }
        };

        let packages = self.package_discovery().await;

        // package discovery does not yield the root, so we must add it
        let root_turbo_json = repo_root.join_component("turbo.json");
        let workspaces = packages.map(|p| {
            chain(
                p.workspaces.into_iter(),
                iter::once(WorkspaceData {
                    package_json: repo_root.join_component("package.json"),
                    turbo_json: root_turbo_json.exists().then_some(root_turbo_json),
                }),
            )
        });

        let tasks = workspaces.map(|workspaces| {
            workspaces
                .filter_map(|wd| {
                    let package_json = PackageJson::load(&wd.package_json).ok()?; // if we can't load a package.json, then we can't infer its tasks
                    let package_json_name = if (&*repo_root)
                        == wd
                            .package_json
                            .parent()
                            .expect("package.json is always in a directory")
                    {
                        Some("//".to_string())
                    } else {
                        package_json.name.map(|name| name.into_inner())
                    };
                    Some(
                        package_json
                            .scripts
                            .into_keys()
                            .map(move |k| (k, package_json_name.clone())),
                    )
                })
                .flatten()
                .into_group_map()
        });

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

            let pipeline = object
                .and_then(|o| o.get_object("tasks"))
                .map(|p| p.properties.iter());

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
                            let suffix = if let Some(suffix) = strip_lit_prefix(&string, "^") {
                                diagnostics.push(Diagnostic {
                                    message: format!(
                                        "The '^' means \"run the `{}` task in the package's \
                                         dependencies before this one\"",
                                        &suffix.value,
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

/// remove quotes from a string range
fn collapse_string_range(range: jsonc_parser::common::Range) -> jsonc_parser::common::Range {
    jsonc_parser::common::Range {
        start: range.start + 1,
        end: range.end - 1,
    }
}

fn report_invalid_packages_and_tasks(
    tasks: &HashMap<String, Vec<Option<String>>>,
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
