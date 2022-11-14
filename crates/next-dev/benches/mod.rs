use std::{
    fs::{self},
    panic::AssertUnwindSafe,
    path::Path,
    time::Duration,
};

use anyhow::{anyhow, bail, Context, Result};
use bundlers::get_bundlers;
use criterion::{
    criterion_group, criterion_main,
    measurement::{Measurement, WallTime},
    BenchmarkGroup, BenchmarkId, Criterion,
};
use once_cell::sync::Lazy;
use tokio::{
    runtime::Runtime,
    time::{sleep, timeout},
};
use util::{
    build_test, create_browser,
    env::{read_env, read_env_bool, read_env_list},
    rand::deterministic_random_pick,
    AsyncBencherExtension, PageGuard, PreparedApp, BINDING_NAME,
};

use self::util::resume_on_error;

mod bundlers;
mod util;

const MAX_UPDATE_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_WARMUP_ATTEMPTS: usize = 30;

fn get_module_counts() -> Vec<usize> {
    read_env_list("TURBOPACK_BENCH_COUNTS", vec![1_000usize]).unwrap()
}

fn bench_startup(c: &mut Criterion) {
    let mut g = c.benchmark_group("bench_startup");
    g.sample_size(10);
    g.measurement_time(Duration::from_secs(60));

    bench_startup_internal(g, false);
}

fn bench_hydration(c: &mut Criterion) {
    let mut g = c.benchmark_group("bench_hydration");
    g.sample_size(10);
    g.measurement_time(Duration::from_secs(60));

    bench_startup_internal(g, true);
}

fn bench_startup_internal(mut g: BenchmarkGroup<WallTime>, hydration: bool) {
    let runtime = Runtime::new().unwrap();
    let browser = &runtime.block_on(create_browser());

    for bundler in get_bundlers() {
        let wait_for_hydration = if !bundler.has_server_rendered_html() {
            // For bundlers without server rendered html "startup" means time to hydration
            // as they only render an empty screen without hydration. Since startup and
            // hydration would be the same we skip the hydration benchmark for them.
            if hydration {
                continue;
            } else {
                true
            }
        } else if !bundler.has_interactivity() {
            // For bundlers without interactivity there is no hydration event to wait for
            if hydration {
                continue;
            } else {
                false
            }
        } else {
            hydration
        };
        for module_count in get_module_counts() {
            let test_app = Lazy::new(|| build_test(module_count, bundler.as_ref()));
            let input = (bundler.as_ref(), &test_app);
            resume_on_error(AssertUnwindSafe(|| {
                g.bench_with_input(
                    BenchmarkId::new(bundler.get_name(), format!("{} modules", module_count)),
                    &input,
                    |b, &(bundler, test_app)| {
                        b.to_async(&runtime).try_iter_async(
                            || async {
                                PreparedApp::new(bundler, test_app.path().to_path_buf()).await
                            },
                            |app| async { Ok(app) },
                            |mut app| async {
                                app.start_server()?;
                                let mut guard = app.with_page(browser).await?;
                                if wait_for_hydration {
                                    guard.wait_for_hydration().await?;
                                }

                                // Defer the dropping of the guard to `teardown`.
                                Ok(guard)
                            },
                            |_guard| async move {},
                        );
                    },
                );
            }));
        }
    }
    g.finish();
}

#[derive(Copy, Clone)]
enum CodeLocation {
    Effect,
    Evaluation,
}

fn bench_hmr_to_eval(c: &mut Criterion) {
    let mut g = c.benchmark_group("bench_hmr_to_eval");
    g.sample_size(10);
    g.measurement_time(Duration::from_secs(60));

    bench_hmr_internal(g, CodeLocation::Evaluation);
}

fn bench_hmr_to_commit(c: &mut Criterion) {
    let mut g = c.benchmark_group("bench_hmr_to_commit");
    g.sample_size(10);
    g.measurement_time(Duration::from_secs(60));

    bench_hmr_internal(g, CodeLocation::Effect);
}

