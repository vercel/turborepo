use std::fs;

use scip::types::{
    symbol_information::Kind, Document, Index, Metadata, SymbolInformation, TextEncoding, ToolInfo,
};
use thiserror::Error;
use turbopath::AbsoluteSystemPath;
use turborepo_repository::package_graph::PackageNode;

use crate::{get_version, run::Run};

#[derive(Debug, Error)]
pub enum Error {
    #[error("Failed to write SCIP message to file: {0}")]
    ScipWriteError(String),
    #[error(transparent)]
    Run(#[from] crate::run::error::Error),
}

impl Run {
    pub fn emit_scip(&self, path: &AbsoluteSystemPath) -> Result<(), Error> {
        let index: Index = self.into();

        scip::write_message_to_file(path, index).map_err(|e| Error::ScipWriteError(e.to_string()))
    }
}

impl<'a> Into<Index> for &'a Run {
    fn into(self) -> Index {
        let documents = self
            .pkg_dep_graph
            .packages()
            .map(|(pkg_name, pkg)| {
                let symbols = self
                    .pkg_dep_graph
                    .immediate_dependencies(&PackageNode::Workspace(pkg_name.clone()))
                    .iter()
                    .flatten()
                    .map(|dep| SymbolInformation {
                        // This follows a specific grammar. We can look into defining
                        // a turbo format later: https://sourcegraph.com/github.com/sourcegraph/scip@6495bfbd33671ccd4a2358505fdf30058140ff32/-/blob/scip.proto?L147
                        symbol: format!(
                            "{} {} {} {} {}/",
                            // Scheme
                            dep.as_package_name(),
                            // Manager
                            self.pkg_dep_graph.package_manager(),
                            // Package Name
                            dep.as_package_name(),
                            // Version
                            "*",
                            dep.as_package_name()
                        ),
                        documentation: vec![],
                        relationships: vec![],
                        kind: Kind::Package.into(),
                        display_name: dep.as_package_name().to_string(),
                        signature_documentation: Default::default(),
                        enclosing_symbol: "".to_string(),
                        special_fields: Default::default(),
                    })
                    .collect();

                Document {
                    language: "json".to_string(),
                    relative_path: pkg.package_json_path.to_string(),
                    occurrences: vec![],
                    symbols,
                    text: fs::read_to_string(&pkg.package_json_path).unwrap_or_default(),
                    position_encoding: Default::default(),
                    special_fields: Default::default(),
                }
            })
            .collect();

        Index {
            metadata: Some(Metadata {
                version: Default::default(),
                tool_info: Some(ToolInfo {
                    name: "turborepo".to_string(),
                    version: get_version().to_string(),
                    arguments: vec![],
                    special_fields: Default::default(),
                })
                .into(),
                project_root: format!("file://{}", self.repo_root),
                text_document_encoding: TextEncoding::UTF8.into(),
                special_fields: Default::default(),
            })
            .into(),
            documents,
            external_symbols: vec![],
            special_fields: Default::default(),
        }
    }
}
