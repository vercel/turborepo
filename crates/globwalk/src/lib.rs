use std::fmt::Display;

use turbopath::AbsoluteSystemPathBuf;

pub enum WalkType {
    Files,
    Folders,
    All,
}

#[derive(Debug)]
pub enum WalkError {}

impl Display for WalkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        todo!()
    }
}

impl std::error::Error for WalkError {}

pub fn globwalk(
    basePath: AbsoluteSystemPathBuf,
    include: &[String],
    exclude: &[String],
    walk_type: WalkType,
) -> Result<Vec<AbsoluteSystemPathBuf>, WalkError> {
    let paths = Vec::new();

    Ok(paths)
}