fn bench_hmr_internal(mut g: BenchmarkGroup<WallTime>, location: CodeLocation) {
    let runtime = Runtime::new().unwrap();
    let browser = &runtime.block_on(create_browser());
    let hmr_samples = read_env("TURBOPACK_BENCH_HMR_SAMPLES", 100).unwrap();
    let warmup_samples = read_env("TURBOPACK_BENCH_HMR_WARMUP", 10).unwrap();

    for bundler in get_bundlers() {
        // TODO HMR for RSC is broken, fix it and enable it here
        if !bundler.has_interactivity() {
            continue;
        }
        for module_count in get_module_counts() {
            let test_app = Lazy::new(|| build_test(module_count, bundler.as_ref()));
            let input = (bundler.as_ref(), &test_app);
            resume_on_error(AssertUnwindSafe(|| {
                g.bench_with_input(
                    BenchmarkId::new(bundler.get_name(), format!("{} modules", module_count)),
                    &input,
                    |b, &(bundler, test_app)| {
                        b.to_async(Runtime::new().unwrap()).try_iter_async_custom(
                            || async {
                                let mut app =
                                    PreparedApp::new(bundler, test_app.path().to_path_buf())
                                        .await?;

                                let test_app_path = test_app.path();
                                let modules: Vec<_> = deterministic_random_pick(
                                    test_app
                                        .modules()
                                        .iter()
                                        .map(|module| {
                                            app.path()
                                                .join(module.strip_prefix(test_app_path).unwrap())
                                        })
                                        .collect(),
                                    hmr_samples + warmup_samples,
                                );

                                app.start_server()?;
                                let mut guard = app.with_page(browser).await?;
                                if bundler.has_interactivity() {
                                    guard.wait_for_hydration().await?;
                                } else {
                                    guard.page().wait_for_navigation().await?;
                                }
                                guard
                                    .page()
                                    .evaluate_expression("globalThis.HMR_IS_HAPPENING = true")
                                    .await
                                    .context(
                                        "Unable to evaluate JavaScript in the page for HMR check \
                                         flag",
                                    )?;

                                Ok((modules, guard))
                            },
                            |(modules, guard)| async move { Ok((modules, guard)) },
                            |measurement, (modules, mut guard)| async move {
                                let mut warmup_modules = modules;
                                let mut modules = warmup_modules.split_off(warmup_samples);
                                let mut failed_warmup_attempts = 0usize;
                                while let Some(module) = warmup_modules.pop() {
                                    if failed_warmup_attempts == MAX_WARMUP_ATTEMPTS {
                                        bail!(
                                            "Unable to warmup HMR after {} attempts",
                                            failed_warmup_attempts
                                        );
                                    }
                                    if let Err(err) = make_change(
                                        &module,
                                        &mut guard,
                                        location,
                                        MAX_UPDATE_TIMEOUT,
                                        &measurement,
                                    )
                                    .await
                                    {
                                        eprintln!("warmup failed {:?}, retrying", err);
                                        warmup_modules.push(module);
                                        failed_warmup_attempts += 1;
                                    }
                                }

                                let mut value = measurement.zero();
                                let measurement_count = modules.len() as u32;
                                while let Some(module) = modules.pop() {
                                    let change_duration = make_change(
                                        &module,
                                        &mut guard,
                                        location,
                                        MAX_UPDATE_TIMEOUT,
                                        &measurement,
                                    )
                                    .await?;
                                    value = measurement.add(&value, &change_duration);
                                }
                                // Defer the dropping of the guard to `teardown`.
                                Ok((value / measurement_count, guard))
                            },
                            |guard| async move {
                                let hmr_is_happening = guard
                                    .page()
                                    .evaluate_expression("globalThis.HMR_IS_HAPPENING")
                                    .await
                                    .unwrap();
                                // Make sure that we are really measuring HMR and not accidentically
                                // full refreshing the page
                                assert!(hmr_is_happening.value().unwrap().as_bool().unwrap());
                            },
                        );
                    },
                );
            }));
        }
    }
}

fn insert_code(contents: &mut String, code: &str, location: CodeLocation) -> Result<()> {
    const PRAGMA_EFFECT_START: &str = "/* @turbopack-bench:effect-start */";
    const PRAGMA_EFFECT_END: &str = "/* @turbopack-bench:effect-end */";
    const PRAGMA_EVAL: &str = "/* @turbopack-bench:eval */";
    match location {
        CodeLocation::Effect => {
            let start = contents
                .find(PRAGMA_EFFECT_START)
                .ok_or_else(|| anyhow!("unable to find effect start pragma in {}", contents))?;
            let end = contents
                .find(PRAGMA_EFFECT_END)
                .ok_or_else(|| anyhow!("unable to find effect end pragma in {}", contents))?;
            contents.replace_range(
                start + PRAGMA_EFFECT_START.len()..end,
                &format!("\nEFFECT = () => {{ {code} }};\n"),
            );
        }
        CodeLocation::Evaluation => {
            let a = contents
                .find(PRAGMA_EVAL)
                .ok_or_else(|| anyhow!("unable to find eval pragma in {}", contents))?;
            contents.insert_str(a + PRAGMA_EVAL.len(), &format!("\n{code}"));
        }
    }

    Ok(())
}

