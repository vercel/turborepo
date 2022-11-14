use std::{
    io::{self, BufRead, BufReader, Read, Write},
    panic::UnwindSafe,
    process::Command,
    time::Duration,
};

use anyhow::Result;
use chromiumoxide::{
    browser::{Browser, BrowserConfig},
    error::CdpError::Ws,
};
use criterion::{
    async_executor::AsyncExecutor,
    black_box,
    measurement::{Measurement, WallTime},
    AsyncBencher,
};
use futures::{Future, StreamExt};
pub use page_guard::PageGuard;
pub use prepared_app::PreparedApp;
use regex::Regex;
use tungstenite::{error::ProtocolError::ResetWithoutClosingHandshake, Error::Protocol};
use turbo_tasks::util::FormatDuration;
use turbo_tasks_testing::retry::{retry, retry_async};
use turbopack_create_test_app::test_app_builder::{PackageJsonConfig, TestApp, TestAppBuilder};

use self::env::read_env_bool;
use crate::bundlers::Bundler;

pub mod env;
pub mod npm;
mod page_guard;
mod prepared_app;
pub mod rand;

pub const BINDING_NAME: &str = "__turbopackBenchBinding";

fn retry_default<A, F, R, E>(args: A, f: F) -> Result<R, E>
where
    F: Fn(&mut A) -> Result<R, E>,
{
    // waits 5, 10, 20, 40 seconds = 75 seconds total
    retry(args, f, 3, Duration::from_secs(5))
}

async fn retry_async_default<A, F, Fut, R, E>(args: A, f: F) -> Result<R, E>
where
    F: Fn(&mut A) -> Fut,
    Fut: Future<Output = Result<R, E>>,
{
    // waits 5, 10, 20, 40 seconds = 75 seconds total
    retry_async(args, f, 3, Duration::from_secs(5)).await
}

pub fn build_test(module_count: usize, bundler: &dyn Bundler) -> TestApp {
    let test_app = TestAppBuilder {
        module_count,
        directories_count: module_count / 20,
        package_json: Some(PackageJsonConfig {
            react_version: bundler.react_version().to_string(),
        }),
        ..Default::default()
    }
    .build()
    .unwrap();

    let npm = command("npm")
        .args(["install", "--prefer-offline", "--loglevel=error"])
        .current_dir(test_app.path())
        .output()
        .unwrap();

    if !npm.status.success() {
        io::stdout().write_all(&npm.stdout).unwrap();
        io::stderr().write_all(&npm.stderr).unwrap();
        panic!("npm install failed. See above.");
    }

    retry_default((), |_| bundler.prepare(test_app.path())).unwrap();

    test_app
}

pub async fn create_browser() -> Browser {
    let with_head = read_env_bool("TURBOPACK_BENCH_WITH_HEAD");
    let with_devtools = read_env_bool("TURBOPACK_BENCH_DEVTOOLS");
    let mut builder = BrowserConfig::builder();
    if with_head {
        builder = builder.with_head();
    }
    if with_devtools {
        builder = builder.arg("--auto-open-devtools-for-tabs");
    }
    let (browser, mut handler) = retry_async(
        builder.build().unwrap(),
        |c| {
            let c = c.clone();
            Browser::launch(c)
        },
        3,
        Duration::from_millis(100),
    )
    .await
    .expect("Launching the browser failed");

    // See https://crates.io/crates/chromiumoxide
    tokio::task::spawn(async move {
        loop {
            if let Err(Ws(Protocol(ResetWithoutClosingHandshake))) = handler.next().await.unwrap() {
                break;
            }
        }
    });

    browser
}

pub fn resume_on_error<F: FnOnce() + UnwindSafe>(f: F) {
    let runs_as_bench = std::env::args().find(|a| a == "--bench");

    if runs_as_bench.is_some() {
        use std::panic::catch_unwind;
        // panics are already printed to the console, so no need to handle the result.
        let _ = catch_unwind(f);
    } else {
        f();
    }
}

pub trait AsyncBencherExtension {
    /// Benchmarks a faillible routine, with setup, warmup, and teardown stages.
    #[inline(never)]
    fn try_iter_async<I, O, S, SF, W, WF, R, F, T, TF>(
        &mut self,
        setup: S,
        warmup: W,
        routine: R,
        teardown: T,
    ) where
        S: Fn() -> SF,
        SF: Future<Output = Result<I>>,
        W: Fn(I) -> WF,
        WF: Future<Output = Result<I>>,
        R: Fn(I) -> F,
        F: Future<Output = Result<O>>,
        T: Fn(O) -> TF,
        TF: Future<Output = ()>,
    {
        let routine = &routine;
        self.try_iter_async_custom(
            setup,
            warmup,
            |measurement, input| async move {
                let start = measurement.start();
                let output = routine(input).await?;
                Ok((measurement.end(start), output))
            },
            teardown,
        )
    }

