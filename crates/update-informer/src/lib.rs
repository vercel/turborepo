#![doc = include_str!("../README.md")]

use std::time::Duration;

pub use package::Package;
pub use registry::Registry;
pub use version::Version;

use crate::{
    http_client::{DefaultHttpClient, HttpClient},
    version_file::VersionFile,
};

mod package;
mod version;
mod version_file;

#[cfg(test)]
mod test_helper;

/// A registry service that stores information about releases.
pub mod registry;

/// An HTTP client to send requests to the registry.
pub mod http_client;

type Error = Box<dyn std::error::Error>;
pub type Result<T> = std::result::Result<T, Error>;

pub trait Check {
    /// Checks for a new version in the registry.
    fn check_version(self) -> Result<Option<Version>>
    where
        Self: Sized,
    {
        Ok(None)
    }
}

/// Checks for a new version on Crates.io, GitHub, Npm and PyPi.
pub struct UpdateInformer<
    R: Registry,
    N: AsRef<str>,
    V: AsRef<str>,
    H: HttpClient = DefaultHttpClient,
> {
    _registry: R,
    name: N,
    version: V,
    http_client: H,
    interval: Duration,
    timeout: Duration,
}

/// Constructs a new `UpdateInformer`.
///
/// # Arguments
///
/// * `registry` - A registry service such as Crates.io or GitHub.
/// * `name` - A project name.
/// * `version` - Current version of the project.
///
/// # Examples
///
/// ```rust
/// use update_informer::{registry, Check};
///
/// let name = env!("CARGO_PKG_NAME");
/// let version = env!("CARGO_PKG_VERSION");
/// let informer = update_informer::new(registry::Crates, name, version);
/// ```
pub fn new<R, N, V>(registry: R, name: N, version: V) -> UpdateInformer<R, N, V>
where
    R: Registry,
    N: AsRef<str>,
    V: AsRef<str>,
{
    UpdateInformer {
        _registry: registry,
        name,
        version,
        http_client: DefaultHttpClient {},
        interval: Duration::from_secs(60 * 60 * 24), // Once a day
        timeout: Duration::from_secs(5),
    }
}

impl<R, N, V, H> UpdateInformer<R, N, V, H>
where
    R: Registry,
    N: AsRef<str>,
    V: AsRef<str>,
    H: HttpClient,
{
    /// Sets an interval how often to check for a new version.
    ///
    /// # Arguments
    ///
    /// * `interval` - An interval in seconds. By default, it is 24 hours.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use std::time::Duration;
    /// use update_informer::{registry, Check};
    ///
    /// const EVERY_HOUR: Duration = Duration::from_secs(60 * 60);
    ///
    /// let informer = update_informer::new(registry::Crates, "crate_name", "0.1.0").interval(EVERY_HOUR);
    /// let _ = informer.check_version(); // The check will start only after an hour
    /// ```
    pub fn interval(self, interval: Duration) -> Self {
        Self { interval, ..self }
    }

    /// Sets a request timeout.
    ///
    /// # Arguments
    ///
    /// * `timeout` - A request timeout. By default, it is 5 seconds.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use std::time::Duration;
    /// use update_informer::{registry, Check};
    ///
    /// const THIRTY_SECONDS: Duration = Duration::from_secs(30);
    ///
    /// let informer = update_informer::new(registry::Crates, "crate_name", "0.1.0").timeout(THIRTY_SECONDS);
    /// let _ = informer.check_version();
    /// ```
    pub fn timeout(self, timeout: Duration) -> Self {
        Self { timeout, ..self }
    }

    /// Sets an HTTP client to send request to the registry.
    ///
    /// # Arguments
    ///
    /// * `http_client` - A type that implements the `HttpClient` trait.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use isahc::ReadResponseExt;
    /// use std::time::Duration;
    /// use serde::de::DeserializeOwned;
    /// use update_informer::{http_client::{HeaderMap, HttpClient}, registry, Check};
    ///
    /// struct YourOwnHttpClient;
    ///
    /// impl HttpClient for YourOwnHttpClient {
    ///     fn get<T: DeserializeOwned>(
    ///         url: &str,
    ///         _timeout: Duration,
    ///         _headers: HeaderMap,
    ///     ) -> update_informer::Result<T> {
    ///         let json = isahc::get(url)?.json()?;
    ///         Ok(json)
    ///     }
    /// }
    ///
    /// let informer = update_informer::new(registry::Crates, "crate_name", "0.1.0").http_client(YourOwnHttpClient);
    /// let _ = informer.check_version();
    /// ```
    pub fn http_client<C: HttpClient>(self, http_client: C) -> UpdateInformer<R, N, V, C> {
        UpdateInformer {
            _registry: self._registry,
            name: self.name,
            version: self.version,
            interval: self.interval,
            timeout: self.timeout,
            http_client,
        }
    }
}

