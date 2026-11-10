//! Entry point: exercises module rules, cfg, embeds and runtime reads.
mod cli;
mod net;
#[cfg(target_os = "linux")]
#[path = "platform/linux.rs"]
mod sys;
#[cfg(windows)]
mod win;
#[cfg(test)]
mod tests;

const HELP: &str = include_str!("../help.txt");
include!(concat!(env!("OUT_DIR"), "/generated.rs"));

fn main() {
    let _cfg = std::fs::read_to_string("config.toml");
    let _logo = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/logo.svg"));
    println!("{HELP}");
}
