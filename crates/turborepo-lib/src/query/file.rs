use std::sync::Arc;

use async_graphql::Object;
use itertools::Itertools;
use turbo_trace::Tracer;
use turbopath::AbsoluteSystemPathBuf;

use crate::{query::Error, run::Run};

pub struct File {
    run: Arc<Run>,
    path: AbsoluteSystemPathBuf,
}

impl File {
    pub fn new(run: Arc<Run>, path: AbsoluteSystemPathBuf) -> Self {
        Self { run, path }
    }
}

#[Object]
impl File {
    async fn contents(&self) -> Result<String, Error> {
        let contents = self.path.read_to_string()?;
        Ok(contents)
    }

    async fn path(&self) -> Result<String, Error> {
        Ok(self
            .run
            .repo_root()
            .anchor(&self.path)
            .map(|path| path.to_string())?)
    }

    async fn absolute_path(&self) -> Result<String, Error> {
        Ok(self.path.to_string())
    }

    async fn dependencies(&self) -> Result<Vec<File>, Error> {
        let tracer = Tracer::new(
            self.run.repo_root().to_owned(),
            vec![self.path.clone()],
            None,
        )?;

        let result = tracer.trace();
        if !result.errors.is_empty() {
            return Err(Error::Trace(result.errors));
        }

        Ok(result
            .files
            .into_iter()
            // Filter out the file we're looking at
            .filter(|file| file != &self.path)
            .map(|path| File::new(self.run.clone(), path))
            .sorted_by(|a, b| a.path.cmp(&b.path))
            .collect())
    }
}