impl<R, N, V, H> Check for UpdateInformer<R, N, V, H>
where
    R: Registry,
    N: AsRef<str>,
    V: AsRef<str>,
    H: HttpClient,
{
    /// Checks for a new version in the registry.
    ///
    /// # Examples
    ///
    /// To check for a new version on Crates.io:
    ///
    /// ```rust
    /// use update_informer::{registry, Check};
    ///
    /// let informer = update_informer::new(registry::Crates, "crate_name", "0.1.0");
    /// let _ = informer.check_version();
    /// ```
    fn check_version(self) -> Result<Option<Version>> {
        let pkg = Package::new(self.name.as_ref(), self.version.as_ref())?;
        let client = http_client::new(self.http_client, self.timeout);

        // If the interval is zero, don't use the cache file
        let latest_version = if self.interval.is_zero() {
            match R::get_latest_version(client, &pkg)? {
                Some(v) => v,
                None => return Ok(None),
            }
        } else {
            let latest_version_file = VersionFile::new(R::NAME, &pkg, self.version.as_ref())?;
            let last_modified = latest_version_file.last_modified()?;

            if last_modified >= self.interval {
                // This is needed to update mtime of the file
                latest_version_file.recreate_file()?;

                match R::get_latest_version(client, &pkg)? {
                    Some(v) => {
                        latest_version_file.write_version(&v)?;
                        v
                    }
                    None => return Ok(None),
                }
            } else {
                latest_version_file.get_version()?
            }
        };

        let latest_version = Version::parse(latest_version)?;
        if &latest_version > pkg.version() {
            return Ok(Some(latest_version));
        }

        Ok(None)
    }
}

/// Fake `UpdateInformer`. Used only for tests.
pub struct FakeUpdateInformer<V: AsRef<str>> {
    version: V,
}

/// Constructs a new `FakeUpdateInformer`.
///
/// # Arguments
///
/// * `registry` - A registry service such as Crates.io or GitHub (not used).
/// * `name` - A project name (not used).
/// * `version` - Current version of the project (not used).
/// * `interval` - An interval how often to check for a new version (not used).
/// * `new_version` - The desired version.
///
/// # Examples
///
/// ```rust
/// use update_informer::{registry, Check};
///
/// let informer = update_informer::fake(registry::Crates, "repo", "0.1.0", "1.0.0");
/// ```
pub fn fake<R, N, V>(_registry: R, _name: N, _version: V, new_version: V) -> FakeUpdateInformer<V>
where
    R: Registry,
    N: AsRef<str>,
    V: AsRef<str>,
{
    FakeUpdateInformer {
        version: new_version,
    }
}

impl<V: AsRef<str>> FakeUpdateInformer<V> {
    pub fn interval(self, _interval: Duration) -> Self {
        self
    }

    pub fn timeout(self, _timeout: Duration) -> Self {
        self
    }

    pub fn http_client<C: HttpClient>(self, _http_client: C) -> Self {
        self
    }
}

