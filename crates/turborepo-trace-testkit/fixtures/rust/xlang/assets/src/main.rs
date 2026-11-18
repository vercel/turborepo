//! Paths anchored at `CARGO_MANIFEST_DIR` in a constant and a local, read through a helper.
use std::path::Path;

const CONFIG: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/data/config.json");

fn size(p: &Path) -> usize {
    std::fs::read(p).map(|b| b.len()).unwrap_or(0)
}

fn main() {
    let schema = Path::new(env!("CARGO_MANIFEST_DIR")).join("data").join("schema.sql");
    // Created at run time: reported as missing.
    let cache = Path::new(env!("CARGO_MANIFEST_DIR")).join("cache.db");
    println!("{} {}", size(Path::new(CONFIG)) + size(&schema), cache.exists());
}
