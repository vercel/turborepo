use thiserror::Error;
use turborepo_ci::is_ci;
use turborepo_scm::{clone::CloneMode, SCM};
use turborepo_telemetry::events::command::CommandEventBuilder;

use crate::commands::CommandBase;

#[derive(Debug, Error)]
pub enum Error {
    #[error("failed to find git binary")]
    GitBinaryNotFound,

    #[error("failed to clone repository")]
    SCM(#[from] turborepo_scm::Error),
}

pub fn run(
    base: CommandBase,
    _telemetry: CommandEventBuilder,
    url: &str,
    dir: Option<&str>,
    ci: bool,
    local: bool,
    depth: Option<usize>,
) -> Result<i32, Error> {
    let clone_mode = if ci {
        CloneMode::CI
    } else if local {
        CloneMode::Local
    } else if is_ci() {
        CloneMode::CI
    } else {
        CloneMode::Local
    };

    let SCM::Git(git) = SCM::new(&base.repo_root) else {
        return Err(Error::GitBinaryNotFound);
    };

    git.clone(url, dir, None, clone_mode, depth)?;

    Ok(0)
}