impl<V: AsRef<str>> Check for FakeUpdateInformer<V> {
    /// Returns the desired version as a new version.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use update_informer::{registry, Check};
    ///
    /// let informer = update_informer::fake(registry::Crates, "crate_name", "0.1.0", "1.0.0");
    /// let result = informer.check_version();
    /// assert!(result.is_ok());
    ///
    /// let version = result.unwrap();
    /// assert!(version.is_some());
    /// assert_eq!(version.unwrap().to_string(), "v1.0.0");
    /// ```
    fn check_version(self) -> Result<Option<Version>> {
        let version = Version::parse(self.version.as_ref())?;

        Ok(Some(version))
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use mockito::Mock;

    use super::*;
    use crate::{registry::Crates, test_helper::within_test_dir};

    const PKG_NAME: &str = "repo";
    const CURRENT_VERSION: &str = "3.1.0";
    const LATEST_VERSION: &str = "3.1.1";

    fn mock_crates(pkg: &str) -> Mock {
        let pkg = Package::new(pkg, CURRENT_VERSION).unwrap();
        let (mock, _) = crate::test_helper::mock_crates(
            &pkg,
            200,
            "tests/fixtures/registry/crates/versions.json",
        );

        mock
    }

    #[test]
    fn no_new_version_with_interval_test() {
        within_test_dir(|_| {
            let informer = new(Crates, PKG_NAME, CURRENT_VERSION);
            let result = informer.check_version();

            assert!(result.is_ok());
            assert_eq!(result.unwrap(), None);
        });
    }

    #[test]
    fn no_new_version_on_registry_test() {
        within_test_dir(|_| {
            let _mock = mock_crates(PKG_NAME);
            let informer = new(Crates, PKG_NAME, LATEST_VERSION).interval(Duration::ZERO);
            let result = informer.check_version();

            assert!(result.is_ok());
            assert_eq!(result.unwrap(), None);
        });
    }

    #[test]
    fn check_version_on_crates_test() {
        within_test_dir(|_| {
            let _mock = mock_crates(PKG_NAME);
            let informer = new(Crates, PKG_NAME, CURRENT_VERSION).interval(Duration::ZERO);
            let result = informer.check_version();
            let version = Version::parse(LATEST_VERSION).expect("parse version");

            assert!(result.is_ok());
            assert_eq!(result.unwrap(), Some(version));
        });
    }

    #[test]
    fn return_version_from_file_test() {
        within_test_dir(|version_file| {
            fs::write(version_file, "4.0.0").expect("create file");

            let informer = new(Crates, PKG_NAME, CURRENT_VERSION);
            let result = informer.check_version();
            let version = Version::parse("4.0.0").expect("parse version");

            assert!(result.is_ok());
            assert_eq!(result.unwrap(), Some(version));
        });
    }

    #[test]
    fn create_version_file_test() {
        within_test_dir(|version_file| {
            assert!(!version_file.exists());

            let informer = new(Crates, PKG_NAME, CURRENT_VERSION);
            let result = informer.check_version();
            assert!(result.is_ok());
            assert!(version_file.exists());

            let version = fs::read_to_string(version_file).expect("read file");
            assert_eq!(version, CURRENT_VERSION);
        });
    }

    #[test]
    fn do_not_create_version_file_test() {
        within_test_dir(|version_file| {
            assert!(!version_file.exists());

            let _mock = mock_crates(PKG_NAME);
            let informer = new(Crates, PKG_NAME, CURRENT_VERSION).interval(Duration::ZERO);
            let result = informer.check_version();

            assert!(result.is_ok());
            assert!(!version_file.exists());
        });
    }

    #[test]
    fn check_version_with_string_name_test() {
        within_test_dir(|_| {
            let pkg_name = format!("{}/{}", "owner", PKG_NAME);
            let informer = new(Crates, pkg_name, CURRENT_VERSION);
            let result = informer.check_version();

            assert!(result.is_ok());
        });
    }

    #[test]
    fn check_version_with_string_version_test() {
        within_test_dir(|_| {
            let version = String::from(CURRENT_VERSION);
            let informer = new(Crates, PKG_NAME, version);
            let result = informer.check_version();

            assert!(result.is_ok());
        });
    }

    #[test]
    fn check_version_with_amp_string_test() {
        within_test_dir(|_| {
            let pkg_name = format!("{}/{}", "owner", PKG_NAME);
            let version = String::from(CURRENT_VERSION);
            let informer = new(Crates, &pkg_name, &version);
            let result = informer.check_version();

            assert!(result.is_ok());
        });
    }

    #[test]
    fn fake_check_version_test() {
        let version = "1.0.0";
        let informer = fake(Crates, PKG_NAME, CURRENT_VERSION, version)
            .interval(Duration::ZERO)
            .timeout(Duration::ZERO);
        let result = informer.check_version();
        let version = Version::parse(version).expect("parse version");

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), Some(version));
    }
}
