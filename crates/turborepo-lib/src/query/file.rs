use std::sync::Arc;

use async_graphql::{Object, SimpleObject};
use turbo_trace::Tracer;
use turbopath::AbsoluteSystemPathBuf;

use crate::{
    query::{Array, Error},
    run::Run,
};

pub struct File {
    run: Arc<Run>,
    path: AbsoluteSystemPathBuf,
}

impl File {
    pub fn new(run: Arc<Run>, path: AbsoluteSystemPathBuf) -> Self {
        Self { run, path }
    }
}

#[derive(SimpleObject)]
struct TraceResult {
    files: Array<File>,
    errors: Array<String>,
}

impl TraceResult {
    fn new(result: turbo_trace::TraceResult, run: Arc<Run>) -> Self {
        Self {
            files: result
                .files
                .into_iter()
                .map(|path| File::new(run.clone(), path))
                .collect(),
            errors: result.errors.into_iter().map(|e| e.to_string()).collect(),
        }
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

    async fn dependencies(&self) -> TraceResult {
        let tracer = Tracer::new(
            self.run.repo_root().to_owned(),
            vec![self.path.clone()],
            None,
        );

        let result = tracer.trace();
        TraceResult::new(result, self.run.clone())
    }
}
