use std::io;

use globwalk::ValidatedGlob;
use thiserror::Error;
use tonic::{Code, Status};
use tracing::info;
use turbopath::AbsoluteSystemPathBuf;

use super::{
    connector::{DaemonConnector, DaemonConnectorError},
    endpoint::SocketOpenError,
    proto::DiscoverPackagesResponse,
    Paths,
};
use crate::{daemon::proto, globwatcher::HashGlobSetupError};

#[derive(Debug, Clone)]
pub struct DaemonClient<T> {
    client: proto::turbod_client::TurbodClient<tonic::transport::Channel>,
    connect_settings: T,
}

impl DaemonClient<()> {
    pub fn new(client: proto::turbod_client::TurbodClient<tonic::transport::Channel>) -> Self {
        Self {
            client,
            connect_settings: (),
        }
    }

    /// Augment the client with the connect settings, allowing it to be
    /// restarted.
    pub fn with_connect_settings(
        self,
        connect_settings: DaemonConnector,
    ) -> DaemonClient<DaemonConnector> {
        DaemonClient {
            client: self.client,
            connect_settings,
        }
    }
}

impl<T> DaemonClient<T> {
    /// Interrogate the server for its version.
    #[tracing::instrument(skip(self))]
    pub(super) async fn handshake(&mut self) -> Result<(), DaemonError> {
        let _ret = self
            .client
            .hello(proto::HelloRequest {
                version: proto::VERSION.to_string(),
                // minor version means that we need the daemon server to have at least the
                // same features as us, but it can have more. it is unlikely that we will
                // ever want to change the version range but we can tune it if, for example,
                // we need to lock to a specific minor version.
                supported_version_range: proto::VersionRange::Minor.into(),
                // todo(arlyon): add session id
                ..Default::default()
            })
            .await?;

        Ok(())
    }

    /// Stops the daemon and closes the connection, returning
    /// the connection settings that were used to connect.
    pub async fn stop(mut self) -> Result<T, DaemonError> {
        info!("Stopping daemon");
        self.client.shutdown(proto::ShutdownRequest {}).await?;
        Ok(self.connect_settings)
    }

    pub async fn get_changed_outputs(
        &mut self,
        hash: String,
        output_globs: &[ValidatedGlob],
    ) -> Result<Vec<String>, DaemonError> {
        let output_globs = output_globs
            .iter()
            .map(|validated_glob| validated_glob.as_str().to_string())
            .collect();
        Ok(self
            .client
            .get_changed_outputs(proto::GetChangedOutputsRequest { hash, output_globs })
            .await?
            .into_inner()
            .changed_output_globs)
    }

    pub async fn notify_outputs_written(
        &mut self,
        hash: String,
        output_globs: &[ValidatedGlob],
        output_exclusion_globs: &[ValidatedGlob],
        time_saved: u64,
    ) -> Result<(), DaemonError> {
        let output_globs = output_globs
            .iter()
            .map(|validated_glob| validated_glob.as_str().to_string())
            .collect();
        let output_exclusion_globs = output_exclusion_globs
            .iter()
            .map(|validated_glob| validated_glob.as_str().to_string())
            .collect();
        self.client
            .notify_outputs_written(proto::NotifyOutputsWrittenRequest {
                hash,
                output_globs,
                output_exclusion_globs,
                time_saved,
            })
            .await?;

        Ok(())
    }

    /// Get the status of the daemon.
    pub async fn status(&mut self) -> Result<proto::DaemonStatus, DaemonError> {
        self.client
            .status(proto::StatusRequest {})
            .await?
            .into_inner()
            .daemon_status
            .ok_or(DaemonError::MalformedResponse)
    }

    pub async fn discover_packages(&mut self) -> Result<DiscoverPackagesResponse, DaemonError> {
        let response = self
            .client
            .discover_packages(proto::DiscoverPackagesRequest {})
            .await?
            .into_inner();

        Ok(response)
    }

