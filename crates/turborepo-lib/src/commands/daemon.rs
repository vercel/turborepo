use std::path::PathBuf;

use super::CommandBase;
use crate::{
    cli::DaemonCommand,
    daemon::{DaemonConnector, DaemonError},
};

/// Runs the daemon command.
pub async fn main(command: &DaemonCommand, base: &CommandBase) -> Result<(), DaemonError> {
    let (dont_start, dont_kill) = match command {
        DaemonCommand::Status { .. } => (true, true),
        DaemonCommand::Restart | DaemonCommand::Stop => (true, false),
        DaemonCommand::Start => (false, false),
    };

    let connector = DaemonConnector {
        dont_start,
        dont_kill,
        pid_file: base.daemon_file_root().join("turbod.pid"),
        sock_file: base.daemon_file_root().join("turbod.sock"),
    };

    let mut client = connector.connect().await?;

    match command {
        DaemonCommand::Restart => {
            client.restart().await?;
        }
        DaemonCommand::Start => {} // no-op
        DaemonCommand::Stop => {
            client.stop().await?;
        }
        DaemonCommand::Status { json } => {
            let status = client.status().await?;
            let status = DaemonStatus {
                uptime_ms: status.uptime_msec,
                log_file: status.log_file.into(),
                pid_file: client.connect_settings.pid_file.clone(),
                sock_file: client.connect_settings.sock_file.clone(),
            };
            if *json {
                println!("{}", serde_json::to_string_pretty(&status).unwrap());
            } else {
                println!("Daemon log file: {}", status.log_file.to_string_lossy());
                println!("Daemon uptime: {}s", status.uptime_ms / 1000);
                println!("Daemon pid file: {}", status.pid_file.to_string_lossy());
                println!("Daemon socket file: {}", status.sock_file.to_string_lossy());
            }
        }
    };

    Ok(())
}

#[derive(serde::Serialize)]
pub struct DaemonStatus {
    pub uptime_ms: u64,
    pub log_file: PathBuf,
    pub pid_file: PathBuf,
    pub sock_file: PathBuf,
}
