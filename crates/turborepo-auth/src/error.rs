use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error(
        "loginUrl is configured to \"{value}\", but cannot be a base URL. This happens in \
         situations like using a `data:` URL."
    )]
    LoginUrlCannotBeABase { value: String },
}
