use std::{env, io, mem, process, sync::Arc};

use camino::Utf8Path;
use clap::CommandFactory;
use clap_complete::generate;
pub use error::Error;
use tracing::{debug, error, log::warn};
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};
use turborepo_api_client::SharedHttpClient;
use turborepo_repository::inference::{RepoMode, RepoState};
use turborepo_shim::TurboState;
use turborepo_telemetry::{
    events::{command::CommandEventBuilder, generic::GenericEventBuilder, EventBuilder},
    init_telemetry, TelemetryHandle,
};
use turborepo_ui::{ColorConfig, GREY};

use crate::{
    cli::error::print_potential_tasks,
    commands::{
        bin, boundaries, config, daemon, docs, generate, get_mfe_port, info, link, login, logout,
        ls, prune, query, run, telemetry, unlink, CommandBase,
    },
    get_version,
    run::watch::WatchClient,
    tracing::TurboSubscriber,
};

mod args;
mod error;
mod observability;
#[cfg(test)]
mod test;

#[allow(unused_imports)]
pub use args::{
    AffectedArgs, Args, BoundariesIgnore, Command, DaemonCommand, ExecutionArgs, GenerateCommand,
    GenerateWorkspaceArgs, GeneratorCustomArgs, LsArgs, OutputFormat, QuerySubcommand, RunArgs,
    TelemetryCommand, Verbosity,
};

fn exit_with_heap_profile(code: i32) -> ! {
    #[cfg(feature = "heap-dhat")]
    crate::heap_profile::finish_global();

    process::exit(code);
}

// Global turbo sets this environment variable to its cwd so that local
// turbo can use it for package inference.
pub const INVOCATION_DIR_ENV_VAR: &str = "TURBO_INVOCATION_DIR";

/// Returns a scaled thread count for rayon's global pool based on
/// available CPU cores, capped at
/// [`crate::rayon_compat::MAX_RAYON_THREADS`].
///
/// See [`init_rayon_pool`] and <https://github.com/vercel/turborepo/issues/12251>
fn rayon_pool_size() -> usize {
    let cpus = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    crate::rayon_compat::scale_thread_count(cpus)
}

/// Explicitly initialize rayon's global thread pool early so we control
/// its size and initialization timing.
///
/// If `RAYON_NUM_THREADS` is set, its value is still clamped to
/// [`crate::rayon_compat::MAX_RAYON_THREADS`] to prevent the known
/// deadlock on high-core-count machines.
fn init_rayon_pool() {
    let pool_size = match std::env::var("RAYON_NUM_THREADS")
        .ok()
        .and_then(|v| v.parse().ok())
    {
        Some(user_val) => {
            let clamped = crate::rayon_compat::scale_thread_count(user_val);
            if clamped < user_val {
                tracing::debug!(
                    requested = user_val,
                    clamped,
                    max = crate::rayon_compat::MAX_RAYON_THREADS,
                    "RAYON_NUM_THREADS exceeds safe limit, clamping"
                );
            }
            clamped
        }
        None => rayon_pool_size(),
    };

    tracing::info!(
        rayon_pool_size = pool_size,
        "initializing rayon global pool"
    );

    let builder = rayon::ThreadPoolBuilder::new().num_threads(pool_size);
    if let Err(e) = builder.build_global() {
        tracing::warn!("rayon global pool already initialized: {e}");
    }
}

fn initialize_deferred_telemetry_client(
    http_client: SharedHttpClient,
    color_config: ColorConfig,
    version: &str,
) -> Option<TelemetryHandle> {
    let deferred_client = turborepo_api_client::telemetry::DeferredTelemetryClient::new(
        http_client.clone(),
        "https://telemetry.vercel.com",
        version,
    );
    match init_telemetry(deferred_client, color_config) {
        Ok((handle, enabled)) => {
            if enabled {
                http_client.activate();
            }
            Some(handle)
        }
        Err(error) => {
            debug!("failed to start telemetry: {:?}", error);
            None
        }
    }
}

