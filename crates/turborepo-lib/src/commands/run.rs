use anyhow::Result;
use tracing::{error, info};

use crate::{commands::CommandBase, run::Run};

#[allow(dead_code)]
pub async fn run(base: CommandBase) -> Result<()> {
    info!("Executing run stub");
    let mut run = Run::new(base);
    info!("configured run struct: {:?}", run);

    match run.run().await {
        Ok(_) => Ok(()),
        Err(err) => {
            error!("run failed: {}", err);
            Err(err)
        }
    }
}
