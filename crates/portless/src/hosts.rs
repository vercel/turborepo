use std::{
    fs, io,
    path::{Path, PathBuf},
};

const MARKER_START: &str = "# portless-start";
const MARKER_END: &str = "# portless-end";

pub fn hosts_path() -> PathBuf {
    #[cfg(windows)]
    {
        let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
        return PathBuf::from(root)
            .join("System32")
            .join("drivers")
            .join("etc")
            .join("hosts");
    }
    #[cfg(not(windows))]
    {
        PathBuf::from("/etc/hosts")
    }
}

/// Returns trimmed, non-empty lines inside the first valid managed block.
pub fn extract_managed_block(content: &str) -> Vec<String> {
    let (Some(start), Some(end)) = (content.find(MARKER_START), content.find(MARKER_END)) else {
        return Vec::new();
    };
    if end <= start {
        return Vec::new();
    }
    content[start + MARKER_START.len()..end]
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Removes the managed block and normalizes runs of 3+ newlines to two.
pub fn remove_block(content: &str) -> String {
    let (Some(start), Some(end)) = (content.find(MARKER_START), content.find(MARKER_END)) else {
        return content.to_owned();
    };
    let after_start = end.saturating_add(MARKER_END.len());
    let mut combined = String::with_capacity(content.len());
    combined.push_str(&content[..start]);
    if after_start <= content.len() {
        combined.push_str(&content[after_start..]);
    }
    let mut normalized = combined;
    while normalized.contains("\n\n\n") {
        normalized = normalized.replace("\n\n\n", "\n\n");
    }
    normalized.truncate(normalized.trim_end().len());
    normalized.push('\n');
    normalized
}

pub fn build_block<S: AsRef<str>>(hostnames: &[S]) -> String {
    if hostnames.is_empty() {
        return String::new();
    }
    let entries = hostnames
        .iter()
        .map(|hostname| format!("127.0.0.1 {}", hostname.as_ref()))
        .collect::<Vec<_>>()
        .join("\n");
    format!("{MARKER_START}\n{entries}\n{MARKER_END}")
}

/// Opt-out semantics from 0.15.1: only exact `0` and `false` disable sync.
pub fn should_auto_sync_hosts(value: Option<&str>) -> bool {
    !matches!(value, Some("0" | "false"))
}

pub fn sync_hosts_file<S: AsRef<str>>(hostnames: &[S]) -> bool {
    sync_hosts_file_at(&hosts_path(), hostnames).is_ok()
}

pub fn sync_hosts_file_at<S: AsRef<str>>(path: &Path, hostnames: &[S]) -> io::Result<()> {
    let content = fs::read_to_string(path).unwrap_or_default();
    let cleaned = remove_block(&content);
    let output = if hostnames.is_empty() {
        cleaned
    } else {
        format!("{}\n\n{}\n", cleaned.trim_end(), build_block(hostnames))
    };
    fs::write(path, output)
}

pub fn clean_hosts_file() -> bool {
    clean_hosts_file_at(&hosts_path()).is_ok()
}

pub fn clean_hosts_file_at(path: &Path) -> io::Result<()> {
    let content = fs::read_to_string(path).unwrap_or_default();
    if content.contains(MARKER_START) {
        fs::write(path, remove_block(&content))?;
    }
    Ok(())
}

pub fn get_managed_hostnames() -> Vec<String> {
    get_managed_hostnames_at(&hosts_path())
}

pub fn get_managed_hostnames_at(path: &Path) -> Vec<String> {
    let content = fs::read_to_string(path).unwrap_or_default();
    extract_managed_block(&content)
        .into_iter()
        .filter_map(|line| line.split_whitespace().nth(1).map(str::to_owned))
        .collect()
}

/// Checks IPv4 system resolution, matching Node's `dns.lookup({ family: 4 })`.
pub async fn check_host_resolution(hostname: &str) -> bool {
    let Ok(addresses) = tokio::net::lookup_host((hostname, 0)).await else {
        return false;
    };
    addresses
        .filter_map(|address| match address.ip() {
            std::net::IpAddr::V4(ip) => Some(ip),
            std::net::IpAddr::V6(_) => None,
        })
        .any(|ip| ip == std::net::Ipv4Addr::LOCALHOST)
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use tempfile::NamedTempFile;

    use super::*;

    #[test]
    fn managed_block_transformations_match_portless() {
        let content = "127.0.0.1 localhost\n\n# portless-start\n 127.0.0.1 app.localhost \
                       \n\n127.0.0.1 api.localhost\n# portless-end\n\nafter\n";
        assert_eq!(
            extract_managed_block(content),
            vec!["127.0.0.1 app.localhost", "127.0.0.1 api.localhost"]
        );
        assert_eq!(
            remove_block(content),
            "127.0.0.1 localhost\n\n\nafter\n".replace("\n\n\n", "\n\n")
        );
        assert_eq!(
            build_block(&["app.localhost", "api.localhost"]),
            "# portless-start\n127.0.0.1 app.localhost\n127.0.0.1 api.localhost\n# portless-end"
        );
    }

    #[test]
    fn malformed_markers_and_opt_out_are_faithful() {
        assert!(extract_managed_block("# portless-end\n# portless-start").is_empty());
        assert_eq!(remove_block("ordinary\n"), "ordinary\n");
        assert!(should_auto_sync_hosts(None));
        assert!(!should_auto_sync_hosts(Some("0")));
        assert!(!should_auto_sync_hosts(Some("false")));
        assert!(should_auto_sync_hosts(Some("FALSE")));
        assert!(should_auto_sync_hosts(Some("")));
    }

    #[test]
    fn sync_replaces_existing_block_and_clean_preserves_other_content() {
        let file = NamedTempFile::new().expect("tempfile");
        fs::write(
            file.path(),
            "before\n# portless-start\n127.0.0.1 old.localhost\n# portless-end\nafter\n",
        )
        .expect("seed");
        sync_hosts_file_at(file.path(), &["new.localhost"]).expect("sync");
        assert_eq!(get_managed_hostnames_at(file.path()), vec!["new.localhost"]);
        clean_hosts_file_at(file.path()).expect("clean");
        let content = fs::read_to_string(file.path()).expect("read");
        assert!(content.contains("before"));
        assert!(content.contains("after"));
        assert!(!content.contains("portless"));
    }

    #[tokio::test]
    async fn localhost_resolves_to_loopback() {
        assert!(check_host_resolution("localhost").await);
    }
}
