use anyhow::Result;
use tracing::error;
use turbopath::AbsoluteSystemPathBuf;

use crate::{
    commands::CommandBase, manager::Manager, opts::Opts, package_json::PackageJson, run::Run,
};

#[tokio::main]
async fn run(base: &mut CommandBase) -> Result<()> {
    // equivalent of optsFromArgs
    let opts: Opts = (&base.args).try_into()?;

    // equivalent of configureRun
    let mut run = Run::new(base, opts);

    match run.run().await {
        Ok(_) => Ok(()),
        Err(err) => {
            error!("run failed: {}", err);
            Err(err)
        }
    }
}
