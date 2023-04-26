use std::{path::PathBuf, time::Duration};

use turbopath::{AbsoluteSystemPathBuf, RelativeSystemPathBuf};

use super::CommandBase;
use crate::{
    cli::DaemonCommand,
    daemon::{DaemonConnector, DaemonError},
};

/// Runs the daemon command.
pub async fn daemon_client(command: &DaemonCommand, base: &CommandBase) -> Result<(), DaemonError> {
    let (can_start_server, can_kill_server) = match command {
        DaemonCommand::Status { .. } => (false, false),
        DaemonCommand::Restart | DaemonCommand::Stop => (false, true),
        DaemonCommand::Start => (true, true),
    };

    let connector = DaemonConnector {
        can_start_server,
        can_kill_server,
        pid_file: base.daemon_file_root().join_relative(
            turbopath::RelativeSystemPathBuf::new("turbod.pid").expect("relative system"),
        ),
        sock_file: base.daemon_file_root().join_relative(
            turbopath::RelativeSystemPathBuf::new("turbod.sock").expect("relative system"),
        ),
    };

    let mut client = connector.connect().await?;

    match command {
        DaemonCommand::Restart => {
            client.restart().await?;
        }
        // connector.connect will have already started the daemon if needed,
        // so this is a no-op
        DaemonCommand::Start => {}
        DaemonCommand::Stop => {
            client.stop().await?;
        }
        DaemonCommand::Status { json } => {
            let status = client.status().await?;
            let status = DaemonStatus {
                uptime_ms: status.uptime_msec,
                log_file: status.log_file.into(),
                pid_file: client.pid_file().to_owned(),
                sock_file: client.sock_file().to_owned(),
            };
            if *json {
                println!("{}", serde_json::to_string_pretty(&status)?);
            } else {
                println!("Daemon log file: {}", status.log_file.to_string_lossy());
                println!(
                    "Daemon uptime: {}s",
                    humantime::format_duration(Duration::from_millis(status.uptime_ms))
                );
                println!("Daemon pid file: {}", status.pid_file.to_string_lossy());
                println!("Daemon socket file: {}", status.sock_file.to_string_lossy());
            }
        }
    };

    Ok(())
}

pub async fn daemon_server(base: &CommandBase, idle_time: &String) -> Result<(), DaemonError> {
    let log_file = {
        let directories = directories::ProjectDirs::from("com", "turborepo", "turborepo")
            .expect("user has a home dir");

        let folder = AbsoluteSystemPathBuf::new(directories.data_dir()).expect("absolute");

        let hash = format!("{}-turbo.log", base.repo_hash());

        let logs = RelativeSystemPathBuf::new("logs").expect("forward relative");
        let file = RelativeSystemPathBuf::new(hash).expect("forward relative");

        folder.join_relative(logs).join_relative(file)
    };

    let repo_root = AbsoluteSystemPathBuf::new(base.repo_root.clone()).expect("absolute");

    let timeout = go_parse_duration::parse_duration(idle_time)
        .map_err(|_| DaemonError::InvalidTimeout(idle_time.to_owned()))
        .map(|d| Duration::from_nanos(d as u64))?;

    let server = crate::daemon::DaemonServer::new(base, timeout, log_file)?;
    server.serve(repo_root).await;

    Ok(())
}

#[derive(serde::Serialize)]
pub struct DaemonStatus {
    pub uptime_ms: u64,
    // this comes from the daemon server, so we trust that
    // it is correct
    pub log_file: PathBuf,
    pub pid_file: turbopath::AbsoluteSystemPathBuf,
    pub sock_file: turbopath::AbsoluteSystemPathBuf,
}
