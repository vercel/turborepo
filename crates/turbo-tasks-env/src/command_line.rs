use crate::{env_snapshot, EnvMapVc, ProcessEnv, ProcessEnvVc, GLOBAL_ENV_LOCK};

/// Load the environment variables defined via command line.
#[turbo_tasks::value]
pub struct CommandLineProcessEnv;

#[turbo_tasks::value_impl]
impl CommandLineProcessEnvVc {
    #[turbo_tasks::function]
    pub fn new() -> Self {
        CommandLineProcessEnv.cell()
    }
}

#[turbo_tasks::value_impl]
impl ProcessEnv for CommandLineProcessEnv {
    #[turbo_tasks::function]
    fn read_all(&self) -> EnvMapVc {
        let lock = GLOBAL_ENV_LOCK.lock().unwrap();
        EnvMapVc::cell(env_snapshot(&lock))
    }
}
