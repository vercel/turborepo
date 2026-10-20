use std::{
    collections::HashSet,
    fs,
    future::{Future, poll_fn},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    task::Poll,
    time::Duration,
};

use tokio::sync::broadcast;
use turbopath::{AbsoluteSystemPathBuf, AnchoredSystemPathBuf};
use turborepo_daemon::PackageChangeEvent;
use turborepo_repository::package_graph::PackageName;
use turborepo_run::RunBuilderInput;
use turborepo_run_opts::{ExecutionSelector, Opts, RunSelector};
use turborepo_telemetry::events::command::CommandEventBuilder;
use turborepo_tracing::TurboSubscriber;
use turborepo_ui::ColorConfig;

use super::{Error, WatchClient};

struct CoordinatorTrace(Arc<AtomicUsize>);

impl tracing::Subscriber for CoordinatorTrace {
    fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
        true
    }

    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }

    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
    fn enter(&self, _: &tracing::span::Id) {}
    fn exit(&self, _: &tracing::span::Id) {}

    fn event(&self, event: &tracing::Event<'_>) {
        struct Settled<'a>(&'a AtomicUsize);
        impl tracing::field::Visit for Settled<'_> {
            fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                if field.name() == "message"
                    && format!("{value:?}") == "all runs completed, ready for next event"
                {
                    self.0.fetch_add(1, Ordering::SeqCst);
                }
            }
        }
        if event.metadata().target() == "turborepo_watch" {
            event.record(&mut Settled(&self.0));
        }
    }
}

