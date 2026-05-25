use std::{
    cmp::Reverse,
    error::Error,
    fmt::Write as _,
    path::{Path, PathBuf},
    sync::Mutex,
};

use serde::{Deserialize, Serialize};

static HEAP_PROFILER: Mutex<Option<HeapProfiler>> = Mutex::new(None);

pub fn start_global(raw_path: impl Into<PathBuf>) {
    let mut profiler = HEAP_PROFILER
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    if profiler.is_none() {
        *profiler = Some(HeapProfiler::start(raw_path));
    }
}

pub fn finish_global() {
    let profiler = HEAP_PROFILER
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .take();

    drop(profiler);
}

pub struct HeapProfiler {
    raw_path: PathBuf,
    profiler: Option<dhat::Profiler>,
}

impl HeapProfiler {
    pub fn start(raw_path: impl Into<PathBuf>) -> Self {
        let raw_path = raw_path.into();

        if let Some(parent) = raw_path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            if let Err(error) = std::fs::create_dir_all(parent) {
                eprintln!(
                    "turbo: failed to create heap profile directory {}: {error}",
                    parent.display()
                );
            }
        }

        let profiler = dhat::Profiler::builder()
            .file_name(&raw_path)
            .trim_backtraces(Some(12))
            .build();

        Self {
            raw_path,
            profiler: Some(profiler),
        }
    }
}

impl Drop for HeapProfiler {
    fn drop(&mut self) {
        let Some(profiler) = self.profiler.take() else {
            return;
        };

        let totals = HeapTotals::from(dhat::HeapStats::get());
        drop(profiler);

        if let Err(error) = write_summaries(&self.raw_path, totals) {
            eprintln!(
                "turbo: failed to write heap profile summary for {}: {error}",
                self.raw_path.display()
            );
        }
    }
}

#[derive(Clone, Copy, Serialize)]
struct HeapTotals {
    total_allocations: u64,
    total_allocated_bytes: u64,
    live_allocations_at_end: usize,
    live_bytes_at_end: usize,
    peak_live_allocations: usize,
    peak_live_bytes: usize,
}

impl From<dhat::HeapStats> for HeapTotals {
    fn from(stats: dhat::HeapStats) -> Self {
        Self {
            total_allocations: stats.total_blocks,
            total_allocated_bytes: stats.total_bytes,
            live_allocations_at_end: stats.curr_blocks,
            live_bytes_at_end: stats.curr_bytes,
            peak_live_allocations: stats.max_blocks,
            peak_live_bytes: stats.max_bytes,
        }
    }
}

#[derive(Deserialize)]
struct DhatProfile {
    cmd: String,
    pid: u32,
    te: u128,
    pps: Vec<DhatPoint>,
    ftbl: Vec<String>,
}

#[derive(Deserialize)]
struct DhatPoint {
    tb: u64,
    tbk: u64,
    #[serde(default)]
    mb: Option<usize>,
    #[serde(default)]
    mbk: Option<usize>,
    #[serde(default)]
    gb: Option<usize>,
    #[serde(default)]
    gbk: Option<usize>,
    #[serde(default)]
    eb: Option<usize>,
    #[serde(default)]
    ebk: Option<usize>,
    fs: Vec<usize>,
}

#[derive(Serialize)]
struct HeapSummary {
    raw_profile: String,
    command: String,
    pid: u32,
    elapsed_micros: u128,
    totals: HeapTotals,
    top_by_allocated_bytes: Vec<AllocationSite>,
    top_by_allocations: Vec<AllocationSite>,
}

#[derive(Clone, Serialize)]
struct AllocationSite {
    allocated_bytes: u64,
    allocations: u64,
    peak_live_bytes: Option<usize>,
    peak_live_allocations: Option<usize>,
    live_bytes_at_global_peak: Option<usize>,
    live_allocations_at_global_peak: Option<usize>,
    live_bytes_at_end: Option<usize>,
    live_allocations_at_end: Option<usize>,
    frames: Vec<String>,
}

fn write_summaries(raw_path: &Path, totals: HeapTotals) -> Result<(), Box<dyn Error>> {
    let profile: DhatProfile = serde_json::from_str(&std::fs::read_to_string(raw_path)?)?;
    let summary = build_summary(raw_path, totals, profile);

    std::fs::write(
        summary_path(raw_path, "summary.json"),
        serde_json::to_string_pretty(&summary)?,
    )?;
    std::fs::write(
        summary_path(raw_path, "summary.txt"),
        summary_text(&summary),
    )?;

    Ok(())
}

fn build_summary(raw_path: &Path, totals: HeapTotals, profile: DhatProfile) -> HeapSummary {
    let mut sites: Vec<_> = profile
        .pps
        .iter()
        .map(|point| allocation_site(point, &profile.ftbl))
        .collect();

    sites.sort_by_key(|site| Reverse(site.allocated_bytes));
    let top_by_allocated_bytes = sites.iter().take(20).cloned().collect();

    sites.sort_by_key(|site| Reverse(site.allocations));
    let top_by_allocations = sites.iter().take(20).cloned().collect();

    HeapSummary {
        raw_profile: raw_path.display().to_string(),
        command: profile.cmd,
        pid: profile.pid,
        elapsed_micros: profile.te,
        totals,
        top_by_allocated_bytes,
        top_by_allocations,
    }
}

