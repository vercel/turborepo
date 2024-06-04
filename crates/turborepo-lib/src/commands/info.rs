//! A command for outputting information about a turborepo.
//! Currently just for internal use (not a public command)
//! Can output in either text or JSON
//! Different than run summary or dry run because it can include
//! sensitive data like your auth token
use std::{fs::File, io::Write};

use turbopath::AbsoluteSystemPathBuf;
use turborepo_repository::{package_graph::PackageGraph, package_json::PackageJson};

use crate::{
    cli::InfoFormat,
    commands::CommandBase,
    info::{Error, RepositoryState},
};

pub async fn run(
    base: &mut CommandBase,
    workspace: Option<&str>,
    format: InfoFormat,
    out: Option<AbsoluteSystemPathBuf>,
) -> Result<(), Error> {
    let root_package_json = PackageJson::load(&base.repo_root.join_component("package.json"))?;
    let package_graph = PackageGraph::builder(&base.repo_root, root_package_json)
        .build()
        .await?;

    let repo_state = RepositoryState::new(package_graph, base.config()?, base.repo_root.clone());
    if matches!(format, InfoFormat::Scip) {
        if workspace.is_some() {
            return Err(Error::ScipForPackage);
        }

        return if let Some(path) = out {
            Ok(repo_state.emit_scip(&path)?)
        } else {
            Err(Error::ScipOutputRequired)
        };
    }

    let mut output: Box<dyn Write> = match out {
        Some(path) => Box::new(File::open(path)?),
        None => Box::new(std::io::stdout()),
    };

    if let Some(workspace) = workspace {
        let package_details = repo_state.as_package_details(workspace);
        match format {
            InfoFormat::Json => writeln!(
                output,
                "{}",
                serde_json::to_string_pretty(&package_details)?
            )?,
            InfoFormat::Text => package_details.print_to(&mut output)?,
            InfoFormat::Scip => unreachable!(),
        }
    } else {
        let repo_details = repo_state.as_details();
        match format {
            InfoFormat::Json => {
                writeln!(output, "{}", serde_json::to_string_pretty(&repo_details)?)?
            }
            InfoFormat::Text => repo_details.print_to(&mut output)?,
            InfoFormat::Scip => unreachable!(),
        }
    }

    Ok(())
}