#[derive(PartialEq)]
enum PrintVersionState {
    Enabled,
    Disabled,
}

fn get_print_version_state() -> PrintVersionState {
    env::var("TURBO_PRINT_VERSION_DISABLED")
        .map(|var| match var.as_str() {
            "1" | "true" => PrintVersionState::Disabled,
            _ => PrintVersionState::Enabled,
        })
        .unwrap_or(PrintVersionState::Enabled)
}

#[derive(PartialEq)]
enum CIState {
    Inside,
    Outside,
}

fn get_ci_state() -> CIState {
    match turborepo_ci::is_ci() {
        true => CIState::Inside,
        _ => CIState::Outside,
    }
}

fn should_print_version() -> bool {
    let print_version_state = get_print_version_state();
    let ci_state = get_ci_state();

    print_version_state == PrintVersionState::Enabled && ci_state == CIState::Outside
}

fn set_run_flags<'a>(
    command: &'a mut Command,
    repo_state: &'a Option<RepoState>,
    cli_args: &'a Args,
) -> Result<&'a mut Command, Error> {
    match command {
        Command::Run {
            run_args: _,
            execution_args,
        }
        | Command::Watch { execution_args, .. } => {
            // Don't overwrite the flag if it's already been set for whatever reason
            execution_args.single_package = execution_args.single_package
                || repo_state
                    .as_ref()
                    .map(|repo_state| matches!(repo_state.mode, RepoMode::SinglePackage))
                    .unwrap_or(false);
            // If this is a run command, and we know the actual invocation path, set the
            // inference root, as long as the user hasn't overridden the cwd
            if cli_args.cwd.is_none() {
                if let Ok(invocation_dir) = env::var(INVOCATION_DIR_ENV_VAR) {
                    // TODO: this calculation can probably be wrapped into the path library
                    // and made a little more robust or clear
                    let invocation_path = Utf8Path::new(&invocation_dir);

                    // If repo state doesn't exist, we're either local turbo running at the root
                    // (cwd), or inference failed.
                    // If repo state does exist, we're global turbo, and want to calculate
                    // package inference based on the repo root
                    let this_dir = AbsoluteSystemPathBuf::cwd()?;
                    let repo_root = repo_state.as_ref().map_or(&this_dir, |r| &r.root);
                    if let Some(relative_path) = inferred_package_root(invocation_path, repo_root) {
                        debug!("pkg_inference_root set to \"{}\"", relative_path);
                        execution_args.pkg_inference_root = Some(relative_path);
                    }
                } else {
                    debug!("{} not set", INVOCATION_DIR_ENV_VAR);
                }
            }
        }
        _ => {}
    }
    Ok(command)
}

fn inferred_package_root(
    invocation_path: &Utf8Path,
    repo_root: &AbsoluteSystemPath,
) -> Option<String> {
    let relative_path = invocation_path.strip_prefix(repo_root).ok()?;
    (!relative_path.as_str().is_empty()).then(|| relative_path.to_string().replace('\\', "/"))
}

fn default_to_run_command(cli_args: &Args) -> Result<Command, Error> {
    let run_args = cli_args.run_args.clone().unwrap_or_default();
    let execution_args = cli_args
        .execution_args
        // We clone instead of take as take would leave the command base a copy of cli_args
        // missing any execution args.
        .clone()
        .ok_or_else(|| Error::NoCommand)?;

    if execution_args.tasks.is_empty() {
        let mut cmd = <Args as CommandFactory>::command();
        let _ = cmd.print_help();
        exit_with_heap_profile(1);
    }

    Ok(Command::Run {
        run_args: Box::new(run_args),
        execution_args: Box::new(execution_args),
    })
}

#[tracing::instrument(skip_all)]
fn get_command(cli_args: &mut Args) -> Result<Command, Error> {
    if let Some(command) = mem::take(&mut cli_args.command) {
        Ok(command)
    } else {
        // If there is no command, we set the command to `Command::Run` with
        // `self.parsed_args.run_args` as arguments.
        default_to_run_command(cli_args)
    }
}

