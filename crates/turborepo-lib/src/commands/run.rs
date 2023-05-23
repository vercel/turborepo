use anyhow::Result;
use tracing::{error, info};

use crate::{commands::CommandBase, opts::Opts, run::Run};

pub async fn run(base: &mut CommandBase) -> Result<()> {
    info!("Executing run stub");
    // equivalent of optsFromArgs
    let opts: Opts = (&base.args).try_into()?;
    info!("generated opts struct: {:?}", opts);
    // equivalent of configureRun
    let mut run = Run::new(base, opts);
    info!("configured run struct: {:?}", run);
    let targets = base.args.get_tasks();
    info!("tasks are {:?}", targets);

    match run.run(targets).await {
        Ok(_) => Ok(()),
        Err(err) => {
            error!("run failed: {}", err);
            Err(err)
        }
    }
}
