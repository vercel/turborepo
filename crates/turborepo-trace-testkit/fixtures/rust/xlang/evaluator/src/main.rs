//! `format!` with literal and bound arguments, `concat!`, and `Path::join`.
use std::path::PathBuf;

const ROOT: &str = env!("CARGO_MANIFEST_DIR");

fn main() {
    let lang = "en";
    let locale = format!("{}/locales/{}.json", ROOT, lang);
    let profile = PathBuf::from(ROOT).join("profiles").join("release.ini");
    let a = std::fs::read_to_string(&locale).unwrap_or_default();
    let b = std::fs::read_to_string(&profile).unwrap_or_default();
    println!("{}", a.len() + b.len());
}