/// Runs the CLI by parsing arguments with clap, then either calling Rust code
/// directly or returning a payload for the Go code to use.
///
/// Scenarios:
/// 1. inference failed, we're running this global turbo. no repo state
/// 2. --skip-infer was passed, assume we're local turbo and run. no repo state
/// 3. There is no local turbo, we're running the global one. repo state exists
/// 4. turbo binary path is set, and it's this one. repo state exists
///
/// # Arguments
///
/// * `repo_state`: If we have done repository inference and NOT executed local
///   turbo, such as in the case where `TURBO_BINARY_PATH` is set, we use it
///   here to modify clap's arguments.
/// * `logger`: The logger to use for the run.
/// * `color_config`: The color configuration to use for the run, i.e. whether
///   we should colorize output.
///
/// returns: Result<i32, Error>
pub fn run(
    repo_state: Option<RepoState>,
    logger: &TurboSubscriber,
    color_config: ColorConfig,
    query_server: Option<Arc<dyn turborepo_query_api::QueryServer>>,
) -> Result<i32, Error> {
    // Initialize rayon's global pool before the tokio runtime so we
    // control thread count and avoid lazy initialization during a hot path.
    init_rayon_pool();

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(Error::Runtime)?;

    runtime.block_on(run_main(repo_state, logger, color_config, query_server))
}