fn allocation_site(point: &DhatPoint, frames: &[String]) -> AllocationSite {
    AllocationSite {
        allocated_bytes: point.tb,
        allocations: point.tbk,
        peak_live_bytes: point.mb,
        peak_live_allocations: point.mbk,
        live_bytes_at_global_peak: point.gb,
        live_allocations_at_global_peak: point.gbk,
        live_bytes_at_end: point.eb,
        live_allocations_at_end: point.ebk,
        frames: point
            .fs
            .iter()
            .filter_map(|idx| frames.get(*idx))
            .take(12)
            .cloned()
            .collect(),
    }
}

fn summary_path(raw_path: &Path, suffix: &str) -> PathBuf {
    let mut path = raw_path.as_os_str().to_owned();
    path.push(format!(".{suffix}"));
    path.into()
}

fn summary_text(summary: &HeapSummary) -> String {
    let mut output = String::new();

    let _ = writeln!(output, "Turbo Heap Allocation Summary");
    let _ = writeln!(output, "raw_profile: {}", summary.raw_profile);
    let _ = writeln!(output, "command: {}", summary.command);
    let _ = writeln!(output, "pid: {}", summary.pid);
    let _ = writeln!(output, "elapsed_micros: {}", summary.elapsed_micros);
    let _ = writeln!(output);
    write_totals(&mut output, summary.totals);
    let _ = writeln!(output);
    write_sites(
        &mut output,
        "top_by_allocated_bytes",
        &summary.top_by_allocated_bytes,
    );
    let _ = writeln!(output);
    write_sites(
        &mut output,
        "top_by_allocations",
        &summary.top_by_allocations,
    );

    output
}

fn write_totals(output: &mut String, totals: HeapTotals) {
    let _ = writeln!(output, "totals:");
    let _ = writeln!(
        output,
        "total_allocated_bytes: {} ({})",
        totals.total_allocated_bytes,
        format_bytes(totals.total_allocated_bytes)
    );
    let _ = writeln!(output, "total_allocations: {}", totals.total_allocations);
    let _ = writeln!(
        output,
        "peak_live_bytes: {} ({})",
        totals.peak_live_bytes,
        format_bytes(totals.peak_live_bytes as u64)
    );
    let _ = writeln!(
        output,
        "peak_live_allocations: {}",
        totals.peak_live_allocations
    );
    let _ = writeln!(
        output,
        "live_bytes_at_end: {} ({})",
        totals.live_bytes_at_end,
        format_bytes(totals.live_bytes_at_end as u64)
    );
    let _ = writeln!(
        output,
        "live_allocations_at_end: {}",
        totals.live_allocations_at_end
    );
}

fn write_sites(output: &mut String, title: &str, sites: &[AllocationSite]) {
    let _ = writeln!(output, "{title}:");
    for (idx, site) in sites.iter().enumerate() {
        let _ = writeln!(
            output,
            "{}. allocated_bytes: {} ({})",
            idx + 1,
            site.allocated_bytes,
            format_bytes(site.allocated_bytes)
        );
        let _ = writeln!(output, "   allocations: {}", site.allocations);
        write_optional_usize(output, "peak_live_bytes", site.peak_live_bytes);
        write_optional_usize(output, "peak_live_allocations", site.peak_live_allocations);
        write_optional_usize(
            output,
            "live_bytes_at_global_peak",
            site.live_bytes_at_global_peak,
        );
        write_optional_usize(
            output,
            "live_allocations_at_global_peak",
            site.live_allocations_at_global_peak,
        );
        write_optional_usize(output, "live_bytes_at_end", site.live_bytes_at_end);
        write_optional_usize(
            output,
            "live_allocations_at_end",
            site.live_allocations_at_end,
        );
        let _ = writeln!(output, "   frames:");
        for frame in &site.frames {
            let _ = writeln!(output, "   - {frame}");
        }
    }
}

fn write_optional_usize(output: &mut String, name: &str, value: Option<usize>) {
    if let Some(value) = value {
        let _ = writeln!(output, "   {name}: {value}");
    }
}

fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = UNITS[0];

    for next_unit in UNITS.iter().skip(1) {
        if value < 1024.0 {
            break;
        }
        value /= 1024.0;
        unit = next_unit;
    }

    if unit == "B" {
        format!("{bytes} B")
    } else {
        format!("{value:.2} {unit}")
    }
}

#[cfg(test)]
mod tests {
    use super::format_bytes;

    #[test]
    fn formats_bytes() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(1024), "1.00 KiB");
        assert_eq!(format_bytes(1024 * 1024), "1.00 MiB");
    }
}
