use std::{
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

pub fn ms_since_epoch() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis()
}

pub fn get_version() -> &'static str {
    log::debug!("fetching current version");
    include_str!("../../../version.txt")
        .split_once('\n')
        .expect("Failed to read version from version.txt")
        .0
}

pub fn get_config_path() -> PathBuf {
    // get directory
    let mut tmp = std::env::temp_dir();
    // set filename
    tmp.set_file_name("turbo-version.json");

    log::debug!("config directory {:?}", tmp);
    tmp
}
