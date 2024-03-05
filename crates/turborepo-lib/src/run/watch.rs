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
            let event = hash.event.expect("event is missing");
            match event {
                proto::package_change_event::Event::PackageChanged(proto::PackageChanged {
                    package_name,
                }) => {
                    println!("{} changed", package_name);
                }
                proto::package_change_event::Event::RediscoverPackages(_) => {
                    println!("Rediscovering packages");
                }
            }
        }

        Ok(())
    }
}
