use super::BunLockfile;
use crate::Lockfile;

/// Check if there are any global changes between two bun lockfiles
pub fn bun_global_change(prev_contents: &[u8], curr_contents: &[u8]) -> Result<bool, crate::Error> {
    let prev = BunLockfile::from_bytes(prev_contents)?;
    let curr = BunLockfile::from_bytes(curr_contents)?;

    Ok(prev.global_change(&curr as &dyn Lockfile))
}
