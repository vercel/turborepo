pub mod http;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum CacheError {
    #[error("IO error: {0}")]
    IO(#[from] std::io::Error),
    #[error("Timestamp is invalid {0}")]
    Timestamp(#[from] std::num::TryFromIntError),
}
