use std::fs;

use criterion::{criterion_group, criterion_main, Criterion};
use tokio::runtime::Runtime;
use turbopath::AbsoluteSystemPathBuf;
use turborepo_lib::{run_command, Args, Command, CommandBase, RunArgs};
use turborepo_ui::UI;

async fn execute_run() -> Result<(), anyhow::Error> {
    let mut cli_args = Args::default();
    let mut run_args = RunArgs::default();
    run_args.tasks = vec!["build".to_string()];
    run_args.no_daemon = true;
    cli_args.command = Some(Command::Run(Box::new(run_args)));
    let repo_root = AbsoluteSystemPathBuf::try_from(fs::canonicalize("../..").unwrap()).unwrap();
    let base = CommandBase::new(cli_args.clone(), repo_root, "1.0", UI::new(true))?;
    match run_command::run(base).await {
        Err(err) => {
            println!("run failed: {}", err);
            Err(err)
        }
        Ok(_) => Ok(()),
    }
}

pub fn criterion_benchmark(c: &mut Criterion) {
    let runtime = Runtime::new().unwrap();
    c.bench_function("run", |b| b.to_async(&runtime).iter(|| execute_run()));
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
