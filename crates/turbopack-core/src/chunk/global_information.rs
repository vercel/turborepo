use std::collections::HashMap;

use anyhow::Result;
use turbo_tasks::{ValueToString, Vc};

use super::ModuleId;
use crate::ident::AssetIdent;

#[turbo_tasks::value]
#[derive(Clone, Debug)]
pub struct GlobalInformation {
    pub module_id_map: HashMap<AssetIdent, ModuleId>,
}

impl GlobalInformation {
    pub async fn get_module_id(&self, asset_ident: Vc<AssetIdent>) -> Result<Vc<ModuleId>> {
        let ident_str = asset_ident.to_string().await?;
        let ident = asset_ident.await?;
        let hashed_module_id = self.module_id_map.get(&ident);
        if let Some(hashed_module_id) = hashed_module_id {
            dbg!("Hashed module ID found", &ident_str, hashed_module_id);
            return Ok(hashed_module_id.clone().cell());
        }
        dbg!("Hashed module ID not found", &ident_str);
        return Ok(ModuleId::String(ident_str.clone_value()).cell());
    }
}

#[turbo_tasks::value(transparent)]
pub struct OptionGlobalInformation(Option<GlobalInformation>);
