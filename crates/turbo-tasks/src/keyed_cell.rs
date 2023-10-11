use anyhow::Result;

use crate as turbo_tasks;
use crate::{
    macro_helpers::find_cell_by_type, CurrentCellRef, RawVc, ValueTypeId, Vc, VcValueType,
};

#[turbo_tasks::value]
pub struct KeyedCellContext;

impl KeyedCellContext {
    pub fn new() -> Vc<Self> {
        KeyedCellContext.cell()
    }
}

#[turbo_tasks::value]
struct KeyedCell {
    cell: RawVc,
    #[turbo_tasks(trace_ignore, debug_ignore)]
    cell_ref: CurrentCellRef,
}

impl KeyedCell {}

#[turbo_tasks::value_impl]
impl KeyedCell {
    #[turbo_tasks::function]
    fn new(_context: Vc<KeyedCellContext>, _key: String, value_type_id: ValueTypeId) -> Vc<Self> {
        let cell_ref = find_cell_by_type(value_type_id);
        let raw: RawVc = cell_ref.into();
        KeyedCell {
            cell: raw.into(),
            cell_ref,
        }
        .cell()
    }
}

pub async fn keyed_cell<T: PartialEq + Eq + VcValueType>(
    context: Vc<KeyedCellContext>,
    key: String,
    content: T,
) -> Result<Vc<T>> {
    let cell = KeyedCell::new(context, key, T::get_value_type_id()).await?;
    cell.cell_ref.compare_and_update_shared(content);
    Ok(cell.cell.into())
}
