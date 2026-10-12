//! Credential-free download mirrors. For each mirror, an environment variable
//! overrides the root `setup.mirrors` policy, which overrides the official
//! source. A mirror replaces only the transport base: `turbo.lock` keeps
//! canonical provenance URLs and locked bytes are still verified against lock
//! digests. Values are never echoed in errors because a mistyped value can
//! contain a secret.

use std::ffi::OsString;

use turborepo_download::{ApprovedOrigin, DownloadClient};
use turborepo_turbo_json::{SetupMirrors, setup::mirror_url};

/// Equivalent to `https://nodejs.org/dist`.
pub const NODE_MIRROR_ENV: &str = "TURBO_SETUP_NODE_MIRROR";
/// Equivalent to `https://registry.npmjs.org`.
pub const NPM_REGISTRY_ENV: &str = "TURBO_SETUP_NPM_REGISTRY";
const NODE_POLICY: &str = "setup.mirrors.node";
const NPM_REGISTRY_POLICY: &str = "setup.mirrors.npmRegistry";

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
    /// This process's environment over the repository's `setup.mirrors`.
    pub fn configured(policy: &SetupMirrors) -> Result<Self, Error> {
        Self::resolve(|name| std::env::var_os(name), policy)
    }

    /// Environment over policy over official, separately for each mirror.
    /// Empty variables are unset, like most mirror variables in the
    /// ecosystem, so they fall through to the policy.
    pub fn resolve(
        var: impl Fn(&str) -> Option<OsString>,
        policy: &SetupMirrors,
    ) -> Result<Self, Error> {
        let select =
            |env: &'static str, key: &'static str, committed: &Option<String>| match var(env)
                .filter(|value| !value.is_empty())
            {
                Some(value) => Mirror::parse(env, value, false).map(Some),
                None => committed
                    .as_ref()
                    .map(|value| Mirror::parse(key, value.into(), false))
                    .transpose(),
            };
        Ok(Self {
            node: select(NODE_MIRROR_ENV, NODE_POLICY, &policy.node)?,
            registry: select(NPM_REGISTRY_ENV, NPM_REGISTRY_POLICY, &policy.npm_registry)?,
        })
    }
}

impl Mirror {
    fn parse(name: &'static str, value: OsString, loopback: bool) -> Result<Self, Error> {
        let invalid = || Error(name);
        let value = value.into_string().map_err(|_| invalid())?;
        // The same rule validates committed `setup.mirrors` values.
        let url = mirror_url(&value).ok_or_else(invalid)?;
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

    fn env(node: &str) -> impl Fn(&str) -> Option<OsString> {
        let node = node.to_owned();
        move |name| (name == NODE_MIRROR_ENV).then(|| node.clone().into())
    }

    fn policy(node: &str, registry: &str) -> SetupMirrors {
        SetupMirrors {
            node: Some(node.into()),
            npm_registry: Some(registry.into()),
        }
    }

    fn base(mirror: Option<Mirror>) -> Option<String> {
        mirror.map(|mirror| mirror.base)
    }

    #[test]
    fn bases_keep_path_prefixes_and_drop_trailing_slashes() {
        let none = SetupMirrors::default();
        let mirrors = Mirrors::resolve(env("https://mirror.example:8443/nodejs/dist/"), &none);
        let node = mirrors.unwrap().node.unwrap();
        assert_eq!(node.base(), "https://mirror.example:8443/nodejs/dist");
        assert_eq!(node.origin, "https://mirror.example:8443");
        assert_eq!(
            Mirrors::resolve(env(""), &none).unwrap(),
            Mirrors::default()
        );
    }

    #[test]
    fn environment_overrides_policy_for_each_mirror_separately() {
        let committed = policy("https://policy.example/node", "https://policy.example/npm/");
        let mirrors = Mirrors::resolve(env("https://env.example/dist"), &committed).unwrap();
        assert_eq!(
            base(mirrors.node).as_deref(),
            Some("https://env.example/dist")
        );
        assert_eq!(
            base(mirrors.registry).as_deref(),
            Some("https://policy.example/npm")
        );
        // An empty variable is unset, so the policy still applies.
        let mirrors = Mirrors::resolve(env(""), &committed).unwrap();
        assert_eq!(
            base(mirrors.node).as_deref(),
            Some("https://policy.example/node")
        );
        // Explicit official bases opt out of a policy mirror for one run.
        let official = Mirrors::resolve(env("https://nodejs.org/dist"), &committed).unwrap();
        assert_eq!(
            base(official.node).as_deref(),
            Some("https://nodejs.org/dist")
        );
    }

    #[test]
    fn invalid_values_name_their_channel_without_echo() {
        let committed = policy(
            "https://policy.example/node",
            "https://user:s3cret@npm.example",
        );
        let error = Mirrors::resolve(env(""), &committed).unwrap_err();
        assert_eq!(error, Error(NPM_REGISTRY_POLICY));
        // A valid policy never masks an invalid override.
        let error = Mirrors::resolve(env("http://s3cret.example"), &committed).unwrap_err();
        assert_eq!(error, Error(NODE_MIRROR_ENV));
        assert!(!error.to_string().contains("s3cret"));
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
            let none = SetupMirrors::default();
            let error = Mirrors::resolve(env(value), &none).unwrap_err().to_string();
            assert!(error.contains(NODE_MIRROR_ENV), "{error}");
            assert!(!error.contains("s3cret") && !error.contains("mirror.example"));
        }
    }
}
