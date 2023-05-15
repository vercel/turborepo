#![feature(future_join)]
#![feature(min_specialization)]

use std::{
    borrow::Cow,
    sync::{Arc, Mutex},
};

use anyhow::{Context, Result};
use clap::Parser;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, Registry};
use turbopack_cli::{arguments::Arguments, raw_trace::RawTraceLayer, register};

#[global_allocator]
static ALLOC: turbo_tasks_malloc::TurboMalloc = turbo_tasks_malloc::TurboMalloc;

struct CloseGuard<T>(Arc<Mutex<Option<T>>>);

impl<T> Drop for CloseGuard<T> {
    fn drop(&mut self) {
        drop(self.0.lock().unwrap().take())
    }
}

fn close_guard<T: Send + 'static>(guard: T) -> Result<CloseGuard<T>> {
    let guard = Arc::new(Mutex::new(Some(guard)));
    {
        let guard = guard.clone();
        ctrlc::set_handler(move || {
            println!("Flushing trace file... (ctrl-c)");
            drop(guard.lock().unwrap().take());
            println!("Flushed trace file");
            std::process::exit(0);
        })
        .context("Unable to set ctrl-c handler")?;
    }
    Ok(CloseGuard(guard))
}

fn main() {
    use turbo_tasks_malloc::TurboMalloc;

    let subscriber = Registry::default();

    let subscriber = subscriber.with(
        EnvFilter::builder()
            .parse(std::env::var("TURBOPACK_TRACE").map_or_else(
                |_| {
                    Cow::Borrowed(
                        "turbopack=info,turbopack_core=info,turbopack_ecmascript=info,\
                         turbopack_css=info,turbopack_dev=info,turbopack_iamge=info,\
                         turbopack_json=info,turbopack_mdx=info,turbopack_node=info,\
                         turbopack_static=info,turbopack_dev_server=info,turbopack_cli_utils=info,\
                         turbopack_cli=info,turbopack_ecmascript=info,turbo_tasks=info,\
                         turbo_tasks_fs=info,turbo_tasks_bytes=info,turbo_tasks_env=info,\
                         turbo_tasks_fetch=info,turbo_tasks_hash=info",
                    )
                },
                |s| Cow::Owned(s),
            ))
            .unwrap(),
    );

    std::fs::create_dir_all("./.turbopack")
        .context("Unable to create .turbopack directory")
        .unwrap();
    let (writer, guard) =
        tracing_appender::non_blocking(std::fs::File::create("./.turbopack/trace.log").unwrap());
    let subscriber = subscriber.with(RawTraceLayer::new(writer));

    let guard = close_guard(guard).unwrap();

    subscriber.init();

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .on_thread_stop(|| {
            TurboMalloc::thread_stop();
        })
        .build()
        .unwrap()
        .block_on(main_inner())
        .unwrap();

    println!("Flushing trace file...");
    drop(guard);
    println!("Flushed trace file");
}

async fn main_inner() -> Result<()> {
    register();
    let args = Arguments::parse();

    match args {
        Arguments::Dev(args) => turbopack_cli::dev::start_server(&args).await,
    }
}
