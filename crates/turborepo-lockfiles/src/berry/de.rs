use serde::Deserialize;

use super::SemverString;

impl<'de> Deserialize<'de> for SemverString {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        // We use this to massage numerical semver versions to strings
        // e.g. 2
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum StringOrNum {
            String(String),
            Num(u64),
        }

        match StringOrNum::deserialize(deserializer)? {
            StringOrNum::String(s) => Ok(SemverString(s)),
            StringOrNum::Num(x) => Ok(SemverString(x.to_string())),
        }
    }
}

#[cfg(test)]
mod test {
    use std::collections::HashMap;

    use super::*;

    #[test]
    fn test_semver() {
        let input = "foo: 1.2.3
bar: 2
baz: latest
";

        let result: HashMap<String, SemverString> = serde_yaml::from_str(input).unwrap();

        assert_eq!(result["foo"].as_ref(), "1.2.3");
        assert_eq!(result["bar"].as_ref(), "2");
        assert_eq!(result["baz"].as_ref(), "latest");
    }
}