static CHANGE_TIMEOUT_MESSAGE: &str = "update was not registered by bundler";

async fn make_change<'a, M>(
    module: &Path,
    guard: &mut PageGuard<'a>,
    location: CodeLocation,
    timeout_duration: Duration,
    measurement: &M,
) -> Result<M::Value>
where
    M: Measurement,
{
    let msg = format!("TURBOPACK_BENCH_CHANGE_{}", guard.app_mut().counter());
    let code = format!(
        "globalThis.{BINDING_NAME} && globalThis.{BINDING_NAME}('{msg}'); console.log('{msg}');"
    );

    // Keep the IO out of the measurement.
    let mut contents = fs::read_to_string(module)?;
    insert_code(&mut contents, &code, location)?;
    fs::write(module, contents)?;

    let start = measurement.start();

    // Wait for the change introduced above to be reflected at runtime.
    // This expects HMR or automatic reloading to occur.
    timeout(timeout_duration, guard.wait_for_binding(&msg))
        .await
        .context(CHANGE_TIMEOUT_MESSAGE)??;

    let value = measurement.end(start);

    Ok(value)
}

fn bench_startup_cached(c: &mut Criterion) {
    let mut g = c.benchmark_group("bench_startup_cached");
    g.sample_size(10);
    g.measurement_time(Duration::from_secs(60));

    bench_startup_cached_internal(g, false);
}

fn bench_hydration_cached(c: &mut Criterion) {
    let mut g = c.benchmark_group("bench_hydration_cached");
    g.sample_size(10);
    g.measurement_time(Duration::from_secs(60));

    bench_startup_cached_internal(g, true);
}

fn bench_startup_cached_internal(mut g: BenchmarkGroup<WallTime>, hydration: bool) {
    if !read_env_bool("TURBOPACK_BENCH_CACHED") {
        return;
    }

    let runtime = Runtime::new().unwrap();
    let browser = &runtime.block_on(create_browser());

    for bundler in get_bundlers() {
        let wait_for_hydration = if !bundler.has_server_rendered_html() {
            // For bundlers without server rendered html "startup" means time to hydration
            // as they only render an empty screen without hydration. Since startup and
            // hydration would be the same we skip the hydration benchmark for them.
            if hydration {
                continue;
            } else {
                true
            }
        } else if !bundler.has_interactivity() {
            // For bundlers without interactivity there is no hydration event to wait for
            if hydration {
                continue;
            } else {
                false
            }
        } else {
            hydration
        };
        for module_count in get_module_counts() {
            let test_app = Lazy::new(|| build_test(module_count, bundler.as_ref()));
            let input = (bundler.as_ref(), &test_app);

            resume_on_error(AssertUnwindSafe(|| {
                g.bench_with_input(
                    BenchmarkId::new(bundler.get_name(), format!("{} modules", module_count)),
                    &input,
                    |b, &(bundler, test_app)| {
                        b.to_async(Runtime::new().unwrap()).try_iter_async(
                            || async {
                                // Run a complete build, shut down, and test running it again
                                let mut app =
                                    PreparedApp::new(bundler, test_app.path().to_path_buf())
                                        .await?;
                                app.start_server()?;
                                let mut guard = app.with_page(browser).await?;
                                if bundler.has_interactivity() {
                                    guard.wait_for_hydration().await?;
                                } else {
                                    guard.page().wait_for_navigation().await?;
                                }

                                let mut app = guard.close_page().await?;

                                // Give it 4 seconds time to store the cache
                                sleep(Duration::from_secs(4)).await;

                                app.stop_server()?;
                                Ok(app)
                            },
                            |app| async { Ok(app) },
                            |mut app| async {
                                app.start_server()?;
                                let mut guard = app.with_page(browser).await?;
                                if wait_for_hydration {
                                    guard.wait_for_hydration().await?;
                                }

                                // Defer the dropping of the guard to `teardown`.
                                Ok(guard)
                            },
                            |_guard| async move {},
                        );
                    },
                );
            }));
        }
    }
}

criterion_group!(
    name = benches;
    config = Criterion::default();
    targets = bench_startup, bench_hydration, bench_startup_cached, bench_hydration_cached, bench_hmr_to_eval, bench_hmr_to_commit
);
criterion_main!(benches);
