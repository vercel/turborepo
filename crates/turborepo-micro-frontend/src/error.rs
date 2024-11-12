use turborepo_errors::ParseDiagnostic;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Unable to read configuration file: {0}")]
    Io(#[from] std::io::Error),
    #[error("Unable to parse JSON: {0}")]
    JsonParse(String),
}

impl Error {
    /// Constructs an error message from multiple biome diagnostic errors
    pub fn biome_error(errors: Vec<biome_diagnostics::Error>) -> Self {
        let error_messages = errors
            .into_iter()
            .map(|err| ParseDiagnostic::from(err).to_string())
            .collect::<Vec<_>>();
        Self::JsonParse(error_messages.join("\n"))
    }
}