    /// Benchmarks a faillible routine, with setup, warmup, and teardown stages.
    ///
    /// It is the routine's responsibility to measure its own execution time.
    fn try_iter_async_custom<I, O, S, SF, W, WF, R, F, T, TF>(
        &mut self,
        setup: S,
        warmup: W,
        routine: R,
        teardown: T,
    ) where
        S: Fn() -> SF,
        SF: Future<Output = Result<I>>,
        W: Fn(I) -> WF,
        WF: Future<Output = Result<I>>,
        R: Fn(WallTime, I) -> F,
        F: Future<Output = Result<(Duration, O)>>,
        T: Fn(O) -> TF,
        TF: Future<Output = ()>;
}

impl<'a, 'b, A: AsyncExecutor> AsyncBencherExtension for AsyncBencher<'a, 'b, A, WallTime> {
    #[inline(never)]
    fn try_iter_async_custom<I, O, S, SF, W, WF, R, F, T, TF>(
        &mut self,
        setup: S,
        warmup: W,
        routine: R,
        teardown: T,
    ) where
        S: Fn() -> SF,
        SF: Future<Output = Result<I>>,
        W: Fn(I) -> WF,
        WF: Future<Output = Result<I>>,
        R: Fn(WallTime, I) -> F,
        F: Future<Output = Result<(Duration, O)>>,
        T: Fn(O) -> TF,
        TF: Future<Output = ()>,
    {
        let config = std::env::var("TURBOPACK_BENCH_BENCH").ok();
        let bench_benchmark_itself = !matches!(
            config.as_deref(),
            None | Some("") | Some("no") | Some("false")
        );
        let log_progress = !matches!(
            std::env::var("TURBOPACK_BENCH_PROGRESS").ok().as_deref(),
            None | Some("") | Some("no") | Some("false")
        );

        let setup = &setup;
        let warmup = &warmup;
        let routine = &routine;
        let teardown = &teardown;
        self.iter_custom(|iters| async move {
            let measurement = WallTime;
            let mut value = measurement.zero();

            let mut iter = 0u64;
            let mut failures = 0u64;
            while iter < iters {
                loop {
                    let setup_start = bench_benchmark_itself.then(|| measurement.start());

                    let input = black_box(
                        retry_async_default((), |_| setup())
                            .await
                            .expect("failed to setup"),
                    );
                    let input = black_box(warmup(input).await).expect("failed to warmup");

                    let setup_duration = setup_start.map(|start| measurement.end(start));

                    match routine(WallTime, input).await {
                        Ok((duration, output)) => {
                            let teardown_start =
                                bench_benchmark_itself.then(|| measurement.start());
                            teardown(black_box(output)).await;
                            let teardown_duration =
                                teardown_start.map(|start| measurement.end(start));

                            if log_progress {
                                eprint!(" {} ", FormatDuration(duration));
                            }
                            value = measurement.add(&value, &duration);

                            if let (Some(teardown_duration), Some(setup_duration)) =
                                (teardown_duration, setup_duration)
                            {
                                value = measurement.add(&value, &setup_duration);
                                value = measurement.add(&value, &teardown_duration);
                            }

                            iter += 1;
                            break;
                        }
                        Err(err) => {
                            failures += 1;
                            if failures > iters {
                                panic!("Routine failed {failures} times, aborting\n{:?}", err)
                            } else {
                                eprintln!("Routine failed, will be retried: {:?}", err);
                                continue;
                            }
                        }
                    }
                }
            }

            value
        })
    }
}

pub fn command(bin: &str) -> Command {
    if cfg!(windows) {
        let mut command = Command::new("cmd.exe");
        command.args(["/C", bin]);
        command
    } else {
        Command::new(bin)
    }
}

pub fn wait_for_match<R>(readable: R, re: Regex) -> Option<String>
where
    R: Read,
{
    // See https://docs.rs/async-process/latest/async_process/#examples
    let mut line_reader = BufReader::new(readable).lines();
    // Read until the match appears in the buffer
    let mut matched: Option<String> = None;
    while let Some(Ok(line)) = line_reader.next() {
        if let Some(cap) = re.captures(&line) {
            matched = Some(cap.get(1).unwrap().as_str().into());
            break;
        }
    }

    matched
}
