use core::fmt;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Path must start with a /")]
    NonRelative,
    #[error("Could not find default zone in micro-frontends configuration")]
    NoDefaultApplication,
    #[error("Multiple applications marked as default: {0:?}")]
    MultipleDefaultApplications(Vec<String>),
    #[error("Could not find micro-frontends configuration for application \"{0}\"")]
    NoApplicationConfiguration(String),
    #[error("Cannot define routing for default application \"{0}\"")]
    RoutingOnDefaultApplication(String),
    #[error("Invalid host: {0}")]
    InvalidHost(#[from] url::ParseError),
    #[error(
        "Application \"{0}\" isn't the default application and is missing routing configuration"
    )]
    MissingRouting(String),
    #[error("Unsupported version: {version}. Supported versions are: {supported_versions}")]
    UnsupportedVersion {
        version: String,
        supported_versions: String,
    },
    #[error("Invalid assetPrefix for application \"{0}\". Must not start or end with a slash.")]
    InvalidAssetPrefix(String),
    #[error("Invalid path for application \"{name}\". {path} must not end with a slash.")]
    PathTrailingSlash { name: String, path: String },
    #[error("Invalid path for application \"{name}\". {path} must start with a slash.")]
    PathNoLeadingSlash { name: String, path: String },
    #[error("Unable to read configuration file: {0}")]
    Io(#[from] std::io::Error),
    #[error("Unable to parse JSON: {0}")]
    JsonParse(String),
}

impl Error {
    /// Constructs an error message from multiple biome diagnostic errors
    pub fn biome_error(errors: Vec<biome_diagnostics::Error>) -> Self {
        struct DisplayDesc(biome_diagnostics::Error);
        impl fmt::Display for DisplayDesc {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                self.0.description(f)
            }
        }
        let error_messages = errors
            .into_iter()
            .map(|err| DisplayDesc(err).to_string())
            .collect::<Vec<_>>();
        Self::JsonParse(error_messages.join("\n"))
    }
}
