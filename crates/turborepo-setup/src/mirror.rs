//! Credential-free download mirrors from the environment. A mirror replaces
//! only the transport base: `turbo.lock` keeps canonical provenance URLs and
//! locked bytes are still verified against lock digests. Values are never
//! echoed in errors because a mistyped value can contain a secret.
//! Repository-configured mirrors wait for the root `setup` policy.

use std::ffi::OsString;

use turborepo_download::{ApprovedOrigin, DownloadClient};

/// Equivalent to `https://nodejs.org/dist`.
pub const NODE_MIRROR_ENV: &str = "TURBO_SETUP_NODE_MIRROR";
/// Equivalent to `https://registry.npmjs.org`.
pub const NPM_REGISTRY_ENV: &str = "TURBO_SETUP_NPM_REGISTRY";

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error(
    "{0} must be an absolute HTTPS URL without credentials, a query, or a fragment; the value is \
     not shown because it may contain a secret"
)]
pub struct Error(&'static str);

/// A validated mirror base: one approved origin plus an optional path prefix.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mirror {
    origin: String,
    base: String,
    #[cfg(any(test, feature = "test-support"))]
    loopback: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Mirrors {
    pub node: Option<Mirror>,
    pub registry: Option<Mirror>,
}

impl Mirrors {
    pub fn from_env() -> Result<Self, Error> {
        Self::from_vars(|name| std::env::var_os(name))
    }

    /// Empty values are unset, like most mirror variables in the ecosystem.
    pub fn from_vars(var: impl Fn(&str) -> Option<OsString>) -> Result<Self, Error> {
        let read = |name| {
            var(name)
                .filter(|value| !value.is_empty())
                .map(|value| Mirror::parse(name, value, false))
                .transpose()
        };
        Ok(Self {
            node: read(NODE_MIRROR_ENV)?,
            registry: read(NPM_REGISTRY_ENV)?,
        })
    }
}

impl Mirror {
    fn parse(name: &'static str, value: OsString, loopback: bool) -> Result<Self, Error> {
        let invalid = || Error(name);
        let value = value.into_string().map_err(|_| invalid())?;
        let url = url::Url::parse(&value).map_err(|_| invalid())?;
        // Inspect the raw authority too: URL normalization erases empty
        // userinfo such as `https://@host`.
        let authority = value
            .split_once("://")
            .and_then(|(_, rest)| rest.split(['/', '?', '#']).next())
            .ok_or_else(invalid)?;
        if authority.is_empty()
            || authority.contains('@')
            || value.contains(['?', '#'])
            || url.host().is_none()
        {
            return Err(invalid());
        }
        let origin = url.origin().ascii_serialization();
        let approved = if loopback {
            ApprovedOrigin::loopback_http_for_tests(&origin)
        } else {
            ApprovedOrigin::https(&origin)
        };
        approved.map_err(|_| invalid())?;
        let base = format!("{origin}{}", url.path().trim_end_matches('/'));
        Ok(Self {
            origin,
            base,
            #[cfg(any(test, feature = "test-support"))]
            loopback,
        })
    }

    /// Literal-loopback HTTP fixture mirror; never reachable from production.
    #[cfg(any(test, feature = "test-support"))]
    pub fn loopback_for_tests(base: &str) -> Result<Self, Error> {
        Self::parse("fixture mirror", base.into(), true)
    }

    pub(crate) fn base(&self) -> &str {
        &self.base
    }

    pub(crate) fn client(&self) -> Result<DownloadClient, turborepo_download::Error> {
        #[cfg(any(test, feature = "test-support"))]
        if self.loopback {
            return DownloadClient::loopback_http_for_tests(&self.origin);
        }
        DownloadClient::new([ApprovedOrigin::https(&self.origin)?])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(node: &str) -> Result<Mirrors, Error> {
        Mirrors::from_vars(|name| (name == NODE_MIRROR_ENV).then(|| node.into()))
    }

    #[test]
    fn bases_keep_path_prefixes_and_drop_trailing_slashes() {
        let mirrors = parse("https://mirror.example:8443/nodejs/dist/").unwrap();
        let node = mirrors.node.unwrap();
        assert_eq!(node.base(), "https://mirror.example:8443/nodejs/dist");
        assert_eq!(node.origin, "https://mirror.example:8443");
        assert!(mirrors.registry.is_none());
        assert_eq!(parse("").unwrap(), Mirrors::default());
    }

    #[test]
    fn credentials_queries_fragments_and_plain_http_are_rejected_without_echo() {
        for value in [
            "https://user:s3cret@mirror.example/dist",
            "https://s3cret@mirror.example/dist",
            "https://@mirror.example/dist",
            "https://mirror.example/dist?token=s3cret",
            "https://mirror.example/dist#s3cret",
            "http://mirror.example/s3cret",
            "http://127.0.0.1:9/s3cret",
            "mirror.example/s3cret",
        ] {
            let error = parse(value).unwrap_err().to_string();
            assert!(error.contains(NODE_MIRROR_ENV), "{error}");
            assert!(!error.contains("s3cret") && !error.contains("mirror.example"));
        }
    }
}
