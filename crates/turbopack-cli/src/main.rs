#![feature(future_join)]
#![feature(min_specialization)]

use std::{borrow::Cow, path::Path};

use anyhow::{Context, Result};
use clap::Parser;
use tracing_appender::non_blocking::DEFAULT_BUFFERED_LINES_LIMIT;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, Registry};
use turbopack_cli::{arguments::Arguments, register};
use turbopack_cli_utils::{exit::exit_guard, raw_trace::RawTraceLayer};

#[global_allocator]
static ALLOC: turbo_tasks_malloc::TurboMalloc = turbo_tasks_malloc::TurboMalloc;

fn main() {
    use turbo_tasks_malloc::TurboMalloc;

    let args = Arguments::parse();

    let trace = std::env::var("TURBOPACK_TRACING").ok();

    let _guard = if let Some(mut trace) = trace {
        // Trace presets
        match trace.as_str() {
            "turbopack" => {
                trace = "root=info,turbopack=info,turbopack_core=info,turbopack_ecmascript=info,\
                         turbopack_css=info,turbopack_dev=info,turbopack_image=info,\
                         turbopack_dev_server=info,turbopack_json=info,turbopack_mdx=info,\
                         turbopack_node=info,turbopack_static=info,turbopack_cli_utils=info,\
                         turbopack_cli=info,turbopack_ecmascript=info"
                    .to_string();
            }
            "turbo-tasks" => {
                trace = "root=info,turbopack=info,turbopack_core=info,turbopack_ecmascript=info,\
                         turbopack_css=info,turbopack_dev=info,turbopack_image=info,\
                         turbopack_dev_server=info,turbopack_json=info,turbopack_mdx=info,\
                         turbopack_node=info,turbopack_static=info,turbopack_cli_utils=info,\
                         turbopack_cli=info,turbopack_ecmascript=info,turbo_tasks=info,\
                         turbo_tasks_viz=info,turbo_tasks_memory=info,turbo_tasks_fs=info"
                    .to_string();
            }
            _ => {}
        }

        let subscriber = Registry::default();

        let subscriber = subscriber.with(EnvFilter::builder().parse(trace).unwrap());

        let internal_dir = args
            .dir()
            .unwrap_or_else(|| Path::new("."))
            .join(".turbopack");
        std::fs::create_dir_all(&internal_dir)
            .context("Unable to create .turbopack directory")
            .unwrap();
        let trace_file = internal_dir.join("trace.log");
        let (writer, guard) = tracing_appender::non_blocking::NonBlockingBuilder::default()
            .lossy(false)
            .buffered_lines_limit(DEFAULT_BUFFERED_LINES_LIMIT * 8)
            .finish(std::fs::File::create(trace_file).unwrap());
        let subscriber = subscriber.with(RawTraceLayer::new(writer));

        let guard = exit_guard(guard).unwrap();

        subscriber.init();

        Some(guard)
    } else {
        None
    };

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .on_thread_stop(|| {
            TurboMalloc::thread_stop();
        })
        .build()
        .unwrap()
        .block_on(main_inner(args))
        .unwrap();
}

async fn main_inner(args: Arguments) -> Result<()> {
    register();

    match args {
        Arguments::Dev(args) => turbopack_cli::dev::start_server(&args).await,
    }
}