async fn settled_runs(settled: &AtomicUsize, count: usize) -> Result<(), String> {
    tokio::time::timeout(Duration::from_secs(30), async {
        while settled.load(Ordering::SeqCst) < count {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .map_err(|_| format!("coordinator did not settle {count} runs"))
}

async fn completed_source(path: &std::path::Path, source: &str) -> Result<(), String> {
    tokio::time::timeout(Duration::from_secs(30), async {
        while fs::read_to_string(path).ok().as_deref() != Some(source) {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .map_err(|_| format!("no task completed with source {source:?}"))
}

#[tokio::test]
async fn coordinator_coalesces_classified_events_before_starting_real_tasks() {
    let settled = Arc::new(AtomicUsize::new(0));
    let _trace = tracing::subscriber::set_default(CoordinatorTrace(settled.clone()));
    let fixture = tempfile::tempdir().unwrap();
    let root = AbsoluteSystemPathBuf::try_from(fixture.path()).unwrap();
    let package = fixture.path().join("packages/a");
    fs::create_dir_all(&package).unwrap();
    fs::write(
        fixture.path().join("package.json"),
        r#"{"name":"coordinator-test","private":true,"packageManager":"npm@10.0.0","workspaces":["packages/*"]}"#,
    )
    .unwrap();
    fs::write(fixture.path().join(".gitignore"), ".markers/\n.turbo/\n").unwrap();
    fs::write(
        fixture.path().join("turbo.json"),
        r#"{"tasks":{"build":{"cache":false,"inputs":["source.txt","build.js"]}}}"#,
    )
    .unwrap();
    fs::write(
        package.join("package.json"),
        r#"{"name":"a","scripts":{"build":"node build.js"}}"#,
    )
    .unwrap();
    fs::write(
        package.join("build.js"),
        "const fs = require('fs');\nconst source = fs.readFileSync('source.txt', \
         'utf8');\nfs.mkdirSync('.markers', { recursive: true \
         });\nfs.appendFileSync('.markers/starts', source + \
         '\\n');\nfs.writeFileSync('.markers/complete', source);\n",
    )
    .unwrap();
    let source_file = package.join("source.txt");
    let complete = package.join(".markers/complete");
    fs::write(&source_file, "initial").unwrap();

    let color = ColorConfig::new(false);
    let mut opts = Opts::new(
        &root,
        &RunSelector::default(),
        &ExecutionSelector {
            tasks: vec!["build".into()],
            ..Default::default()
        },
        turborepo_config::ConfigurationOptions::default(),
    )
    .unwrap();
    opts.run_opts.ui_mode = turborepo_types::UIMode::Stream;
    let dispatches = Arc::new(AtomicUsize::new(0));
    let selected = dispatches.clone();
    let mut client = WatchClient::new(
        RunBuilderInput {
            repo_root: root,
            color_config: color,
            opts,
            version: "test",
            api_auth: None,
            managed_environment: None,
        },
        Box::new(move || {
            selected.fetch_add(1, Ordering::SeqCst);
            Ok(None)
        }),
        false,
        CommandEventBuilder::new("watch"),
        None,
        &TurboSubscriber::new_with_verbosity(0, &color),
        0,
    )
    .await
    .unwrap();
    let (events, receiver) = broadcast::channel(16);
    client.package_change_events = receiver;
    events.send(PackageChangeEvent::Rediscover).unwrap();

    let result = async {
        let start = client.start();
        tokio::pin!(start);
        tokio::select! {
            result = &mut start => return Err(format!("coordinator exited before initial task: {result:?}")),
            result = async {
                completed_source(&complete, "initial").await?;
                settled_runs(&settled, 1).await
            } => result?,
        }
        let initial_dispatches = dispatches.load(Ordering::SeqCst);
        let mut early_dispatches = Vec::new();
        tokio::time::pause();
        let burst_started = tokio::time::Instant::now();
        let mut delivered_at = Vec::new();
        for index in 0..5 {
            if index > 0 {
                tokio::time::advance(Duration::from_millis(100)).await;
            }
            fs::write(&source_file, format!("edit-{index}")).unwrap();
            events
                .send(PackageChangeEvent::Package {
                    name: PackageName::from("a"),
                    changed_files: Arc::new(HashSet::from([
                        AnchoredSystemPathBuf::from_raw("packages/a/source.txt").unwrap(),
                    ])),
                })
                .unwrap();
            // Poll the real collector and coordinator without letting idle time
            // advance the clock or waiting for filesystem classification.
            let polled = poll_fn(|cx| Poll::Ready(start.as_mut().poll(cx))).await;
            if polled.is_ready() || events.len() != 0 {
                tokio::time::resume();
                return Err(format!("classified event was not consumed by a live coordinator: {polled:?}"));
            }
            delivered_at.push(burst_started.elapsed());
            early_dispatches.push(dispatches.load(Ordering::SeqCst));
        }
        tokio::time::advance(Duration::from_millis(200)).await;
        tokio::time::resume();
        tokio::select! {
            result = &mut start => return Err(format!("coordinator exited before final task: {result:?}")),
            result = async {
                completed_source(&complete, "edit-4").await?;
                settled_runs(&settled, 2).await
            } => result?,
        }
        loop {
            let dispatched = dispatches.load(Ordering::SeqCst);
            tokio::select! {
                result = &mut start => return Err(format!("coordinator exited before all dispatched runs settled: {result:?}")),
                result = settled_runs(&settled, dispatched) => result?,
            }
            if dispatches.load(Ordering::SeqCst) == dispatched {
                break;
            }
        }
        drop(events);
        let stopped = tokio::time::timeout(Duration::from_secs(5), &mut start)
            .await
            .map_err(|_| "coordinator did not stop after event channel closed".to_string())?;
        if !matches!(stopped, Err(Error::PackageChangeClosed)) {
            return Err(format!("unexpected coordinator exit: {stopped:?}"));
        }
        let starts = fs::read_to_string(package.join(".markers/starts")).unwrap();
        eprintln!("classified event delivery: {delivered_at:?}; task starts: {starts:?}");
        if delivered_at != (0..5).map(|i| Duration::from_millis(i * 100)).collect::<Vec<_>>() {
            return Err(format!("classified delivery did not follow the controlled clock: {delivered_at:?}"));
        }
        if starts.lines().collect::<Vec<_>>() != ["initial", "edit-4"] {
            return Err(format!("expected one final-input rebuild, task starts were {starts:?}"));
        }
        if early_dispatches != vec![initial_dispatches; 5] {
            return Err(format!("run dispatched before burst became quiet: {early_dispatches:?}"));
        }
        if dispatches.load(Ordering::SeqCst) != initial_dispatches + 1 {
            return Err("expected exactly one follow-up run dispatch".into());
        }
        Ok(())
    }
    .await;
    client.shutdown().await;
    assert!(client.lifecycle.active_runs.is_empty());
    assert!(client.lifecycle.background_stoppers.is_empty());
    result.unwrap();
}
