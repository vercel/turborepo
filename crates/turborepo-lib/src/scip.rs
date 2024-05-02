use scip::Index;
use thiserror::Error;

use crate::run::Run;

#[derive(Debug, Error)]
enum Error {}

impl Run {
    pub fn output_scip(&self) -> Result<Index, Error> {
        Ok(Index {
            metadata: MessageField::some(Metadata::new()),
        })
    }
}
