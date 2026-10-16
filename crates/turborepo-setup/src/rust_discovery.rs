//! Root rustup toolchain files. Parsing only: no rustup, rustc, network, or
//! directory-override search beyond the repository root.

use std::collections::BTreeSet;

use serde::Deserialize;

use crate::lock::Declaration;

const MAX_ENTRIES: usize = 32;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error("{0} is not a valid rustup toolchain file")]
    Invalid(&'static str),
    #[error("{0} declares {1}, which managed setup does not support")]
    Unsupported(&'static str, &'static str),
}

/// A portable rustup channel request. Host-qualified names are rejected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Channel {
    Stable,
    /// `X.Y` (latest patch) or exact `X.Y.Z`.
    Release(String),
    Beta(Option<String>),
    Nightly(Option<String>),
}

impl Channel {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        let dated = |rest: &str| {
            let bytes = rest.as_bytes();
            (rest.len() == 10
                && bytes[4] == b'-'
                && bytes[7] == b'-'
                && bytes
                    .iter()
                    .enumerate()
                    .all(|(i, b)| i == 4 || i == 7 || b.is_ascii_digit()))
            .then(|| rest.to_owned())
        };
        Some(match value {
            "stable" => Self::Stable,
            "beta" => Self::Beta(None),
            "nightly" => Self::Nightly(None),
            _ => {
                if let Some(date) = value.strip_prefix("beta-") {
                    Self::Beta(Some(dated(date)?))
                } else if let Some(date) = value.strip_prefix("nightly-") {
                    Self::Nightly(Some(dated(date)?))
                } else {
                    let parts: Vec<_> = value.split('.').collect();
                    let numeric = |part: &&str| {
                        !part.is_empty()
                            && part.len() <= 6
                            && part.bytes().all(|b| b.is_ascii_digit())
                            && (*part == "0" || !part.starts_with('0'))
                    };
                    if !matches!(parts.len(), 2 | 3) || !parts.iter().all(numeric) {
                        return None;
                    }
                    Self::Release(value.to_owned())
                }
            }
        })
    }

    /// Manifest path below the official distribution origin.
    pub fn manifest_path(&self) -> String {
        match self {
            Self::Stable => "/dist/channel-rust-stable.toml".into(),
            Self::Release(version) => format!("/dist/channel-rust-{version}.toml"),
            Self::Beta(None) => "/dist/channel-rust-beta.toml".into(),
            Self::Nightly(None) => "/dist/channel-rust-nightly.toml".into(),
            Self::Beta(Some(date)) => format!("/dist/{date}/channel-rust-beta.toml"),
            Self::Nightly(Some(date)) => format!("/dist/{date}/channel-rust-nightly.toml"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Profile {
    Minimal,
    Default,
}

impl Profile {
    pub fn name(self) -> &'static str {
        match self {
            Self::Minimal => "minimal",
            Self::Default => "default",
        }
    }
}

/// Root toolchain requirements plus their exact authored provenance.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Toolchain {
    pub channel: Channel,
    /// rustup's default profile applies when the file names none.
    pub profile: Profile,
    pub components: BTreeSet<String>,
    pub targets: BTreeSet<String>,
    declarations: Vec<Declaration>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    toolchain: Section,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Section {
    channel: Option<String>,
    path: Option<String>,
    profile: Option<String>,
    components: Option<Vec<String>>,
    targets: Option<Vec<String>>,
}

impl Toolchain {
    /// `file` is `rust-toolchain.toml` or the legacy `rust-toolchain`, whose
    /// single bare line names only a channel.
    pub fn parse(file: &'static str, text: &str) -> Result<Self, Error> {
        let invalid = || Error::Invalid(file);
        let legacy = text.trim();
        if file == "rust-toolchain" && !legacy.contains(['\n', '=', '[']) {
            let channel = Channel::parse(legacy).ok_or(Error::Unsupported(
                file,
                "a host-specific or custom channel",
            ))?;
            return Ok(Self {
                channel,
                profile: Profile::Default,
                components: BTreeSet::new(),
                targets: BTreeSet::new(),
                declarations: vec![Declaration {
                    file: file.into(),
                    field: None,
                    request: Some(legacy.into()),
                }],
            });
        }
        let File { toolchain } = toml::from_str(text).map_err(|_| invalid())?;
        if toolchain.path.is_some() {
            return Err(Error::Unsupported(file, "a custom toolchain path"));
        }
        let raw = toolchain
            .channel
            .ok_or(Error::Unsupported(file, "no toolchain channel"))?;
        let channel = Channel::parse(&raw).ok_or(Error::Unsupported(
            file,
            "a host-specific or custom channel",
        ))?;
        let profile = match toolchain.profile.as_deref() {
            None | Some("default") => Profile::Default,
            Some("minimal") => Profile::Minimal,
            Some(_) => return Err(Error::Unsupported(file, "an unsupported profile")),
        };
        let declaration = |field: &str, request: String| Declaration {
            file: file.into(),
            field: Some(format!("toolchain.{field}")),
            request: Some(request),
        };
        let mut declarations = vec![declaration("channel", raw)];
        if let Some(profile) = toolchain.profile {
            declarations.push(declaration("profile", profile));
        }
        let mut list = |field, values: Option<Vec<String>>| {
            let Some(values) = values else {
                return Ok(BTreeSet::new());
            };
            if values.len() > MAX_ENTRIES || !values.iter().all(|value| name(value)) {
                return Err(invalid());
            }
            // Authored order and spelling stay provenance; the set is semantic.
            let request = serde_json::to_string(&values).map_err(|_| invalid())?;
            declarations.push(declaration(field, request));
            Ok(values.into_iter().collect())
        };
        let components = list("components", toolchain.components)?;
        let targets = list("targets", toolchain.targets)?;
        Ok(Self {
            channel,
            profile,
            components,
            targets,
            declarations,
        })
    }

    pub fn declarations(&self) -> &[Declaration] {
        &self.declarations
    }
}

// Component and target-triple spellings, e.g. `thumbv8m.main-none-eabi`.
fn name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value.as_bytes()[0].is_ascii_lowercase()
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"-_.".contains(&b))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOML: &str = "rust-toolchain.toml";

    #[test]
    fn portable_channels_and_manifest_locations() {
        for (raw, path) in [
            ("stable", "/dist/channel-rust-stable.toml"),
            ("1.90", "/dist/channel-rust-1.90.toml"),
            ("1.90.0", "/dist/channel-rust-1.90.0.toml"),
            ("beta", "/dist/channel-rust-beta.toml"),
            ("nightly", "/dist/channel-rust-nightly.toml"),
            (
                "nightly-2026-10-06",
                "/dist/2026-10-06/channel-rust-nightly.toml",
            ),
            ("beta-2026-10-06", "/dist/2026-10-06/channel-rust-beta.toml"),
        ] {
            assert_eq!(Channel::parse(raw).unwrap().manifest_path(), path);
        }
        for raw in [
            "stable-x86_64-unknown-linux-gnu",
            "1.90.0-x86_64-unknown-linux-gnu",
            "nightly-2026-10",
            "nightly-2026/10/06",
            "1",
            "1.90.0.1",
            "01.90.0",
            "1.x",
            "my-toolchain",
            "",
        ] {
            assert_eq!(Channel::parse(raw), None, "{raw}");
        }
    }

    #[test]
    fn toml_declarations_keep_authored_provenance() {
        let toolchain = Toolchain::parse(
            TOML,
            "[toolchain]\nchannel = \"1.90\"\nprofile = \"minimal\"\ncomponents = [\"rustfmt\", \
             \"clippy\"]\ntargets = [\"wasm32-unknown-unknown\"]\n",
        )
        .unwrap();
        assert_eq!(toolchain.channel, Channel::Release("1.90".into()));
        assert_eq!(toolchain.profile, Profile::Minimal);
        assert_eq!(
            toolchain.components.iter().collect::<Vec<_>>(),
            ["clippy", "rustfmt"]
        );
        let fields: Vec<_> = toolchain
            .declarations()
            .iter()
            .map(|d| (d.field.as_deref().unwrap(), d.request.as_deref().unwrap()))
            .collect();
        assert_eq!(
            fields,
            [
                ("toolchain.channel", "1.90"),
                ("toolchain.profile", "minimal"),
                ("toolchain.components", r#"["rustfmt","clippy"]"#),
                ("toolchain.targets", r#"["wasm32-unknown-unknown"]"#),
            ]
        );
        let default = Toolchain::parse(TOML, "[toolchain]\nchannel = \"stable\"\n").unwrap();
        assert_eq!(default.profile, Profile::Default);
        assert_eq!(default.declarations().len(), 1);
    }

    #[test]
    fn legacy_files_name_one_channel_or_use_toml() {
        let legacy = Toolchain::parse("rust-toolchain", "nightly-2026-10-06\n").unwrap();
        assert_eq!(legacy.channel, Channel::Nightly(Some("2026-10-06".into())));
        assert_eq!(legacy.declarations()[0].field, None);
        let toml = Toolchain::parse("rust-toolchain", "[toolchain]\nchannel = \"beta\"\n");
        assert_eq!(toml.unwrap().channel, Channel::Beta(None));
        assert!(Toolchain::parse("rust-toolchain", "stable\nnightly\n").is_err());
    }

    #[test]
    fn unsupported_and_malformed_files_fail_closed() {
        for (text, expected) in [
            (
                "[toolchain]\npath = \"/opt/rust\"\n",
                Error::Unsupported(TOML, "a custom toolchain path"),
            ),
            (
                "[toolchain]\ncomponents = [\"clippy\"]\n",
                Error::Unsupported(TOML, "no toolchain channel"),
            ),
            (
                "[toolchain]\nchannel = \"stable-aarch64-apple-darwin\"\n",
                Error::Unsupported(TOML, "a host-specific or custom channel"),
            ),
            (
                "[toolchain]\nchannel = \"stable\"\nprofile = \"complete\"\n",
                Error::Unsupported(TOML, "an unsupported profile"),
            ),
            (
                "[toolchain]\nchannel = \"stable\"\nextra = 1\n",
                Error::Invalid(TOML),
            ),
            ("[other]\nchannel = \"stable\"\n", Error::Invalid(TOML)),
            (
                "[toolchain]\nchannel = \"stable\"\ncomponents = [\"Clippy\"]\n",
                Error::Invalid(TOML),
            ),
            (
                "[toolchain]\nchannel = \"stable\"\ntargets = [\"../x\"]\n",
                Error::Invalid(TOML),
            ),
            ("stable\n", Error::Invalid(TOML)),
        ] {
            assert_eq!(Toolchain::parse(TOML, text), Err(expected), "{text}");
        }
        let many = format!(
            "[toolchain]\nchannel = \"stable\"\ntargets = [{}]\n",
            vec!["\"wasm32-unknown-unknown\""; MAX_ENTRIES + 1].join(",")
        );
        assert_eq!(Toolchain::parse(TOML, &many), Err(Error::Invalid(TOML)));
    }
}
