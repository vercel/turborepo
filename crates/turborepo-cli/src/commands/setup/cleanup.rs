//! Automatic cleanup of managed storage that turbo.lock no longer selects.
//! Best effort: nothing here can fail setup.
use std::{path::Path, time::Duration};

use turborepo_setup::{
    artifact_cache::ArtifactCache,
    lock::{Installation, Lock, Platform},
};
use turborepo_tool_install::{StorageUse, Store};

const DAY: Duration = Duration::from_secs(24 * 60 * 60);
/// `turbo run` uses the selected generation without the setup lock, so a
/// replaced generation stays this long for runs that started before setup.
const GRACE_DAYS: u32 = 7;
/// Shared downloads that no setup in any clone referenced for this long are
/// evicted, like Cargo's automatic cleanup of downloaded crates.
const UNUSED_DAYS: u32 = 90;

/// After a successful reconcile, under the Store guard.
pub(super) fn after_provisioning(
    store: &mut Store,
    cache: Option<&ArtifactCache>,
    lock: &Lock,
    host: Platform,
) -> Vec<String> {
    let mut lines = match store.remove_inactive(DAY * GRACE_DAYS) {
        Ok(usage) => report(&usage, "Removed"),
        Err(error) => vec![warning("replaced managed tools", &error)],
    };
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

/// Read-only report of what the next setup removes from this clone.
pub(super) fn inspect(root: &Path) -> Vec<String> {
    match Store::storage_use(root, DAY * GRACE_DAYS) {
        Ok(Some(usage)) => report(&usage, "turbo setup will remove"),
        Ok(None) => Vec::new(),
        Err(error) => vec![format!(
            "Warning: could not measure managed tool disk use ({error})."
        )],
    }
}

fn report(usage: &StorageUse, removal: &str) -> Vec<String> {
    let mut lines = Vec::new();
    if usage.removable.entries > 0 {
        lines.push(format!(
            "{removal} {} of managed tools turbo.lock no longer selects.",
            size(usage.removable.bytes)
        ));
    }
    if usage.retained.entries > 0 {
        lines.push(format!(
            "Keeping {} of replaced managed tools for {GRACE_DAYS} days after replacement in case \
             a running task still uses them.",
            size(usage.retained.bytes)
        ));
    }
    lines
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
