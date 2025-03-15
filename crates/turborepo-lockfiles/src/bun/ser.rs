use serde::{ser::SerializeTuple, Serialize};

use super::PackageEntry;

impl Serialize for PackageEntry {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut tuple = serializer.serialize_tuple(4)?;
        tuple.serialize_element(&self.ident)?;

        if let Some(info) = &self.info {
            tuple.serialize_element(&self.registry.as_deref().unwrap_or(""))?;
            tuple.serialize_element(info)?;
            tuple.serialize_element(&self.checksum)?;
            if let Some(root) = &self.root {
                tuple.serialize_element(root)?;
            }
        };

        tuple.end()
    }
}

#[cfg(test)]
mod test {
    use serde_json::json;

    use super::*;
    use crate::bun::PackageInfo;

    #[test]
    fn test_serialize_registry_package() {
        let package = PackageEntry {
            ident: "is-odd@3.0.1".into(),
            registry: Some("registry".into()),
            info: Some(PackageInfo {
                dependencies: [("is-number".into(), "^6.0.0".into())]
                    .into_iter()
                    .collect(),
                ..Default::default()
            }),
            checksum: Some("sha".into()),
            root: None,
        };

        let expected =
            json!(["is-odd@3.0.1", "registry", {"dependencies": {"is-number": "^6.0.0"}}, "sha"]);
        let actual = serde_json::to_value(&package).unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_serialize_registry_package_no_deps() {
        let package = PackageEntry {
            ident: "is-odd@3.0.1".into(),
            registry: Some("registry".into()),
            info: Some(PackageInfo {
                ..Default::default()
            }),
            checksum: Some("sha".into()),
            root: None,
        };

        let expected = json!(["is-odd@3.0.1", "registry", {}, "sha"]);
        let actual = serde_json::to_value(&package).unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_serialize_workspace_package() {
        let package = PackageEntry {
            ident: "@workspace/package".into(),
            registry: None,
            info: None,
            checksum: None,
            root: None,
        };

        let expected = json!(["@workspace/package"]);
        let actual = serde_json::to_value(&package).unwrap();
        assert_eq!(actual, expected);
    }
}
