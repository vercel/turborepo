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
}
