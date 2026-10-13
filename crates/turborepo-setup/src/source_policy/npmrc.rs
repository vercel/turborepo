//! npm configuration keys the official-only source policy accepts in project
//! `.npmrc` files. This is an allowlist, never a denylist: registries (plain
//! or scoped), credentials, proxies, CA/TLS settings, `userconfig` and
//! `globalconfig` indirection, interpolated text, and every unknown key stay
//! rejected. A committed `.npmrc` never stands in for `setup.mirrors`, so
//! registry keys are rejected even at their official values.
//!
//! Keys use npm's canonical kebab-case spelling, which is also how npm and pnpm
//! name `npm_config_*` environment keys once `_` becomes `-`, so environment
//! checks can share [`non_fetching`].

/// Settings that only decide how npm or pnpm resolves, validates, or lays out
/// dependencies from the source it is already configured to use.
const NON_FETCHING: &[&str] = &[
    // Installs missing peer dependencies instead of warning about them.
    "auto-install-peers",
    // Links direct dependencies into each project instead of deduping them at
    // the root.
    "dedupe-direct-deps",
    // Fails an install whose `engines` do not match; a validation switch.
    "engine-strict",
    // Picks which packages are hoisted into `node_modules/.pnpm/node_modules`.
    "hoist-pattern",
    // Chooses between linking a workspace package and installing the same
    // version from the configured registry.
    "link-workspace-packages",
    // Selects the `node_modules` layout: isolated, hoisted, or pnp.
    "node-linker",
    // Picks which packages are hoisted into the root `node_modules`.
    "public-hoist-pattern",
    // Picks among satisfying versions (highest, time-based, lowest-direct).
    "resolution-mode",
    // Makes `add` and `install <pkg>` save exact versions.
    "save-exact",
    // Hoists every dependency into the root `node_modules`.
    "shamefully-hoist",
    // Fails on missing or invalid peer dependencies instead of warning.
    "strict-peer-dependencies",
];

/// Pattern lists may use ini's `key[]=value` form; other keys may not.
const LISTS: &[&str] = &["hoist-pattern", "public-hoist-pattern"];

/// Larger project `.npmrc` files are not inspected at all.
pub(super) const LIMIT: u64 = 64 * 1024;

/// Whether a canonical npm configuration key can only shape dependency
/// resolution or layout, never sources, credentials, or transport.
pub(super) fn non_fetching(key: &str) -> bool {
    NON_FETCHING.contains(&key)
}

/// The 1-based line of the first setting that is not known to be
/// non-fetching, or None when every setting is. Comments and blank lines
/// carry no settings.
///
/// This mirrors npm's and pnpm's `ini` decoding closely enough to fail closed:
/// a lone `\r` also ends a line, keys are trimmed and may end in `[]`, and a
/// key without `=` is a boolean. Anything that parser could read as another
/// key is rejected rather than interpreted: non-ASCII and control bytes,
/// sections, quoted or escaped keys, inline comments in keys, and `$`, which
/// npm and pnpm expand as `${VAR}` in keys and values.
pub(super) fn rejected_line(bytes: &[u8]) -> Option<usize> {
    for (index, line) in bytes.split(|&byte| byte == b'\n').enumerate() {
        for part in line.split(|&byte| byte == b'\r') {
            if !part
                .iter()
                .all(|&byte| byte == b'\t' || (b' '..=b'~').contains(&byte))
            {
                return Some(index + 1);
            }
            // Printable ASCII and tabs are always UTF-8.
            let text = std::str::from_utf8(part)
                .unwrap_or_default()
                .trim_matches([' ', '\t']);
            if text.is_empty() || text.starts_with([';', '#']) {
                continue;
            }
            if text.contains('$') || !setting_is_non_fetching(text) {
                return Some(index + 1);
            }
        }
    }
    None
}

fn setting_is_non_fetching(text: &str) -> bool {
    let key = text
        .split_once('=')
        .map_or(text, |(key, _)| key)
        .trim_matches([' ', '\t']);
    match key.strip_suffix("[]") {
        Some(list) => LISTS.contains(&list),
        None => non_fetching(key),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_allowlisted_layout_settings() {
        let repository = "auto-install-peers = true\nhoist-pattern[]=vfile-message\n# \
                          comment\n\n; comment\nresolution-mode=highest\ndedupe-direct-deps=true\n";
        assert_eq!(rejected_line(repository.as_bytes()), None);
        for key in NON_FETCHING {
            for line in [
                format!("{key}=true"),
                format!("  {key} = x ; note"),
                key.to_string(),
            ] {
                assert_eq!(rejected_line(line.as_bytes()), None, "{line}");
            }
        }
        for list in LISTS {
            assert_eq!(rejected_line(format!("{list}[]=*eslint*").as_bytes()), None);
        }
        assert_eq!(rejected_line(b""), None);
        assert_eq!(rejected_line(b"\r\n\t\r\n"), None);
    }

    #[test]
    fn rejects_sources_credentials_transport_indirection_and_unknown_keys() {
        for setting in [
            "registry=https://registry.npmjs.org/",
            "@scope:registry=https://npm.example.test/",
            "//registry.npmjs.org/:_authToken=private-secret",
            "_auth=private-secret",
            "always-auth=true",
            "proxy=http://127.0.0.1:1",
            "https-proxy=http://127.0.0.1:1",
            "noproxy=example.test",
            "strict-ssl=false",
            "ca=private",
            "cafile=/tmp/ca.pem",
            "cert=private",
            "key=private",
            "userconfig=/tmp/npmrc",
            "globalconfig=/tmp/npmrc",
            "prefix=/tmp/prefix",
            "ignore-scripts=true",
            "auto_install_peers=true",
            "autoInstallPeers=true",
            "AUTO-INSTALL-PEERS=true",
            "auto-install-peers[]=true",
            "hoist-pattern []=react",
            "unknown=true",
        ] {
            assert_eq!(rejected_line(setting.as_bytes()), Some(1), "{setting}");
        }
    }

    #[test]
    fn rejects_forms_npm_could_read_as_another_key() {
        for line in [
            "auto-install-peers=${REGISTRY}",
            "${KEY}=https://npm.example.test/",
            "[section]",
            "\"registry\"=https://npm.example.test/",
            "registry;x=https://npm.example.test/",
            "=true",
            "auto-install-peers=true\u{0b}",
            "\u{feff}auto-install-peers=true",
            "auto-install-peers=caf\u{e9}",
        ] {
            assert_eq!(rejected_line(line.as_bytes()), Some(1), "{line:?}");
        }
        // ini also splits at a lone carriage return.
        assert_eq!(
            rejected_line(b"save-exact=true\rregistry=https://npm.example.test/"),
            Some(1)
        );
        assert_eq!(
            rejected_line(b"save-exact=true\n\n# ok\nregistry=https://npm.example.test/\n"),
            Some(4)
        );
    }
}
