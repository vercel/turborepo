use tracing::{debug, error};

use crate::{commands::CommandBase, run, run::Run};

pub async fn run(base: CommandBase) -> Result<i32, run::Error> {
    let mut run = Run::new(&base);
    debug!("using the experimental rust codepath");
    debug!("configured run struct: {:?}", run);

    match run.run().await {
        Ok(code) => Ok(code),
        Err(err) => {
            error!("run failed: {}", err);
            Err(err)
        }
    }
}
