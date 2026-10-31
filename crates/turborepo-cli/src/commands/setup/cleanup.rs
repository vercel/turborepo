//! Automatic cleanup of the tool store. Best effort: nothing here can fail
//! setup.
use std::time::{Duration, SystemTime};

use turborepo_tool_install::{GRACE, UserStore};

const DAY: Duration = Duration::from_secs(24 * 60 * 60);

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