    pub async fn discover_packages_blocking(
        &mut self,
    ) -> Result<DiscoverPackagesResponse, DaemonError> {
        let response = self
            .client
            .discover_packages_blocking(proto::DiscoverPackagesRequest {})
            .await?
            .into_inner();

        Ok(response)
    }
}

impl DaemonClient<DaemonConnector> {
    /// Stops the daemon, closes the connection, and opens a new connection.
    pub async fn restart(self) -> Result<DaemonClient<DaemonConnector>, DaemonError> {
        self.stop().await?.connect().await.map_err(Into::into)
    }

    pub fn paths(&self) -> &Paths {
        &self.connect_settings.paths
    }
}

fn format_repo_relative_glob(glob: &str) -> String {
    #[cfg(windows)]
    let glob = {
        let glob = if let Some(idx) = glob.find(':') {
            &glob[..idx]
        } else {
            glob
        };
        glob.replace("\\", "/")
    };
    glob.replace(':', "\\:")
}

#[derive(Error, Debug)]
pub enum DaemonError {
    /// The server was connected but is now unavailable.
    #[error("server is unavailable: {0}")]
    Unavailable(String),
    #[error("error opening socket: {0}")]
    SocketOpen(#[from] SocketOpenError),
    /// The server is running a different version of turborepo.
    #[error("version mismatch: {0:?}")]
    VersionMismatch(Option<String>),
    /// There is an issue with the underlying grpc transport.
    #[error("bad grpc transport: {0}")]
    GrpcTransport(#[from] tonic::transport::Error),
    /// The daemon returned an unexpected status code.
    #[error("bad grpc status code: {0}")]
    GrpcFailure(tonic::Code),
    /// The daemon returned a malformed response.
    #[error("malformed response")]
    MalformedResponse,
    /// There was an issue connecting to the daemon.
    #[error("unable to connect: {0}")]
    DaemonConnect(#[from] DaemonConnectorError),
    /// The timeout specified was invalid.
    #[error("invalid timeout specified ({0})")]
    #[allow(dead_code)]
    InvalidTimeout(String),
    /// The server is unable to start file watching.
    #[error("unable to start file watching")]
    SetupFileWatching(#[from] HashGlobSetupError),

    #[error("unable to display output: {0}")]
    DisplayError(#[from] serde_json::Error),

    #[error("unable to construct log file name: {0}")]
    InvalidLogFile(#[from] time::Error),

    #[error("unable to complete daemon clean")]
    CleanFailed,

    #[error("failed to setup cookie dir {1}: {0}")]
    CookieDir(io::Error, AbsoluteSystemPathBuf),

    #[error("failed to determine package manager: {0}")]
    PackageManager(#[from] turborepo_repository::package_manager::Error),

    #[error("`tail` is not installed. Please install it to use this feature.")]
    TailNotInstalled,

    #[error("could not find log file")]
    LogFileNotFound,
}

impl From<Status> for DaemonError {
    fn from(status: Status) -> DaemonError {
        match status.code() {
            Code::FailedPrecondition => {
                DaemonError::VersionMismatch(Some(status.message().to_owned()))
            }
            Code::Unimplemented => DaemonError::VersionMismatch(None),
            Code::Unavailable => DaemonError::Unavailable(status.message().to_string()),
            c => DaemonError::GrpcFailure(c),
        }
    }
}

#[cfg(test)]
mod test {
    use std::path::MAIN_SEPARATOR_STR;

    use crate::daemon::client::format_repo_relative_glob;

    #[test]
    fn test_format_repo_relative_glob() {
        let raw_glob = ["some", ".turbo", "turbo-foo:bar.log"].join(MAIN_SEPARATOR_STR);
        #[cfg(windows)]
        let expected = "some/.turbo/turbo-foo";
        #[cfg(not(windows))]
        let expected = "some/.turbo/turbo-foo\\:bar.log";

        let result = format_repo_relative_glob(&raw_glob);
        assert_eq!(result, expected);
    }
}
