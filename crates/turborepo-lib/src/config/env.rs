use std::collections::HashMap;

use config::Environment;

#[derive(Debug, Clone, Default)]
pub struct MappedEnvironment {
    inner: Environment,
    replacements: HashMap<String, String>,
}

impl MappedEnvironment {
    #[allow(dead_code)]
    pub fn with_prefix(s: &str) -> Self {
        Self {
            inner: Environment::with_prefix(s),
            ..Default::default()
        }
    }

    #[allow(dead_code)]
    pub fn source(mut self, source: Option<HashMap<String, String>>) -> Self {
        self.inner = self.inner.source(source);
        self
    }

    /// Adds a replacement rule that will map environment variable names to new
    /// ones
    ///
    /// Useful when environment variable names don't match up with config file
    /// names Replacement happens after config::Environment normalization
    #[allow(dead_code)]
    pub fn replace<S: Into<String>>(mut self, variable_name: S, replacement: S) -> Self {
        self.replacements
            .insert(variable_name.into(), replacement.into());
        self
    }
}

impl config::Source for MappedEnvironment {
    fn clone_into_box(&self) -> Box<dyn config::Source + Send + Sync> {
        Box::new(Self {
            inner: self.inner.clone(),
            replacements: self.replacements.clone(),
        })
    }

    fn collect(
        &self,
    ) -> std::result::Result<config::Map<String, config::Value>, config::ConfigError> {
        self.inner.collect().map(|config| {
            config
                .into_iter()
                .map(|(key, value)| {
                    let key = self.replacements.get(&key).cloned().unwrap_or(key);
                    (key, value)
                })
                .collect()
        })
    }
}

#[cfg(test)]
mod test {
    use config::Config;
    use serde::Deserialize;

    use super::*;

    #[test]
    fn test_replacement() {
        #[derive(Debug, Clone, Deserialize)]
        struct TestConfig {
            bar: u32,
            baz: String,
        }

        let mapped_env = MappedEnvironment::with_prefix("TURBO")
            .replace("foo", "bar")
            .source({
                let mut map = HashMap::new();
                map.insert("TURBO_FOO".into(), "42".into());
                map.insert("TURBO_BAZ".into(), "sweet".into());
                Some(map)
            });

        let config: TestConfig = Config::builder()
            .add_source(mapped_env)
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap();
        assert_eq!(config.bar, 42);
        assert_eq!(config.baz, "sweet");
    }
}
