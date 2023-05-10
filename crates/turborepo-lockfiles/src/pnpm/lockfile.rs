use super::data::{PnpmLockfileData, ProjectSnapshot};

pub struct PnpmLockfile<'a> {
    data: &'a PnpmLockfileData,
}
