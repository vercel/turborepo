use std::{collections::HashMap, sync::Arc};

use oxc_resolver::{Resolver, TsConfigSerde};
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};

pub struct TsConfigLoader<'a> {
    configs: HashMap<AbsoluteSystemPathBuf, Arc<TsConfigSerde>>,
    resolver: &'a Resolver,
}

impl<'a> TsConfigLoader<'a> {
    pub fn new(resolver: &'a Resolver) -> Self {
        Self {
            configs: HashMap::new(),
            resolver,
        }
    }

    pub fn load(&mut self, path: &AbsoluteSystemPath) -> Option<Arc<TsConfigSerde>> {
        for dir in path.ancestors() {
            if let Some(config) = self.configs.get(dir) {
                return Some(config.clone());
            }
            if let Some(config) = self.resolver.resolve_tsconfig(&dir).ok() {
                self.configs.insert(dir.to_owned(), config.clone());
                return Some(config);
            }
        }

        None
    }
}
