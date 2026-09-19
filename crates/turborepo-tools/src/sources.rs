//! Where each tool is downloaded from.
//!
//! Every upstream has an override so air-gapped or mirrored environments can
//! point `turbo setup` at their own copy. Overrides are plain base URLs; the
//! path layout below each is the upstream's.

use std::env;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sources {
    /// Node.js distribution root, e.g. `https://nodejs.org/dist`.
    pub node_dist: String,
    /// npm registry used for npm, pnpm, yarn, and bun version lookups.
    pub npm_registry: String,
    /// bun GitHub release download root.
    pub bun_releases: String,
    /// rustup update root (`RUSTUP_UPDATE_ROOT`).
    pub rustup_update_root: String,
    /// uv GitHub release download root.
    pub uv_releases: String,
    /// PyPI JSON API root used to list uv releases.
    pub pypi: String,
    /// Go download root, e.g. `https://go.dev/dl`.
    pub go_dist: String,
}

pub const NODE_MIRROR_ENV: &str = "TURBO_TOOLS_NODE_MIRROR";
pub const NPM_REGISTRY_ENV: &str = "TURBO_TOOLS_NPM_REGISTRY";
pub const BUN_MIRROR_ENV: &str = "TURBO_TOOLS_BUN_MIRROR";
pub const RUSTUP_UPDATE_ROOT_ENV: &str = "RUSTUP_UPDATE_ROOT";
pub const UV_MIRROR_ENV: &str = "TURBO_TOOLS_UV_MIRROR";
pub const PYPI_URL_ENV: &str = "TURBO_TOOLS_PYPI_URL";
pub const GO_MIRROR_ENV: &str = "TURBO_TOOLS_GO_MIRROR";

impl Default for Sources {
    fn default() -> Self {
        Self {
            node_dist: "https://nodejs.org/dist".into(),
            npm_registry: "https://registry.npmjs.org".into(),
            bun_releases: "https://github.com/oven-sh/bun/releases/download".into(),
            rustup_update_root: "https://static.rust-lang.org/rustup".into(),
            uv_releases: "https://github.com/astral-sh/uv/releases/download".into(),
            pypi: "https://pypi.org/pypi".into(),
            go_dist: "https://go.dev/dl".into(),
        }
    }
}

impl Sources {
    /// Defaults with environment overrides applied.
    pub fn from_env() -> Self {
        let mut sources = Self::default();
        let override_from = |target: &mut String, keys: &[&str]| {
            for key in keys {
                if let Some(value) = env::var(key).ok().filter(|value| !value.trim().is_empty()) {
                    *target = value.trim().trim_end_matches('/').to_string();
                    return;
                }
            }
        };
        override_from(&mut sources.node_dist, &[NODE_MIRROR_ENV]);
        override_from(
            &mut sources.npm_registry,
            &[
                NPM_REGISTRY_ENV,
                "npm_config_registry",
                "NPM_CONFIG_REGISTRY",
            ],
        );
        override_from(&mut sources.bun_releases, &[BUN_MIRROR_ENV]);
        override_from(&mut sources.rustup_update_root, &[RUSTUP_UPDATE_ROOT_ENV]);
        override_from(&mut sources.uv_releases, &[UV_MIRROR_ENV]);
        override_from(&mut sources.pypi, &[PYPI_URL_ENV]);
        override_from(&mut sources.go_dist, &[GO_MIRROR_ENV]);
        sources
    }
}
