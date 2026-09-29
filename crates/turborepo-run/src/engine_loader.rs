use turborepo_engine::BuilderError;
use turborepo_microfrontends_config::UnifiedTurboJsonLoader;
use turborepo_repository::package_graph::PackageName;
use turborepo_turbo_json::TurboJson;

/// Adapts the shared turbo.json loader to the engine's error boundary.
///
/// The newtype is local to turborepo-run because both the loader type and the
/// engine trait are defined in other crates.
pub struct EngineTurboJsonLoader<'a>(&'a UnifiedTurboJsonLoader);

impl<'a> EngineTurboJsonLoader<'a> {
    pub fn new(loader: &'a UnifiedTurboJsonLoader) -> Self {
        Self(loader)
    }

    pub fn load(&self, package: &PackageName) -> Result<&'a TurboJson, BuilderError> {
        self.0
            .load(package)
            .map_err(|error| BuilderError::from(turborepo_config::Error::from(error)))
    }
}

impl turborepo_engine::TurboJsonLoader for EngineTurboJsonLoader<'_> {
    fn load(&self, package: &PackageName) -> Result<&TurboJson, BuilderError> {
        EngineTurboJsonLoader::load(self, package)
    }
}
