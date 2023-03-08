use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("Error making HTTP request: {0}")]
    ReqwestError(#[from] reqwest::Error),
    #[error("skipping HTTP Request, too many failures have occurred.\nLast error: {0}")]
    TooManyFailures(#[from] Box<Error>),
}