#[tracing::instrument(skip_all)]
async fn run_main(
    repo_state: Option<RepoState>,
    #[allow(unused_variables)] logger: &TurboSubscriber,
    color_config: ColorConfig,
    query_server: Option<Arc<dyn turborepo_query_api::QueryServer>>,
) -> Result<i32, Error> {
    let _cli_run_span = tracing::info_span!("cli_run").entered();
    let http_client = SharedHttpClient::new();

    let mut cli_args = {
        let _span = tracing::info_span!("cli_arg_parsing").entered();
        Args::new(env::args_os().collect())
    };
    let version = get_version();

    // Initialize telemetry immediately so events are captured from startup.
    // The shared HTTP client is only activated if telemetry is actually
    // enabled for this invocation.
    let telemetry_handle = {
        let _span = tracing::info_span!("telemetry_init").entered();
        initialize_deferred_telemetry_client(http_client.clone(), color_config, version)
    };

    let mut command = get_command(&mut cli_args)?;

    // Suppress the version banner in --json mode — all output on stdout
    // must be machine-readable NDJSON.
    let is_json_mode = matches!(
        &command,
        Command::Run { execution_args, .. } | Command::Watch { execution_args, .. }
            if execution_args.json
    );
    if should_print_version() && !is_json_mode {
        eprintln!("{}", GREY.apply_to(format!("• turbo {}", get_version())));
    }

    // Set some run flags if we have the data and are executing a Run
    set_run_flags(&mut command, &repo_state, &cli_args)?;

    let cwd = repo_state
        .as_ref()
        .map(|state| state.root.as_path())
        .or(cli_args.cwd.as_deref());

    let repo_root = if let Some(cwd) = cwd {
        AbsoluteSystemPathBuf::from_cwd(cwd)?
    } else {
        AbsoluteSystemPathBuf::cwd()?
    };

    // Windows has this annoying habit of abbreviating paths
    // like `C:\Users\Admini~1` instead of `C:\Users\Administrator`
    // We canonicalize to get the proper, full length path
    let repo_root = if cfg!(windows) {
        repo_root.to_realpath()?
    } else {
        repo_root
    };

    cli_args.command = Some(command);

    let root_telemetry = GenericEventBuilder::new();
    root_telemetry.track_start();

    // track system info
    root_telemetry.track_platform(TurboState::platform_name());
    root_telemetry.track_version(TurboState::version());
    root_telemetry.track_ai_agent(turborepo_ai_agents::get_agent());
    root_telemetry.track_cpus(
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1),
    );
    // track args
    cli_args.track(&root_telemetry);

    let Some(command) = cli_args.command.as_ref() else {
        return Err(Error::NoCommand);
    };

    let cli_result = match command {
        Command::Bin => {
            CommandEventBuilder::new("bin")
                .with_parent(&root_telemetry)
                .track_call();
            bin::run()?;

            Ok(0)
        }
        Command::GetMfePort => {
            let event = CommandEventBuilder::new("get-mfe-port").with_parent(&root_telemetry);
            event.track_call();

            let base = CommandBase::new(cli_args.clone(), repo_root, version, color_config)?;
            get_mfe_port::run(&base).await?;

            Ok(0)
        }
        Command::Boundaries { ignore, reason, .. } => {
            let event = CommandEventBuilder::new("boundaries").with_parent(&root_telemetry);
            let ignore = *ignore;
            let reason = reason.clone();

            event.track_call();
            let base = CommandBase::new(cli_args.clone(), repo_root, version, color_config)?;

            Ok(boundaries::run(base, event, ignore, reason).await?)
        }
        #[allow(unused_variables)]
        Command::Daemon {
            command,
            idle_time,
            turbo_json_path,
        } => {
            let event = CommandEventBuilder::new("daemon").with_parent(&root_telemetry);
            event.track_call();
            let base = CommandBase::new(cli_args.clone(), repo_root, version, color_config)?;
            event.track_ui_mode(base.opts.run_opts.ui_mode);

            match command {
                Some(command) => {
                    daemon::daemon_client(command, &base, turbo_json_path.clone()).await
                }
                None => {
                    daemon::daemon_server(&base, idle_time, turbo_json_path.clone(), logger).await
                }
            }?;

            Ok(0)
        }
        Command::Devtools { port, no_open } => {
            let event = CommandEventBuilder::new("devtools").with_parent(&root_telemetry);
            event.track_call();

            crate::commands::devtools::run(repo_root, *port, *no_open).await?;
            Ok(0)
        }
        Command::Docs {
            query,
            docs_version,
        } => {
            let event = CommandEventBuilder::new("docs").with_parent(&root_telemetry);
            event.track_call();

            docs::run(query, docs_version.as_deref()).await?;
            Ok(0)
        }
        Command::Generate {
            tag,
            generator_name,
            config,
            root,
            args,
            command,
        } => {
            let event = CommandEventBuilder::new("generate").with_parent(&root_telemetry);
            event.track_call();
            let tag = tag.clone().unwrap_or_else(|| get_version().to_string());
            // build GeneratorCustomArgs struct
            let args = GeneratorCustomArgs {
                generator_name: generator_name.clone(),
                config: config.clone(),
                root: root.clone(),
                args: args.clone(),
            };
            let child_event = event.child();
            generate::run(&repo_root, &tag, command, &args, child_event)?;
            Ok(0)
        }
        Command::Info => {
            let event = CommandEventBuilder::new("info").with_parent(&root_telemetry);

            event.track_call();
            let base = CommandBase::new(cli_args.clone(), repo_root, version, color_config)?;
            event.track_ui_mode(base.opts.run_opts.ui_mode);

            info::run(base).await;
            Ok(0)
        }
        Command::Telemetry { command } => {
            let event = CommandEventBuilder::new("telemetry").with_parent(&root_telemetry);
            event.track_call();
            let mut base = CommandBase::new(cli_args.clone(), repo_root, version, color_config)?;
            event.track_ui_mode(base.opts.run_opts.ui_mode);
            let child_event = event.child();
            telemetry::configure(command, &mut base, child_event);
            Ok(0)
        }
        Command::Scan => {
            let event = CommandEventBuilder::new("scan").with_parent(&root_telemetry);
            event.track_call();
            warn!("`turbo scan` is deprecated and will be removed in a future major version.");
            Ok(1)
        }
        Command::Config => {
            CommandEventBuilder::new("config")
                .with_parent(&root_telemetry)
                .track_call();
            config::run(repo_root, cli_args).await?;
            Ok(0)
        }
        Command::Ls {
            packages, output, ..
        } => {
            let Some(ref query_server) = query_server else {
                return Err(error::Error::QueryNotAvailable);
            };
            let event = CommandEventBuilder::new("info").with_parent(&root_telemetry);

            event.track_call();
            let output = *output;
            let packages = packages.clone();
            let base = CommandBase::new(cli_args, repo_root, version, color_config)?;
            event.track_ui_mode(base.opts.run_opts.ui_mode);
            ls::run(base, packages, event, output, query_server.as_ref()).await?;

            Ok(0)
        }
        Command::Link {
            no_gitignore,
            scope,
            yes,
        } => {
            let event = CommandEventBuilder::new("link").with_parent(&root_telemetry);
            event.track_call();

            if cli_args.team.is_some() {
                warn!("team flag does not set the scope for linking. Use --scope instead.");
            }

            if cli_args.test_run {
                println!("Link test run successful");
                return Ok(0);
            }

            let modify_gitignore = !*no_gitignore;
            let yes = *yes;
            let scope = scope.clone();
            let mut base = CommandBase::new(cli_args, repo_root, version, color_config)?;
            event.track_ui_mode(base.opts.run_opts.ui_mode);

            link::link(&mut base, scope, modify_gitignore, yes).await?;

            Ok(0)
        }
        Command::Logout { invalidate } => {
            let event = CommandEventBuilder::new("logout").with_parent(&root_telemetry);
            event.track_call();
            let invalidate = *invalidate;

            let mut base = CommandBase::new(cli_args, repo_root, version, color_config)?;
            event.track_ui_mode(base.opts.run_opts.ui_mode);
            let event_child = event.child();

            logout::logout(&mut base, invalidate, event_child).await?;

            Ok(0)
        }
        Command::Login {
            sso_team,
            force,
            manual,
        } => {
            let event = CommandEventBuilder::new("login").with_parent(&root_telemetry);
            event.track_call();
            if cli_args.test_run {
                println!("Login test run successful");
                return Ok(0);
            }

            let sso_team = sso_team.clone();
            let force = *force;
            let manual = *manual;

            let mut base = CommandBase::new(cli_args, repo_root, version, color_config)?;
            event.track_ui_mode(base.opts.run_opts.ui_mode);
            let event_child = event.child();

            login::login(&mut base, event_child, sso_team.as_deref(), force, manual).await?;

            Ok(0)
        }
        Command::Unlink => {
            let event = CommandEventBuilder::new("unlink").with_parent(&root_telemetry);
            event.track_call();
            if cli_args.test_run {
                println!("Unlink test run successful");
                return Ok(0);
            }

            let mut base = CommandBase::new(cli_args, repo_root, version, color_config)?;
            event.track_ui_mode(base.opts.run_opts.ui_mode);

            unlink::unlink(&mut base)?;

            Ok(0)
        }
        Command::Run {
            run_args,
            execution_args,
        } => {
            let event = CommandEventBuilder::new("run").with_parent(&root_telemetry);
            event.track_call();

            let base = {
                let _span = tracing::info_span!("command_base_new").entered();
                CommandBase::new(cli_args.clone(), repo_root, version, color_config)?
            };
            event.track_ui_mode(base.opts.run_opts.ui_mode);

            if execution_args.tasks.is_empty() {
                print_potential_tasks(base, event).await?;
                finalize_chrome_profile(logger, version);
                return Ok(1);
            }

            run_args.track(&event);
            let verbosity: u8 = cli_args.verbosity.into();
            let exit_code = run::run(
                base,
                event,
                http_client,
                query_server.clone(),
                logger,
                verbosity,
            )
            .await
            .inspect(|code| {
                if *code != 0 {
                    error!("run failed: command  exited ({code})");
                }
            })?;

            // Chrome tracing is enabled early in shim::run(). Here we just
            // flush and generate the markdown summary.
            finalize_chrome_profile(logger, version);

            Ok(exit_code)
        }
        Command::Query {
            subcommand,
            query,
            variables,
            schema,
        } => {
            let Some(ref query_server) = query_server else {
                return Err(error::Error::QueryNotAvailable);
            };
            let subcommand = subcommand.clone();
            let query = query.clone();
            let variables = variables.clone();
            let schema = *schema;
            let event = CommandEventBuilder::new("query").with_parent(&root_telemetry);
            event.track_call();

            let base = CommandBase::new(cli_args, repo_root, version, color_config)?;
            event.track_ui_mode(base.opts.run_opts.ui_mode);

            let query = query::run(
                base,
                event,
                subcommand,
                query,
                variables.as_deref(),
                schema,
                query_server.as_ref(),
            )
            .await?;

            Ok(query)
        }
        Command::Watch {
            execution_args,
            experimental_write_cache,
        } => {
            let event = CommandEventBuilder::new("watch").with_parent(&root_telemetry);
            event.track_call();
            let base = CommandBase::new(cli_args.clone(), repo_root, version, color_config)?;
            event.track_ui_mode(base.opts.run_opts.ui_mode);

            if execution_args.tasks.is_empty() {
                print_potential_tasks(base, event).await?;
                return Ok(1);
            }

            let verbosity: u8 = cli_args.verbosity.into();
            let mut client = WatchClient::new(
                base,
                *experimental_write_cache,
                event,
                query_server.clone(),
                logger,
                verbosity,
            )
            .await?;
            match client.start().await {
                Ok(()) => {}
                Err(crate::run::watch::Error::SignalInterrupt) => {
                    // Normal shutdown via Ctrl+C — not an error.
                }
                Err(e) => {
                    client.shutdown().await;
                    return Err(e.into());
                }
            }
            client.shutdown().await;
            if let Some(path) = logger.stderr_redirect_path() {
                logger.restore_stderr();
                println!("Verbose logs written to {path}");
            }
            return Ok(0);
        }
        Command::Prune {
            scope,
            scope_arg,
            docker,
            production,
            output_dir,
            use_gitignore,
        } => {
            let event = CommandEventBuilder::new("prune").with_parent(&root_telemetry);
            event.track_call();
            let scope = scope_arg
                .as_ref()
                .or(scope.as_ref())
                .cloned()
                .unwrap_or_default();
            let docker = *docker;
            let production = *production;
            let output_dir = output_dir.clone();
            let use_gitignore = use_gitignore.unwrap_or(true);
            let base = CommandBase::new(cli_args, repo_root, version, color_config)?;
            event.track_ui_mode(base.opts.run_opts.ui_mode);
            let event_child = event.child();
            prune::prune(
                &base,
                &scope,
                docker,
                production,
                &output_dir,
                use_gitignore,
                event_child,
            )
            .await?;
            Ok(0)
        }
        Command::Completion { shell } => {
            CommandEventBuilder::new("completion")
                .with_parent(&root_telemetry)
                .track_call();
            generate(*shell, &mut Args::command(), "turbo", &mut io::stdout());
            Ok(0)
        }
    };

    root_telemetry.track_end();
    match telemetry_handle {
        Some(handle) => handle.close_with_timeout().await,
        None => debug!("Skipping telemetry close - not initialized"),
    }

    cli_result
}

fn finalize_chrome_profile(logger: &TurboSubscriber, version: &str) {
    let Some(file_path) = logger.chrome_tracing_file() else {
        return;
    };

    let _ = logger.flush_chrome_tracing();

    if let Err(e) = crate::tracing::inject_trace_metadata(std::path::Path::new(&file_path), version)
    {
        warn!("Failed to inject trace metadata: {e}");
    }

    let md_path = format!("{file_path}.md");
    if let Err(e) = turborepo_profile_md::trace_to_markdown(
        std::path::Path::new(&file_path),
        std::path::Path::new(&md_path),
    ) {
        warn!("Failed to generate profile markdown: {e}");
    }
}
