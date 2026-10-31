//! Automatic cleanup of the tool store and the shared download cache. Best
//! effort: nothing here can fail setup.
use std::time::{Duration, SystemTime};

use turborepo_setup::{
    artifact_cache::ArtifactCache,
    lock::{Installation, Lock, Platform},
};
use turborepo_tool_install::{GRACE, UserStore};

const DAY: Duration = Duration::from_secs(24 * 60 * 60);
/// Shared downloads that no setup in any clone referenced for this long are
/// evicted, like Cargo's automatic cleanup of downloaded crates.
const UNUSED_DAYS: u32 = 90;

/// After a successful setup, under the writer guard.
pub(super) fn after_provisioning(
    cache: Option<&ArtifactCache>,
    lock: &Lock,
    host: Platform,
) -> Vec<String> {
    let mut lines = Vec::new();
    let Some(cache) = cache else {
        return lines;
    };
    // Installs reused without reading the cache still count as a use.
    for (platform, sha256) in referenced(lock, host) {
        cache.mark_used(platform, sha256);
    }
    match cache.prune_unused(DAY * UNUSED_DAYS) {
        Ok(removed) => {
            if removed.entries > 0 {
                lines.push(format!(
                    "Removed {} of shared downloads no setup used in {UNUSED_DAYS} days.",
                    size(removed.bytes)
                ));
            }
        }
        Err(error) => lines.push(warning("the shared download cache", &error)),
    }
    lines
}

/// After setup released its own lease: collects what no repository using
/// `store` has referenced for the grace period, and reports store-wide disk
/// use. Skipped while another setup holds the store.
pub(super) fn collect(store: &UserStore, now: SystemTime) -> Vec<String> {
    match try_collect(store, now) {
        Ok(Some(lines)) => lines,
        Ok(None) => {
            tracing::debug!("skipped tool store collection: another setup is running");
            Vec::new()
        }
        Err(error) => {
            tracing::debug!("could not collect unused managed tools: {error}");
            Vec::new()
        }
    }
}

/// `None` when something else held the store and this round was skipped.
pub(super) fn try_collect(
    store: &UserStore,
    now: SystemTime,
) -> Result<Option<Vec<String>>, turborepo_tool_install::Error> {
    let Some(collected) = store.collect(now, |dir| super::trust::repo_state(store, dir))? else {
        return Ok(None);
    };
    let mut lines = Vec::new();
    if collected.removed.entries > 0 {
        lines.push(format!(
            "Removed {} of managed tools no repository selects.",
            size(collected.removed.bytes)
        ));
    }
    if collected.retained.entries > 0 {
        lines.push(format!(
            "Keeping {} of replaced managed tools for {} days after replacement in case a running \
             task still uses them.",
            size(collected.retained.bytes),
            GRACE.as_secs() / DAY.as_secs()
        ));
    }
    Ok(Some(lines))
}

// This host's locked artifacts, keyed as the shared cache stores them.
fn referenced(lock: &Lock, host: Platform) -> impl Iterator<Item = (Platform, &str)> {
    lock.tools()
        .values()
        .filter_map(move |tool| match &tool.installation {
            Installation::Managed { artifacts } => artifacts
                .get_key_value(&host)
                .or_else(|| artifacts.get_key_value(&Platform::Any)),
            _ => None,
        })
        .flat_map(|(platform, parts)| {
            parts
                .values()
                .map(move |artifact| (*platform, artifact.sha256.as_str()))
        })
}

fn warning(what: &str, error: &dyn std::fmt::Display) -> String {
    format!("Warning: could not clean up {what} ({error}). Setup is unaffected.")
}

fn size(bytes: u64) -> String {
    match bytes {
        0..1_000 => format!("{bytes} B"),
        1_000..1_000_000 => format!("{:.1} KB", bytes as f64 / 1e3),
        1_000_000..1_000_000_000 => format!("{:.1} MB", bytes as f64 / 1e6),
        _ => format!("{:.1} GB", bytes as f64 / 1e9),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn sizes_are_short_decimal_units() {
        for (bytes, expected) in [
            (0, "0 B"),
            (999, "999 B"),
            (1_500, "1.5 KB"),
            (182_400_000, "182.4 MB"),
            (2_000_000_000, "2.0 GB"),
        ] {
            assert_eq!(super::size(bytes), expected);
        }
    }
}
