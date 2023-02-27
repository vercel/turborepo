use anyhow::Result;
use indexmap::IndexMap;
use turbopack_core::{chunk::ModuleIdReadRef, code_builder::CodeReadRef};

use super::{content::EcmascriptChunkContentVc, version::EcmascriptChunkVersionVc};

#[turbo_tasks::value]
pub(super) struct EcmascriptChunkUpdate {
    pub added: IndexMap<ModuleIdReadRef, (u64, CodeReadRef)>,
    pub deleted: IndexMap<ModuleIdReadRef, u64>,
    pub modified: IndexMap<ModuleIdReadRef, CodeReadRef>,
}

pub(super) async fn update_ecmascript_chunk(
    content: EcmascriptChunkContentVc,
    from_version: EcmascriptChunkVersionVc,
) -> Result<UpdateVc> {
    let to_version = self_vc.version();
    let from_version =
        if let Some(from) = EcmascriptChunkVersionVc::resolve_from(from_version).await? {
            from
        } else {
            return Ok(Update::Total(TotalUpdate {
                to: to_version.into(),
            })
            .cell());
        };

    let to = to_version.await?;
    let from = from_version.await?;

    // When to and from point to the same value we can skip comparing them.
    // This will happen since `cell_local` will not clone the value, but only make
    // the local cell point to the same immutable value (Arc).
    if from.ptr_eq(&to) {
        return Ok(Update::None.cell());
    }

    let this = self_vc.await?;
    let chunk_path = &this.chunk_path.await?.path;

    // TODO(alexkirsz) This should probably be stored as a HashMap already.
    let mut module_factories: IndexMap<_, _> = this
        .module_factories
        .iter()
        .map(|entry| (entry.id(), entry))
        .collect();
    let mut added = IndexMap::new();
    let mut modified = IndexMap::new();
    let mut deleted = IndexSet::new();

    for (id, hash) in &from.module_factories_hashes {
        let id = &**id;
        if let Some(entry) = module_factories.remove(id) {
            if entry.hash != *hash {
                modified.insert(id, HmrUpdateEntry::new(entry, chunk_path));
            }
        } else {
            deleted.insert(id);
        }
    }

    // Remaining entries are added
    for (id, entry) in module_factories {
        added.insert(id, HmrUpdateEntry::new(entry, chunk_path));
    }

    let update = if added.is_empty() && modified.is_empty() && deleted.is_empty() {
        Update::None
    } else {
        let chunk_update = EcmascriptChunkUpdate {
            added,
            modified,
            deleted,
        };

        Update::Partial(PartialUpdate {
            to: to_version.into(),
            instruction: JsonValueVc::cell(serde_json::to_value(&chunk_update)?),
        })
    };

    Ok(update.into())
}
