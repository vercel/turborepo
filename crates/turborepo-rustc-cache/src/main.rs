mod args;
mod cache;
mod hash;

use std::{env, process};

use crate::{args::parse_rustc_args, cache::LocalCache, hash::compute_cache_key};

fn main() {
    let exit_code = match run() {
        Ok(code) => code,
        Err(e) => {
            eprintln!("turbo-rustc-cache: falling back to direct rustc: {e}");
            run_rustc_directly()
        }
    };
    process::exit(exit_code);
}

fn run() -> Result<i32, Box<dyn std::error::Error>> {
    let all_args: Vec<String> = env::args().collect();

    let Some(parsed) = parse_rustc_args(&all_args) else {
        // Can't parse args — just run rustc directly
        return Ok(run_rustc_directly());
    };

    if !parsed.is_cacheable() {
        // Binary, proc-macro, or cdylib — run rustc directly, no caching
        return Ok(run_rustc_passthrough(&parsed));
    }

    let Some(out_dir) = &parsed.out_dir else {
        return Ok(run_rustc_passthrough(&parsed));
    };

    let Some(crate_name) = &parsed.crate_name else {
        return Ok(run_rustc_passthrough(&parsed));
    };

    let cache = LocalCache::from_env();
    let cache_key = compute_cache_key(&parsed)?;

    // Try cache restore
    if let Ok(Some(_restored_files)) = cache.restore(&cache_key, out_dir) {
        if env::var("TURBO_RUSTC_CACHE_LOG").is_ok() {
            eprintln!(
                "turbo-rustc-cache: cache hit for {crate_name} ({key})",
                key = &cache_key[..12]
            );
        }
        return Ok(0);
    }

    // Cache miss — run the real compiler
    let exit_code = run_rustc_passthrough(&parsed);

    if exit_code == 0 {
        // Compilation succeeded — cache the output
        let emit_types = parsed.cacheable_emit_types();
        if let Err(e) = cache.store(&cache_key, out_dir, crate_name, &emit_types) {
            if env::var("TURBO_RUSTC_CACHE_LOG").is_ok() {
                eprintln!("turbo-rustc-cache: failed to cache {crate_name}: {e}");
            }
            // Non-fatal — the build still succeeded
        } else if env::var("TURBO_RUSTC_CACHE_LOG").is_ok() {
            eprintln!(
                "turbo-rustc-cache: cached {crate_name} ({key})",
                key = &cache_key[..12]
            );
        }
    }

    Ok(exit_code)
}

/// Run rustc with the original arguments, bypassing caching entirely.
fn run_rustc_directly() -> i32 {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        return 1;
    }

    let status = process::Command::new(&args[1]).args(&args[2..]).status();

    match status {
        Ok(s) => s.code().unwrap_or(1),
        Err(_) => 1,
    }
}

/// Run rustc for a parsed invocation. Used for uncacheable targets
/// and cache misses.
fn run_rustc_passthrough(parsed: &args::ParsedArgs) -> i32 {
    let all_args: Vec<String> = env::args().collect();

    let status = process::Command::new(&parsed.rustc_path)
        .args(&all_args[2..])
        .status();

    match status {
        Ok(s) => s.code().unwrap_or(1),
        Err(_) => 1,
    }
}
