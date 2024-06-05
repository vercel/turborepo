use crate::{
    http_client::{GenericHttpClient, HttpClient},
    Package, Result,
};

#[cfg(feature = "crates")]
mod crates;
#[cfg(feature = "crates")]
pub use crates::Crates;

#[cfg(feature = "github")]
mod github;
#[cfg(feature = "github")]
pub use github::GitHub;

#[cfg(feature = "npm")]
mod npm;
#[cfg(feature = "npm")]
pub use npm::Npm;

#[cfg(feature = "pypi")]
mod pypi;

#[cfg(feature = "pypi")]
pub use pypi::PyPI;

pub trait Registry {
    /// The name of the registry.
    const NAME: &'static str;

    /// Gets the latest version of a package from the registry.
    ///
    /// # Arguments
    ///
    /// * `http_client` - An HTTP client to send requests to the registry.
    /// * `pkg` - A `Package` struct.
    /// * `current_version` - The current version of the package.
    fn get_latest_version<T: HttpClient>(
        http_client: GenericHttpClient<T>,
        pkg: &Package,
    ) -> Result<Option<String>>;
}
