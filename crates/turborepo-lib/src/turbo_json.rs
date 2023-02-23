use std::{fs::File, path::Path};

use anyhow::Result;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct TurboJson {
    extends: Option<Vec<String>>,
}

impl TurboJson {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Ok(serde_json::from_reader(File::open(path)?)?)
    }
    pub fn no_extends(&self) -> bool {
        self.extends.is_none()
    }
}

#[test]
fn test_turbo_json() {
    let turbo_json: TurboJson = serde_json::from_str("{}").unwrap();
    assert_eq!(turbo_json.extends, None);

    let turbo_json: TurboJson = serde_json::from_str(r#"{ "extends": ["//"] }"#).unwrap();
    assert_eq!(turbo_json.extends, Some(vec!["//".to_string()]));

    let turbo_json: TurboJson = serde_json::from_str(r#"{ "extends": ["//", "~"] }"#).unwrap();
    assert_eq!(
        turbo_json.extends,
        Some(vec!["//".to_string(), "~".to_string()])
    );
}
