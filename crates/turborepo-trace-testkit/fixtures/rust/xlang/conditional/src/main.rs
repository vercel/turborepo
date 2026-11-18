//! `#[cfg]` alternatives: only the target's constant is live.
#[cfg(target_os = "linux")]
const CONF: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/conf/linux.conf");
#[cfg(not(target_os = "linux"))]
const CONF: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/conf/other.conf");

fn main() {
    println!("{}", std::fs::read_to_string(CONF).unwrap().len());
}
