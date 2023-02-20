use anyhow::Result;
use turbo_tasks::{
    primitives::{BoolVc, U64Vc},
    TryJoinIterExt, ValueToString,
};
use turbo_tasks_hash::Xxh3Hash64Hasher;

use crate::asset::{Asset, AssetVc};

#[turbo_tasks::value]
pub struct AvailableAssets {
    parent: Option<AvailableAssetsVc>,
    roots: Vec<AssetVc>,
}

#[turbo_tasks::value_impl]
impl AvailableAssetsVc {
    #[turbo_tasks::function]
    fn new_normalized(parent: Option<AvailableAssetsVc>, roots: Vec<AssetVc>) -> Self {
        AvailableAssets { parent, roots }.cell()
    }

    #[turbo_tasks::function]
    pub fn new(roots: Vec<AssetVc>) -> Self {
        Self::new_normalized(None, roots)
    }

    #[turbo_tasks::function]
    pub async fn with_roots(self, roots: Vec<AssetVc>) -> Result<Self> {
        let roots = roots
            .into_iter()
            .map(|root| async move { Ok((self.includes(root).await?, root)) })
            .try_join()
            .await?
            .into_iter()
            .filter_map(|(included, root)| (!*included).then_some(root))
            .collect();
        Ok(Self::new_normalized(Some(self), roots))
    }

    #[turbo_tasks::function]
    pub async fn hash(self) -> Result<U64Vc> {
        let this = self.await?;
        let mut hasher = Xxh3Hash64Hasher::new();
        if let Some(parent) = this.parent {
            hasher.write_value(parent.hash().await?);
        }
        for root in &this.roots {
            hasher.write_value(root.ident().to_string().await?);
        }
        Ok(U64Vc::cell(hasher.finish()))
    }

    #[turbo_tasks::function]
    pub async fn includes(self, asset: AssetVc) -> Result<BoolVc> {
        let this = self.await?;
        if let Some(parent) = this.parent {
            if *parent.includes(asset).await? {
                return Ok(BoolVc::cell(true));
            }
        }
        // TODO implement
        Ok(BoolVc::cell(false))
    }
}

impl AvailableAssetsVc {
    pub fn from(parent: Option<Self>, root: Option<AssetVc>) -> Option<Self> {
        let Some(root) = root else {
            return parent;
        };
        if let Some(parent) = parent {
            Some(parent.with_roots(vec![root]))
        } else {
            Some(Self::new(vec![root]))
        }
    }
}
