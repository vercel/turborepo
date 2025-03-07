use std::env::current_dir;

use camino::Utf8Path;
use thiserror::Error;
use turbopath::AbsoluteSystemPathBuf;
use turborepo_ci::is_ci;
use turborepo_scm::clone::{CloneMode, Git};

#[derive(Debug, Error)]
pub enum Error {
    #[error("path is not valid UTF-8")]
    Path(#[from] camino::FromPathBufError),

    #[error(transparent)]
    Turbopath(#[from] turbopath::PathError),

    #[error("failed to clone repository")]
    Scm(#[from] turborepo_scm::Error),
}

pub fn run(
    cwd: Option<&Utf8Path>,
    url: &str,
    dir: Option<&str>,
    ci: bool,
    local: bool,
    depth: Option<usize>,
) -> Result<i32, Error> {
    // We do *not* want to use the repo root but the actual, literal cwd for clone
    let cwd = if let Some(cwd) = cwd {
        cwd.to_owned()
    } else {
        current_dir()
            .expect("could not get current directory")
            .try_into()?
    };
    let abs_cwd = AbsoluteSystemPathBuf::from_cwd(cwd)?;

    let clone_mode = if ci {
        CloneMode::CI
    } else if local {
        CloneMode::Local
    } else if is_ci() {
        CloneMode::CI
    } else {
        CloneMode::Local
    };

    let git = Git::find()?;
    git.clone(url, abs_cwd, dir, None, clone_mode, depth)?;

    Ok(0)
}
