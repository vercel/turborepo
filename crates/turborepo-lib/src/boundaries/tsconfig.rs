use std::{collections::HashMap, sync::Arc};

use oxc_resolver::{Resolver, TsConfigSerde};
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};

use crate::boundaries::BoundariesResult;

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

    pub fn load(
        &mut self,
        path: &AbsoluteSystemPath,
        result: &mut BoundariesResult,
    ) -> Option<Arc<TsConfigSerde>> {
        for dir in path.ancestors() {
            if let Some(config) = self.configs.get(dir) {
                return Some(config.clone());
            }
            match self.resolver.resolve_tsconfig(dir) {
                Ok(config) => {
                    self.configs.insert(dir.to_owned(), config.clone());
                    return Some(config);
                }
                Err(err) => {
                    result
                        .warnings
                        .push(format!("Could not load tsconfig for {dir}: {err}"));
                }
            }
        }

        None
    }
}
