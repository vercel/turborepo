use futures::StreamExt;
use turbopath::AbsoluteSystemPath;

use crate::{
    daemon::{proto, DaemonError},
    DaemonConnector, DaemonPaths,
};

pub struct WatchClient {}

impl WatchClient {
    pub async fn start(repo_root: &AbsoluteSystemPath) -> Result<(), DaemonError> {
        let connector = DaemonConnector {
            can_start_server: true,
            can_kill_server: true,
            paths: DaemonPaths::from_repo_root(repo_root),
        };

        let mut client = connector.connect().await?;

        let mut hashes = client.package_changes().await?;
        while let Some(hash) = hashes.next().await {
            // Should we recover here?
            let hash = hash.unwrap();
            match proto::PackageChangeType::try_from(hash.change_type).unwrap() {
                proto::PackageChangeType::Package => {
                    if let Some(package) = hash.package_name {
                        println!("{} changed", package);
                    }
                }
                proto::PackageChangeType::Rediscover => {
                    println!("Rediscovering packages");
                }
            }
        }

        Ok(())
    }
}
