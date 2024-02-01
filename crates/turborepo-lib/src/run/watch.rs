use crate::{commands::CommandBase, run::Error, DaemonConnector};

struct WatchClient {}

impl WatchClient {
    async fn start(base: &CommandBase) -> Result<(), Error> {
        let pid_file = base.daemon_file_root().join_component("turbod.pid");
        let sock_file = base.daemon_file_root().join_component("turbod.sock");

        let connector = DaemonConnector {
            can_start_server,
            can_kill_server,
            pid_file: pid_file.clone(),
            sock_file: sock_file.clone(),
        };

        let client = connector.connect().await?;
    }
}
