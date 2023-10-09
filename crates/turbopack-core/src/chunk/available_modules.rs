use anyhow::Result;
use indexmap::IndexSet;
use turbo_tasks::{TryFlatJoinIterExt, TryJoinIterExt, ValueToString, Vc};
use turbo_tasks_hash::Xxh3Hash64Hasher;

use super::ChunkableModule;
use crate::module::Module;

/// Allows to gather information about which assets are already available.
/// Adding more roots will form a linked list like structure to allow caching
/// `include` queries.
#[turbo_tasks::value]
pub struct AvailableAssets {
    parent: Option<Vc<AvailableAssets>>,
    modules: IndexSet<Vc<Box<dyn ChunkableModule>>>,
}

#[turbo_tasks::value_impl]
impl AvailableAssets {
    #[turbo_tasks::function]
    fn new_normalized(
        parent: Option<Vc<AvailableAssets>>,
        modules: Vec<Vc<Box<dyn ChunkableModule>>>,
    ) -> Vc<Self> {
        AvailableAssets {
            parent,
            modules: modules.into_iter().collect(),
        }
        .cell()
    }

    #[turbo_tasks::function]
    pub fn new(modules: Vec<Vc<Box<dyn ChunkableModule>>>) -> Vc<Self> {
        Self::new_normalized(None, modules)
    }

    #[turbo_tasks::function]
    pub async fn with_modules(
        self: Vc<Self>,
        modules: Vec<Vc<Box<dyn ChunkableModule>>>,
    ) -> Result<Vc<Self>> {
        let modules = modules
            .into_iter()
            .map(|module| async move { Ok((!*self.includes(module).await?).then_some(module)) })
            .try_flat_join()
            .await?;
        Ok(Self::new_normalized(Some(self), modules))
    }

    #[turbo_tasks::function]
    pub async fn hash(self: Vc<Self>) -> Result<Vc<u64>> {
        let this = self.await?;
        let mut hasher = Xxh3Hash64Hasher::new();
        if let Some(parent) = this.parent {
            hasher.write_value(parent.hash().await?);
        } else {
            hasher.write_value(0u64);
        }
        let module_idents = this
            .modules
            .iter()
            .map(|module| module.ident().to_string())
            .try_join()
            .await?;
        for ident in module_idents {
            hasher.write_value(ident);
        }
        Ok(Vc::cell(hasher.finish()))
    }

    #[turbo_tasks::function]
    pub async fn includes(self: Vc<Self>, asset: Vc<Box<dyn ChunkableModule>>) -> Result<Vc<bool>> {
        let this = self.await?;
        if let Some(parent) = this.parent {
            if *parent.includes(asset).await? {
                return Ok(Vc::cell(true));
            }
        }
        if this.modules.contains(&asset) {
            return Ok(Vc::cell(true));
        }
        Ok(Vc::cell(false))
    }
}
