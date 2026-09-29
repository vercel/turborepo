use std::io::ErrorKind;

use turbopath::AbsoluteSystemPath;
use turborepo_repository::{
    package_graph::PackageGraphBuilder,
    package_json::{self, PackageJson},
};

#[derive(Debug, Clone, Copy)]
pub struct RepositoryGraphFeatures {
    pub(crate) cargo: bool,
    pub(crate) python: bool,
    pub(crate) go: bool,
}

impl RepositoryGraphFeatures {
    pub fn new(future_flags: &turborepo_turbo_json::FutureFlags) -> Self {
        Self {
            cargo: future_flags.experimental_cargo_workspaces,
            python: future_flags.experimental_python_workspaces,
            go: future_flags.experimental_go_workspaces,
        }
    }

    pub fn cargo_enabled(self) -> bool {
        self.cargo
    }

    pub fn python_enabled(self) -> bool {
        self.python
    }

    pub fn go_enabled(self) -> bool {
        self.go
    }

    pub fn load_root_package_json(
        self,
        repo_root: &AbsoluteSystemPath,
    ) -> Result<Option<PackageJson>, package_json::Error> {
        match PackageJson::load(&repo_root.join_component("package.json")) {
            Ok(package_json) => Ok(Some(package_json)),
            Err(package_json::Error::Io(io))
                if io.kind() == ErrorKind::NotFound
                    && ((self.cargo
                        && repo_root
                            .join_component(turborepo_repository::cargo::CARGO_TOML)
                            .exists())
                        || (self.python
                            && repo_root
                                .join_component(turborepo_repository::uv::PYPROJECT_TOML)
                                .exists())
                        || (self.go
                            && repo_root
                                .join_component(turborepo_repository::go::GO_WORK)
                                .exists())) =>
            {
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }

    pub fn configure<'a, T>(
        self,
        mut builder: PackageGraphBuilder<'a, T>,
    ) -> PackageGraphBuilder<'a, T> {
        if self.cargo {
            builder = builder.with_cargo();
        }
        if self.python {
            builder = builder.with_uv();
        }
        if self.go {
            builder = builder.with_go();
        }
        builder
    }
}
